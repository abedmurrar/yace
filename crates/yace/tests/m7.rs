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
