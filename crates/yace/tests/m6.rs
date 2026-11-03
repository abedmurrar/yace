//! M6: themes and customisation (spec §16, §17).

mod common;

use common::Harness;
use ratatui::style::{Color, Modifier};
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

fn config_text(h: &Harness) -> String {
    std::fs::read_to_string(h.paths().user_settings()).unwrap_or_default()
}

#[test]
fn theme_picker_previews_reverts_and_persists() {
    let original = "# keep me\n[editor]\ntabSize = 4 # four\n";
    let mut h = Harness::with_config(original);
    assert_eq!(&*h.app.theme.name, "YACE Dark");
    let dark_bg = h.app.theme.bg;
    h.key("ctrl+k");
    h.key("ctrl+t");
    assert!(h.screen().contains("Select Color Theme"));
    // The current theme is preselected; moving previews the next one.
    assert_eq!(&*h.app.theme.name, "YACE Dark");
    h.key("down");
    assert_eq!(&*h.app.theme.name, "YACE Light");
    assert_ne!(h.render()[(0, 0)].bg, dark_bg, "the screen is redrawn in the previewed theme");
    // Esc restores the theme and writes nothing.
    h.key("escape");
    assert_eq!(&*h.app.theme.name, "YACE Dark");
    assert_eq!(config_text(&h), original);
    // Search, preview, Enter: saved with the user's comments kept.
    h.key("ctrl+k");
    h.key("ctrl+t");
    h.type_text("dracula");
    assert_eq!(&*h.app.theme.name, "Dracula");
    h.key("enter");
    assert_eq!(&*h.app.theme.name, "Dracula");
    assert_eq!(h.app.settings.workbench.color_theme, "Dracula");
    let text = config_text(&h);
    assert!(text.starts_with("# keep me\n[editor]\ntabSize = 4 # four\n"), "{text}");
    assert!(text.contains("[workbench]\ncolorTheme = \"Dracula\""), "{text}");
    // A new window starts with the saved theme.
    assert_eq!(&*h.new_app().theme.name, "Dracula");
}

#[test]
fn closing_the_picker_any_other_way_reverts_the_preview() {
    let mut h = Harness::new();
    h.command("workbench.action.selectTheme");
    h.key("down");
    assert_ne!(&*h.app.theme.name, "YACE Dark");
    h.app.overlays.clear();
    h.key("escape");
    assert_eq!(&*h.app.theme.name, "YACE Dark");
}

#[test]
fn settings_overrides_and_invalid_themes() {
    let mut h = Harness::with_config(
        r##"[workbench]
colorTheme = "One Dark"
colorCustomizations = { "statusBar.background" = "#010203", "[One Dark]" = { "tab.activeBackground" = "#040506" } }
[editor]
tokenColorCustomizations = { comments = { foreground = "#070809", fontStyle = "bold" } }
"##,
    );
    assert!(h.app.config_errors.is_empty(), "{:?}", h.app.config_errors);
    let t = &h.app.theme;
    assert_eq!(&*t.name, "One Dark");
    assert_eq!(t.status_bg, Color::Rgb(1, 2, 3));
    assert_eq!(t.tab_active_bg, Color::Rgb(4, 5, 6));
    let c = yace_syntax::Scope::Comment as usize;
    assert_eq!(t.syntax[c], Color::Rgb(7, 8, 9));
    assert!(t.syntax_mods[c].contains(Modifier::BOLD));
    // No folder is open, so the status bar uses its "no folder" colour.
    let status = h.render()[(0, h.app.area.height - 1)].bg;
    assert_eq!(status, h.app.theme.status_no_folder_bg);

    // A broken user theme: the error is reported and the theme stays.
    let themes = h.paths().themes_dir();
    std::fs::create_dir_all(&themes).unwrap();
    std::fs::write(themes.join("bad.toml"), "name = \"Bad\"\nextends = \"nope\"\n").unwrap();
    std::fs::write(h.paths().user_settings(), "[workbench]\ncolorTheme = \"Bad\"\n").unwrap();
    h.command("yace.reloadConfiguration");
    assert_eq!(&*h.app.theme.name, "One Dark", "the previous theme stays");
    assert!(
        h.app.config_errors.iter().any(|e| e.to_string().contains("unknown theme `nope`")),
        "{:?}",
        h.app.config_errors
    );

    // A fresh start with a broken theme falls back to YACE Dark.
    assert_eq!(&*h.new_app().theme.name, "YACE Dark");
}

#[test]
fn user_theme_extends_a_builtin() {
    let mut h = Harness::with_config("[workbench]\ncolorTheme = \"My Night\"\n");
    let themes = h.paths().themes_dir();
    std::fs::create_dir_all(&themes).unwrap();
    std::fs::write(
        themes.join("my-night.toml"),
        "name = \"My Night\"\nextends = \"tokyo-night\"\n[colors]\n\"editor.background\" = \"#000000\"\n",
    )
    .unwrap();
    h.command("yace.reloadConfiguration");
    assert_eq!(&*h.app.theme.name, "My Night");
    assert_eq!(h.app.theme.bg, Color::Rgb(0, 0, 0));
    // The picker lists it after the built-ins, with its file.
    h.command("workbench.action.selectTheme");
    let screen = h.screen();
    assert!(screen.contains("My Night") && screen.contains("my-night.toml"), "{screen}");
    h.key("escape");
    // "Create Theme from Current" writes a file extending the current theme.
    h.command("yace.createThemeFromCurrent");
    let created = std::fs::read_to_string(themes.join("my-theme.toml")).unwrap();
    assert!(created.contains("extends = \"my-night\""), "{created}");
    assert!(h.app.active().is_some(), "the new theme file is opened");
}

#[test]
fn monochrome_shows_state_with_text_attributes() {
    let mut h = Harness::with_config("[workbench]\ncolorDepth = \"mono\"\n");
    assert!(h.app.theme.is_mono());
    h.command("workbench.action.files.newUntitledFile");
    h.type_text("hello world");
    h.key("shift+home");
    let buf = h.render();
    let area = buf.area;
    let mut found = false;
    for y in 0..area.height {
        let row: String = (0..area.width).map(|x| buf[(x, y)].symbol().to_string()).collect();
        if let Some(b) = row.find("hello") {
            let x = row[..b].chars().count() as u16;
            assert!(buf[(x, y)].modifier.contains(Modifier::REVERSED), "selection is reverse video");
            assert_eq!(buf[(x, y)].bg, Color::Reset);
            found = true;
            break;
        }
    }
    assert!(found);
}

#[test]
fn imports_a_vscode_theme_file() {
    let d = tempfile::tempdir().unwrap();
    let json = d.path().join("cool-theme.json");
    std::fs::write(
        &json,
        r##"{ "name": "Cool", "type": "light",
          "colors": { "editor.background": "#fafafa", "statusBar.background": "#abcdef" },
          "tokenColors": [ { "scope": "keyword", "settings": { "foreground": "#aa0000" } } ] }"##,
    )
    .unwrap();
    let mut h = Harness::new();
    h.command("yace.importVSCodeThemeFile");
    h.type_text(&json.display().to_string());
    h.key("enter");
    assert_eq!(&*h.app.theme.name, "cool-theme");
    assert_eq!(h.app.theme.status_bg, Color::Rgb(0xab, 0xcd, 0xef));
    assert_eq!(h.app.theme.kind, yace_app::theme::ThemeKind::Light);
    assert_eq!(h.app.theme.syntax[yace_syntax::Scope::Keyword as usize], Color::Rgb(0xaa, 0, 0));
    assert!(h.paths().themes_dir().join("cool-theme.toml").is_file());
    assert_eq!(h.app.settings.workbench.color_theme, "cool-theme");
}

#[test]
fn inspect_tokens_names_the_scope() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("main.rs"), "fn main() {}\n").unwrap();
    let mut h = Harness::new();
    open_folder(&mut h, d.path());
    h.app.open_file(&d.path().join("main.rs"), false);
    h.settle();
    h.command("editor.action.inspectTMScopes");
    let screen = h.screen();
    assert!(screen.contains("Scope: keyword"), "{screen}");
    assert!(screen.contains("Rust"), "{screen}");
}

#[test]
fn config_files_reload_live_and_broken_files_keep_the_last_good_values() {
    let mut h = Harness::with_config("[editor]\ntabSize = 3\n");
    let cfg = h.paths().user_settings();
    assert_eq!(h.app.settings.editor.tab_size, 3);
    h.app.on_tick(std::time::Instant::now());
    assert!(h.app.watched_dirs().contains(&h.paths().config_dir), "the config folder is watched");

    // An external edit applies through the watcher.
    std::fs::write(&cfg, "[editor]\ntabSize = 5\n[workbench]\ncolorTheme = \"Tokyo Night\"\n").unwrap();
    h.fs_changed(vec![cfg.clone()]);
    assert_eq!(h.app.settings.editor.tab_size, 5);
    assert_eq!(&*h.app.theme.name, "Tokyo Night");
    assert!(h.screen().contains("Settings reloaded"));

    // A broken file: error with the line, previous values stay.
    std::fs::write(&cfg, "[editor]\ntabSize = 7\n[workbench\n").unwrap();
    h.fs_changed(vec![cfg.clone()]);
    assert_eq!(h.app.settings.editor.tab_size, 5);
    assert_eq!(&*h.app.theme.name, "Tokyo Night");
    let err = h.app.config_errors.iter().map(|e| e.to_string()).collect::<Vec<_>>().join("\n");
    assert!(err.contains("line 3") && err.contains("previous settings stay"), "{err}");

    // An event for an unchanged file does nothing (e.g. our own write).
    std::fs::write(&cfg, "[editor]\ntabSize = 5\n[workbench]\ncolorTheme = \"Tokyo Night\"\n").unwrap();
    h.fs_changed(vec![cfg.clone()]);
    h.app.notifications.clear();
    h.fs_changed(vec![cfg.clone()]);
    assert!(h.app.notifications.is_empty());

    // Keybindings reload too, and a broken file keeps the last good ones.
    let kb = h.paths().keybindings();
    std::fs::write(&kb, "[[bindings]]\nkey = \"ctrl+alt+j\"\ncommand = \"workbench.action.selectTheme\"\n").unwrap();
    h.fs_changed(vec![kb.clone()]);
    h.key("ctrl+alt+j");
    assert!(h.screen().contains("Select Color Theme"));
    h.key("escape");
    std::fs::write(&kb, "[[bindings]\n").unwrap();
    h.fs_changed(vec![kb.clone()]);
    h.key("ctrl+alt+j");
    assert!(h.screen().contains("Select Color Theme"), "previous bindings stay");
    h.key("escape");
}

#[test]
fn editing_the_current_theme_file_applies_on_save() {
    let mut h = Harness::with_config("[workbench]\ncolorTheme = \"Mine\"\n");
    let themes = h.paths().themes_dir();
    std::fs::create_dir_all(&themes).unwrap();
    let file = themes.join("mine.toml");
    std::fs::write(&file, "name = \"Mine\"\n[colors]\n\"statusBar.background\" = \"#111111\"\n").unwrap();
    h.command("yace.reloadConfiguration");
    assert_eq!(h.app.theme.status_bg, Color::Rgb(0x11, 0x11, 0x11));
    h.app.on_tick(std::time::Instant::now());
    assert!(h.app.watched_dirs().contains(&themes));
    std::fs::write(&file, "name = \"Mine\"\n[colors]\n\"statusBar.background\" = \"#222222\"\n").unwrap();
    h.fs_changed(vec![file.clone()]);
    assert_eq!(h.app.theme.status_bg, Color::Rgb(0x22, 0x22, 0x22));
    // Saving it from inside YACE applies it as well.
    h.app.open_file(&file, false);
    h.settle();
    h.key("ctrl+end");
    h.type_text("\"editorGroup.border\" = \"#333333\"\n");
    h.key("ctrl+s");
    h.settle();
    assert_eq!(h.app.theme.border, Color::Rgb(0x33, 0x33, 0x33));
}

fn kb_text(h: &Harness) -> String {
    std::fs::read_to_string(h.paths().keybindings()).unwrap_or_default()
}

fn kb_current(h: &Harness) -> Option<yace_app::keybindings_editor::KbRow> {
    match h.app.overlays.last() {
        Some(yace_app::overlay::Overlay::Keybindings(e)) => e.current().cloned(),
        _ => None,
    }
}

#[test]
fn keybinding_editor_defines_resets_and_removes() {
    let mut h = Harness::new();
    std::fs::create_dir_all(h.paths().config_dir).unwrap();
    std::fs::write(h.paths().keybindings(), "# my own bindings\n").unwrap();
    h.key("ctrl+k");
    h.key("ctrl+s");
    let screen = h.screen();
    assert!(
        screen.contains("Keyboard Shortcuts") && screen.contains("Keybinding") && screen.contains("Source"),
        "{screen}"
    );

    // Search, Enter, press the new keys, Enter.
    h.type_text("color theme");
    let row = kb_current(&h).unwrap();
    assert_eq!(row.command, "workbench.action.selectTheme");
    assert_eq!(row.keys_text(), "Ctrl+K Ctrl+T");
    h.key("enter");
    h.key("ctrl+alt+t");
    assert!(h.screen().contains("Press the keys for"));
    h.key("enter");
    let text = kb_text(&h);
    assert!(text.starts_with("# my own bindings\n"), "comments kept: {text}");
    assert!(
        text.contains("command = \"-workbench.action.selectTheme\"") && text.contains("key = \"ctrl+k ctrl+t\""),
        "{text}"
    );
    assert!(text.contains("key = \"ctrl+alt+t\"\ncommand = \"workbench.action.selectTheme\""), "{text}");
    let row = kb_current(&h).unwrap();
    assert_eq!((row.keys_text().as_str(), row.source_text()), ("Ctrl+Alt+T", "User"));
    h.key("escape");
    assert!(h.app.overlays.is_empty());
    h.key("ctrl+alt+t");
    assert!(h.screen().contains("Select Color Theme"), "the new keys work");
    h.key("escape");
    h.key("ctrl+k");
    h.key("ctrl+t");
    assert!(!h.screen().contains("Select Color Theme"), "the old keys are gone");

    // Reset to default.
    h.command("workbench.action.openGlobalKeybindings");
    h.type_text("color theme");
    h.key("alt+r");
    assert!(!kb_text(&h).contains("selectTheme"), "{}", kb_text(&h));
    assert_eq!(kb_current(&h).unwrap().keys_text(), "Ctrl+K Ctrl+T");
    assert!(h.screen().contains("Reset Preferences: Color Theme"));

    // Search by recorded keys, then remove a default binding.
    h.key("ctrl+a");
    h.key("backspace");
    h.key("alt+k");
    h.key("ctrl+b");
    let row = kb_current(&h).unwrap();
    assert_eq!(row.keys_text(), "Ctrl+B");
    h.key("escape"); // stop recording
    h.key("delete");
    assert!(kb_text(&h).contains(&format!("command = \"-{}\"", row.command)), "{}", kb_text(&h));
    h.key("escape");
    assert!(!h.app.keymap.bindings().iter().any(|b| b.command == row.command && b.keys.to_string() == "Ctrl+B"));
}

#[test]
fn keybinding_editor_lists_unbound_commands_and_conflicts() {
    let mut h = Harness::new();
    std::fs::create_dir_all(h.paths().config_dir).unwrap();
    std::fs::write(h.paths().keybindings(), "[[bindings]]\nkey = \"ctrl+k ctrl+t\"\ncommand = \"yace.about\"\n")
        .unwrap();
    h.command("yace.reloadConfiguration");
    h.command("workbench.action.openGlobalKeybindings");
    h.type_text("@conflicts");
    let screen = h.screen();
    assert!(screen.contains("Help: About") && screen.contains("Preferences: Color Theme"), "{screen}");
    // Unbound commands are listed, and can get keys.
    h.key("ctrl+a");
    h.type_text("@unbound export keybindings");
    let row = kb_current(&h).unwrap();
    assert_eq!((row.command.as_str(), row.source), ("yace.exportKeybindings", None));
    h.key("enter");
    h.key("ctrl+alt+e");
    h.key("enter");
    assert!(kb_text(&h).contains("key = \"ctrl+alt+e\"\ncommand = \"yace.exportKeybindings\""));
    h.key("escape");
    h.key("ctrl+alt+e");
    let exported = h.active_text();
    assert!(exported.contains("[[bindings]] # default") && exported.contains("[[bindings]] # user"), "{exported}");
}

#[test]
fn restore_default_keybindings_keeps_a_backup() {
    let mut h = Harness::new();
    std::fs::create_dir_all(h.paths().config_dir).unwrap();
    std::fs::write(h.paths().keybindings(), "[[bindings]]\nkey = \"ctrl+alt+j\"\ncommand = \"yace.about\"\n").unwrap();
    h.command("yace.reloadConfiguration");
    h.command("yace.restoreDefaultKeybindings");
    h.key("r");
    assert!(!h.paths().keybindings().exists());
    assert!(h.paths().keybindings().with_extension("toml.bak").exists());
    assert!(!h.app.keymap.bindings().iter().any(|b| b.command == "yace.about"));
}

#[test]
fn every_small_terminal_size_renders_without_panicking() {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir(d.path().join("src")).unwrap();
    let long = "fn main() { let x = \"a very long line that needs wrapping or scrolling ✓ 你好\"; }\n".repeat(30);
    std::fs::write(d.path().join("src").join("main.rs"), &long).unwrap();
    let mut h = Harness::with_config("[workbench]\nsideBarLocation = \"right\"\n");
    open_folder(&mut h, d.path());
    h.app.open_file(&d.path().join("src").join("main.rs"), false);
    h.settle();
    h.app.panel.visible = true;
    type Setup = Box<dyn Fn(&mut Harness)>;
    let states: Vec<(&str, Setup)> = vec![
        ("editor", Box::new(|_| {})),
        ("split", Box::new(|h| h.command("workbench.action.splitEditor"))),
        ("find", Box::new(|h| h.key("ctrl+f"))),
        ("palette", Box::new(|h| h.key("ctrl+shift+p"))),
        ("theme picker", Box::new(|h| h.command("workbench.action.selectTheme"))),
        ("keybindings", Box::new(|h| h.command("workbench.action.openGlobalKeybindings"))),
        (
            "define keys",
            Box::new(|h| {
                h.command("workbench.action.openGlobalKeybindings");
                h.key("enter");
                h.key("ctrl+j");
            }),
        ),
        ("info", Box::new(|h| h.command("yace.about"))),
        ("prompt", Box::new(|h| h.command("workbench.action.gotoLine"))),
        ("search view", Box::new(|h| h.key("ctrl+shift+f"))),
        ("explorer focus", Box::new(|h| h.key("ctrl+shift+e"))),
    ];
    for (name, setup) in &states {
        h.app.overlays.clear();
        h.app.resize(120, 40);
        setup(&mut h);
        for w in 1..=50u16 {
            for ht in 1..=15u16 {
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    h.render_at(w, ht);
                }));
                assert!(r.is_ok(), "{name}: panicked at {w}×{ht}");
            }
        }
    }
}

#[test]
fn sidebar_on_the_right_and_reset_layout() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("a.txt"), "hello\n").unwrap();
    let mut h = Harness::new();
    open_folder(&mut h, d.path());
    h.command("workbench.action.toggleSidebarPosition");
    assert_eq!(h.app.settings.workbench.side_bar_location, yace_config::SideBarLocation::Right);
    assert!(config_text(&h).contains("sideBarLocation = \"right\""));
    let screen = h.screen();
    let first = screen.lines().next().unwrap();
    assert!(first.find("EXPLORER").unwrap() > 60, "{first}");
    // Clicking a file on the right opens it.
    let lines: Vec<&str> = screen.lines().collect();
    let (y, line) = lines.iter().enumerate().find(|(_, l)| l.contains("a.txt")).unwrap();
    let x = line[..line.find("a.txt").unwrap()].chars().count() as u16;
    h.click(x, y as u16, ratatui::crossterm::event::KeyModifiers::NONE);
    assert_eq!(h.active_text(), "hello\n");
    // Dragging the border resizes from the right.
    let wl = yace_app::ui::workbench_layout(&h.app);
    let border = wl.sidebar_border.unwrap();
    use ratatui::crossterm::event::{MouseButton, MouseEventKind};
    h.mouse(MouseEventKind::Down(MouseButton::Left), border.x, 5);
    h.mouse(MouseEventKind::Drag(MouseButton::Left), border.x - 10, 5);
    h.mouse(MouseEventKind::Up(MouseButton::Left), border.x - 10, 5);
    assert_eq!(h.app.sidebar_width, 120 - (border.x - 10) - 1);
    // Reset Layout restores sizes.
    h.command("workbench.action.splitEditor");
    h.app.panel.maximized = true;
    h.app.sidebar_visible = false;
    h.command("yace.resetWorkbenchLayout");
    assert!(h.app.sidebar_visible && !h.app.panel.maximized);
    assert_eq!(h.app.sidebar_width, h.app.settings.workbench.sidebar_width);
    h.command("workbench.action.toggleSidebarPosition");
    assert_eq!(h.app.settings.workbench.side_bar_location, yace_config::SideBarLocation::Left);
}
