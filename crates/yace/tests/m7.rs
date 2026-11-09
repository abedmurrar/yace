//! M7: syntax highlighting, language rules, folding/wrap and language
//! servers (spec §12, §13).

mod common;

use common::Harness;
use yace_app::{
    cli::{Launch, Target},
    initial_workspace,
};

fn open_folder(h: &mut Harness, dir: &std::path::Path) {
    initial_workspace(
        &mut h.app,
        &Launch { target: Target::Folder(dir.to_path_buf()), add_roots: vec![], goto: None, restore: false },
    );
    h.settle();
}

/// The colour of the first screen cell showing `needle`.
fn fg_of(h: &mut Harness, needle: &str) -> ratatui::style::Color {
    let buf = h.render();
    let area = buf.area;
    for y in 0..area.height {
        let row: String = (0..area.width).map(|x| buf[(x, y)].symbol().to_string()).collect();
        if let Some(byte) = row.find(needle) {
            let x = row[..byte].chars().count() as u16;
            return buf[(x, y)].fg;
        }
    }
    panic!("{needle} not on screen:\n{}", h.screen());
}

#[test]
fn rust_file_is_highlighted_and_stays_highlighted_while_typing() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("main.rs"), "// note\nfn main() {\n    let s = \"hi\";\n}\n").unwrap();
    let mut h = Harness::new();
    open_folder(&mut h, d.path());
    h.app.open_file(&d.path().join("main.rs"), false);
    h.settle();
    let d_id = h.app.active().unwrap().2;
    assert_eq!(h.app.syntax.language(d_id).id, "rust");
    assert!(h.app.syntax.has_tree(d_id));
    let syn = h.app.theme.syntax;
    use yace_syntax::Scope;
    assert_eq!(fg_of(&mut h, "fn main"), syn[Scope::Keyword as usize]);
    assert_eq!(fg_of(&mut h, "\"hi\""), syn[Scope::String as usize]);
    assert_eq!(fg_of(&mut h, "// note"), syn[Scope::Comment as usize]);
    assert!(h.screen().contains("Rust"), "status bar language");
    // Typing a new statement is highlighted immediately (incremental).
    h.key("ctrl+end");
    h.type_text("const X: u32 = 7;");
    assert_eq!(fg_of(&mut h, "const X"), syn[Scope::Keyword as usize]);
    assert_eq!(fg_of(&mut h, "u32"), syn[Scope::TypeBuiltin as usize]);
    // Undo keeps the tree in sync.
    h.key("ctrl+z");
    h.key("ctrl+z");
    let tree = h.app.syntax.tree(d_id).unwrap().root_node().to_sexp();
    let full = yace_syntax::parse_full(yace_syntax::by_id("rust").unwrap(), h.app.docs.doc(d_id).unwrap().rope(), None)
        .unwrap()
        .root_node()
        .to_sexp();
    assert_eq!(tree, full, "incremental tree equals a fresh parse after undo");
}

#[test]
fn large_file_parses_in_background_and_edits_are_replayed() {
    let d = tempfile::tempdir().unwrap();
    let body = "export function f(a: number): number {\n  return a * 2;\n}\n".repeat(3000);
    std::fs::write(d.path().join("big.ts"), &body).unwrap();
    let mut h = Harness::new();
    open_folder(&mut h, d.path());
    h.app.open_file(&d.path().join("big.ts"), false);
    h.settle();
    let d_id = h.app.active().unwrap().2;
    assert!(h.app.syntax.has_tree(d_id), "worker result installed");
    h.type_text("let y = 1;\n");
    let tree = h.app.syntax.tree(d_id).unwrap().root_node().to_sexp();
    let full =
        yace_syntax::parse_full(yace_syntax::by_id("typescript").unwrap(), h.app.docs.doc(d_id).unwrap().rope(), None)
            .unwrap()
            .root_node()
            .to_sexp();
    assert_eq!(tree, full);
}

#[test]
fn language_rules_comments_and_indent() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("a.py"), "def f():").unwrap();
    std::fs::write(d.path().join("s.css"), "a { color: red; }").unwrap();
    let mut h = Harness::new();
    open_folder(&mut h, d.path());
    h.app.open_file(&d.path().join("a.py"), false);
    h.settle();
    h.key("end");
    h.key("enter");
    h.type_text("pass");
    assert_eq!(h.active_text(), "def f():\n    pass");
    h.key("ctrl+/");
    assert_eq!(h.active_text(), "def f():\n    # pass");
    h.app.open_file(&d.path().join("s.css"), false);
    h.settle();
    h.key("ctrl+/");
    assert_eq!(h.active_text(), "/* a { color: red; } */", "CSS has only block comments");
    h.key("shift+alt+a");
    assert_eq!(h.active_text(), "a { color: red; }");
}

#[test]
fn files_associations_override_detection() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("rules.mdc"), "# Title\n").unwrap();
    let mut h = Harness::with_config("[files.associations]\n\"*.mdc\" = \"markdown\"\n");
    open_folder(&mut h, d.path());
    h.app.open_file(&d.path().join("rules.mdc"), false);
    h.settle();
    let d_id = h.app.active().unwrap().2;
    assert_eq!(h.app.syntax.language(d_id).id, "markdown");
}

const RS: &str = "fn main() {\n    let a = 1;\n    let b = 2;\n}\n\nfn other() {\n    call();\n}\n";

#[test]
fn folding_hides_lines_and_movement_skips_them() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("f.rs"), RS).unwrap();
    let mut h = Harness::new();
    open_folder(&mut h, d.path());
    h.app.open_file(&d.path().join("f.rs"), false);
    h.settle();
    h.command("editor.fold");
    assert_eq!(h.app.view_folds(), vec![(0, 2)]);
    let screen = h.screen();
    assert!(!screen.contains("let a"), "folded lines hidden:\n{screen}");
    assert!(screen.contains("fn main() {"), "header shown");
    assert!(screen.contains("▸"), "folded marker");
    // Down from the header lands on the closing line, skipping the fold.
    h.key("down");
    assert_eq!(h.cursor(), (3, 0));
    h.key("up");
    assert_eq!(h.cursor().0, 0);
    // Right from the end of the header steps over the fold.
    h.key("end");
    h.key("right");
    assert_eq!(h.cursor(), (3, 0));
    // Editing elsewhere keeps the fold; editing inside reveals it.
    h.key("ctrl+end");
    h.type_text("// tail");
    assert_eq!(h.app.view_folds(), vec![(0, 2)]);
    h.command("workbench.action.gotoLine");
    h.type_text("2");
    h.key("enter");
    assert!(h.app.view_folds().is_empty(), "going into a fold opens it");
    // Fold All / Unfold All.
    h.command("editor.foldAll");
    assert_eq!(h.app.view_folds(), vec![(0, 2), (5, 6)]);
    h.command("editor.unfoldAll");
    assert!(h.app.view_folds().is_empty());
}

#[test]
fn gutter_click_toggles_folds_and_folds_persist_in_the_session() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("f.rs"), RS).unwrap();
    let mut h = Harness::new();
    open_folder(&mut h, d.path());
    h.app.open_file(&d.path().join("f.rs"), false);
    h.settle();
    let wl = yace_app::ui::workbench_layout(&h.app);
    let g = wl.groups[0].clone();
    let marker_x = g.content.x + g.gutter - 2;
    let none = ratatui::crossterm::event::KeyModifiers::NONE;
    h.click(marker_x, g.content.y + 5, none);
    assert_eq!(h.app.view_folds(), vec![(5, 6)]);
    // Persisted: save, close everything, reopen with restore.
    h.app.save_session();
    h.command("workbench.action.closeAllEditors");
    initial_workspace(
        &mut h.app,
        &Launch { target: Target::Folder(d.path().to_path_buf()), add_roots: vec![], goto: None, restore: true },
    );
    h.settle();
    assert_eq!(h.app.view_folds(), vec![(5, 6)], "folds restored with the session");
    h.click(marker_x, g.content.y + 5, none);
    assert!(h.app.view_folds().is_empty());
}

#[test]
fn word_wrap_wraps_long_lines_and_moves_by_rows() {
    let d = tempfile::tempdir().unwrap();
    let long = "word ".repeat(60);
    std::fs::write(d.path().join("w.txt"), format!("{long}\nshort\n")).unwrap();
    let mut h = Harness::with_config("[editor]\nwordWrap = \"wordWrapColumn\"\nwordWrapColumn = 40\n");
    open_folder(&mut h, d.path());
    h.app.open_file(&d.path().join("w.txt"), false);
    h.settle();
    let screen = h.screen();
    let wrapped_rows = screen.lines().filter(|l| l.contains("word word")).count();
    assert!(wrapped_rows >= 7, "300 chars at 40 columns wrap onto several rows:\n{screen}");
    // Down moves within the wrapped line first.
    h.key("down");
    let (line, col) = h.cursor();
    assert_eq!(line, 0);
    assert!((35..=40).contains(&col), "second row of line 0, got col {col}");
    // Alt+Z turns wrapping off for the window.
    h.key("alt+z");
    let screen = h.screen();
    assert_eq!(screen.lines().filter(|l| l.contains("word word")).count(), 1, "{screen}");
}

const NESTED: &str = "/* block\n   comment */\nfn main() {\n    if a {\n        if b {\n            x();\n        }\n    }\n}\nfn other() {\n    y();\n}\n";

/// Fold Level N, Fold Recursively / Unfold Recursively and Fold All Block
/// Comments (VS Code's commands and Ctrl+K chords).
#[test]
fn fold_levels_recursive_and_block_comments() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("n.rs"), NESTED).unwrap();
    let mut h = Harness::new();
    open_folder(&mut h, d.path());
    h.app.open_file(&d.path().join("n.rs"), false);
    h.settle();
    // Level 2 is the outer `if`; the cursor (line 0) is not inside it.
    h.key("ctrl+k ctrl+2");
    assert_eq!(h.app.view_folds(), vec![(3, 6)]);
    h.command("editor.foldLevel1");
    assert_eq!(h.app.view_folds(), vec![(2, 7), (3, 6), (9, 10)], "level 1 skips the comment holding the cursor");
    h.command("editor.unfoldAll");
    h.key("ctrl+k ctrl+/");
    assert_eq!(h.app.view_folds(), vec![(0, 1)], "only the block comment");
    h.command("editor.unfoldAll");
    // Recursively: the function and everything in it.
    h.app.execute("workbench.action.gotoLine", Some(&serde_json::json!({ "line": 3 })));
    h.settle();
    h.key("ctrl+k ctrl+[");
    assert_eq!(h.app.view_folds(), vec![(2, 7), (3, 6), (4, 5)]);
    assert_eq!(h.cursor().0, 2, "cursor stays on the visible header");
    h.key("ctrl+k ctrl+]");
    assert!(h.app.view_folds().is_empty(), "unfolded with everything inside");
    // A cursor inside a level-3 region keeps it open.
    h.app.execute("workbench.action.gotoLine", Some(&serde_json::json!({ "line": 6 })));
    h.settle();
    h.command("editor.foldLevel3");
    assert!(h.app.view_folds().is_empty());
}
