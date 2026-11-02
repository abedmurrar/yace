//! Mouse, auto-save, close-flow and tab-history tests.

mod common;

use std::{
    fs,
    time::{Duration, Instant},
};

use common::Harness;
use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEventKind};
use yace_app::{
    app::Focus,
    cli::{Launch, Target},
    initial_workspace,
    overlay::Overlay,
    ui::workbench_layout,
};

fn open_folder(h: &mut Harness, dir: &std::path::Path) {
    let launch = Launch { target: Target::Folder(dir.to_path_buf()), add_roots: vec![], goto: None, restore: false };
    initial_workspace(&mut h.app, &launch);
    h.settle();
}

fn folder() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("a.txt"), "alpha line\nsecond\n").unwrap();
    fs::write(d.path().join("b.txt"), "beta\n").unwrap();
    d
}

#[test]
fn mouse_explorer_editor_tabs_and_sidebar() {
    let d = folder();
    let mut h = Harness::new();
    open_folder(&mut h, d.path());
    let wl = workbench_layout(&h.app);
    let list = wl.explorer_list.unwrap();
    let row_a = h.app.explorer.rows.iter().position(|r| r.label == "a.txt").unwrap() as u16;
    // Single click previews and keeps focus in the explorer.
    h.click(list.x + 3, list.y + row_a, KeyModifiers::NONE);
    assert_eq!(h.app.focus, Focus::Explorer);
    assert!(h.app.layout.active_group().active_view().unwrap().preview);
    // Double click pins and focuses the editor.
    h.click(list.x + 3, list.y + row_a, KeyModifiers::NONE);
    assert_eq!(h.app.focus, Focus::Editor);
    assert!(!h.app.layout.active_group().active_view().unwrap().preview);

    let wl = workbench_layout(&h.app);
    let g = &wl.groups[0];
    // Click on line 2, column 3 places the cursor.
    h.click(g.text_x + 3, g.content.y + 1, KeyModifiers::NONE);
    let (_, v, _) = h.app.active().unwrap();
    assert_eq!(h.app.layout.active_group().views[v].sel.primary().head, 11 + 3);
    // Alt+click adds a cursor.
    h.click(g.text_x + 1, g.content.y, KeyModifiers::ALT);
    assert_eq!(h.app.layout.active_group().views[v].sel.len(), 2);
    // Drag selects.
    h.mouse(MouseEventKind::Down(MouseButton::Left), g.text_x, g.content.y);
    h.mouse(MouseEventKind::Drag(MouseButton::Left), g.text_x + 5, g.content.y);
    h.mouse(MouseEventKind::Up(MouseButton::Left), g.text_x + 5, g.content.y);
    let sel = h.app.layout.active_group().views[v].sel.primary();
    assert_eq!((sel.from(), sel.to()), (0, 5));

    // Open a second file, then click the first tab and middle-click to close it.
    h.app.open_file(&d.path().join("b.txt"), false);
    let wl = workbench_layout(&h.app);
    let tabs = wl.groups[0].tabs;
    h.click(tabs.x + 2, tabs.y, KeyModifiers::NONE);
    assert_eq!(h.app.docs.name(h.app.active().unwrap().2), "a.txt");
    h.mouse(MouseEventKind::Down(MouseButton::Middle), tabs.x + 2, tabs.y);
    assert_eq!(h.app.layout.active_group().views.len(), 1);

    // Drag the sidebar border.
    let border = workbench_layout(&h.app).sidebar_border.unwrap();
    h.mouse(MouseEventKind::Down(MouseButton::Left), border.x, 5);
    h.mouse(MouseEventKind::Drag(MouseButton::Left), 50, 5);
    h.mouse(MouseEventKind::Up(MouseButton::Left), 50, 5);
    assert_eq!(h.app.sidebar_width, 50);

    // Wheel scrolls the editor.
    let g = workbench_layout(&h.app).groups[0].clone();
    for _ in 0..50 {
        h.app.type_text_for_tests("\n");
    }
    h.mouse(MouseEventKind::ScrollDown, g.text_x + 1, g.content.y + 1);
    assert!(h.app.layout.active_group().active_view().unwrap().scroll_line > 0);
}

#[test]
fn auto_save_after_delay() {
    let d = folder();
    let mut h = Harness::with_config("[files]\nautoSave = \"afterDelay\"\nautoSaveDelay = 200\n");
    open_folder(&mut h, d.path());
    h.app.open_file(&d.path().join("b.txt"), false);
    h.type_text("Z");
    assert_eq!(fs::read_to_string(d.path().join("b.txt")).unwrap(), "beta\n");
    assert!(h.app.next_deadline().is_some());
    h.app.on_tick(Instant::now() + Duration::from_secs(1));
    assert_eq!(fs::read_to_string(d.path().join("b.txt")).unwrap(), "Zbeta\n");
}

#[test]
fn quit_with_save_all_saves_every_dirty_file() {
    let d = folder();
    let mut h = Harness::new();
    open_folder(&mut h, d.path());
    h.app.open_file(&d.path().join("a.txt"), false);
    h.type_text("1");
    h.app.open_file(&d.path().join("b.txt"), false);
    h.type_text("2");
    h.key("ctrl+q");
    assert!(matches!(h.app.overlays.last(), Some(Overlay::Confirm(c)) if c.buttons.iter().any(|b| b.0 == "All: Save")));
    h.key("a");
    assert!(h.app.quit);
    assert_eq!(fs::read_to_string(d.path().join("a.txt")).unwrap(), "1alpha line\nsecond\n");
    assert_eq!(fs::read_to_string(d.path().join("b.txt")).unwrap(), "2beta\n");
}

#[test]
fn untitled_save_during_quit_prompts_for_a_path() {
    let d = folder();
    let mut h = Harness::new();
    open_folder(&mut h, d.path());
    h.key("ctrl+n");
    h.type_text("draft");
    h.key("ctrl+q");
    h.key("s"); // Save
    assert!(matches!(h.app.overlays.last(), Some(Overlay::Prompt(_))), "Save As prompt for the untitled file");
    if let Some(Overlay::Prompt(p)) = h.app.overlays.last_mut() {
        p.input = yace_app::overlay::LineInput::new(d.path().join("draft.txt").display().to_string());
    }
    h.key("enter");
    assert!(h.app.quit, "quit continues after Save As");
    assert_eq!(fs::read_to_string(d.path().join("draft.txt")).unwrap(), "draft");
}

#[test]
fn reopen_closed_editor_and_focus_parts() {
    let d = folder();
    let mut h = Harness::new();
    open_folder(&mut h, d.path());
    h.app.open_file(&d.path().join("a.txt"), false);
    h.app.open_file(&d.path().join("b.txt"), false);
    h.key("ctrl+w");
    assert_eq!(h.app.docs.name(h.app.active().unwrap().2), "a.txt");
    h.key("ctrl+shift+t");
    assert_eq!(h.app.docs.name(h.app.active().unwrap().2), "b.txt");
    assert_eq!(h.app.focus, Focus::Editor);
    h.key("f6");
    assert_eq!(h.app.focus, Focus::Explorer);
    h.key("f6");
    assert_eq!(h.app.focus, Focus::Editor);
}
