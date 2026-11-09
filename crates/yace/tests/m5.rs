//! M5: integrated terminal — real shells on PTYs (spec §11, §28.3).

mod common;

use std::time::Duration;

use common::{Harness, multi_root};
use yace_app::{
    app::Focus,
    cli::{Launch, Target},
    initial_workspace,
    overlay::Overlay,
};

const SH: &str = "[terminal]\nshell = \"/bin/sh\"\n[terminal.env]\nPS1 = \"$ \"\nENV = \"\"\n";
const T: Duration = Duration::from_secs(10);

fn open_ws(h: &mut Harness, ws: &std::path::Path) {
    let launch =
        Launch { target: Target::WorkspaceFile(ws.to_path_buf()), add_roots: vec![], goto: None, restore: true };
    initial_workspace(&mut h.app, &launch);
    h.settle();
}

fn screen_has(h: &mut Harness, needle: &str) -> bool {
    h.screen().contains(needle)
}

fn run(h: &mut Harness, cmd: &str) {
    h.type_text(cmd);
    h.key("enter");
}

fn canon(p: &std::path::Path) -> String {
    std::fs::canonicalize(p).unwrap().display().to_string()
}

#[test]
fn terminal_opens_in_the_active_root_and_routes_input() {
    let m = multi_root();
    let mut h = Harness::with_config(SH);
    open_ws(&mut h, &m.ws_file);
    h.app.open_file(&m.web.join("next.config.ts"), false);
    let before = h.active_text();
    h.key("ctrl+`");
    assert_eq!(h.app.focus, Focus::Terminal);
    assert!(h.wait_for(T, |h| screen_has(h, "$ ")), "prompt");
    assert!(screen_has(&mut h, "LEARN APP"), "tab shows its root");
    run(&mut h, "pwd");
    let want = canon(&m.web);
    assert!(h.wait_for(T, |h| screen_has(h, &want)), "started in the active root:\n{}", h.screen());
    assert_eq!(h.active_text(), before, "typing went to the shell, not the editor");
    // Skip-shell commands still work while the terminal has focus.
    h.key("ctrl+p");
    assert!(matches!(h.app.overlays.last(), Some(Overlay::Picker(_))));
    h.key("escape");
    assert_eq!(h.app.focus, Focus::Terminal);
    // Ctrl+` hides the panel and returns to the editor.
    h.key("ctrl+`");
    assert!(!h.app.panel.visible);
    assert_eq!(h.app.focus, Focus::Editor);
}

#[test]
fn two_terminals_in_different_roots_are_independent() {
    let m = multi_root();
    let mut h = Harness::with_config(SH);
    open_ws(&mut h, &m.ws_file);
    for name in ["Global API", "Chat API"] {
        h.command("yace.terminal.newInRoot");
        h.type_text(name);
        h.key("enter");
        assert!(h.wait_for(T, |h| screen_has(h, "$ ")));
    }
    assert_eq!(h.app.panel.tabs.len(), 2);
    run(&mut h, "pwd");
    let chat = canon(&m.chat);
    assert!(h.wait_for(T, |h| screen_has(h, &chat)));
    h.key("ctrl+pageup");
    assert_eq!(h.app.panel.active, 0);
    run(&mut h, "pwd");
    let api = canon(&m.api);
    assert!(h.wait_for(T, |h| screen_has(h, &api)), "{}", h.screen());
    let screen = h.screen();
    assert!(screen.contains("GLOBAL API") && screen.contains("CHAT API"), "both tabs labelled by root");
}

#[test]
fn ctrl_c_interrupts_exit_restart_and_kill_confirmation() {
    let d = tempfile::tempdir().unwrap();
    let mut h = Harness::with_config(SH);
    initial_workspace(
        &mut h.app,
        &Launch { target: Target::Folder(d.path().to_path_buf()), add_roots: vec![], goto: None, restore: false },
    );
    h.settle();
    h.key("ctrl+`");
    assert!(h.wait_for(T, |h| screen_has(h, "$ ")));
    run(&mut h, "sleep 30");
    assert!(h.wait_for(T, |h| h.app.panel.active_tab().unwrap().has_running_job()), "job running");
    h.key("ctrl+c");
    assert!(h.wait_for(T, |h| !h.app.panel.active_tab().unwrap().has_running_job()), "Ctrl+C reached the shell");
    // Kill with a running job asks first.
    run(&mut h, "sleep 30");
    assert!(h.wait_for(T, |h| h.app.panel.active_tab().unwrap().has_running_job()));
    h.command("workbench.action.terminal.kill");
    assert!(matches!(h.app.overlays.last(), Some(Overlay::Confirm(c)) if c.title == "Kill terminal"));
    h.key("escape");
    assert_eq!(h.app.panel.tabs.len(), 1, "cancel keeps it");
    h.key("ctrl+c");
    // Exit shows the code; Enter restarts.
    run(&mut h, "exit 7");
    assert!(h.wait_for(T, |h| h.app.panel.active_tab().unwrap().exited == Some(Some(7))));
    assert!(screen_has(&mut h, "exited with code 7"));
    h.key("enter");
    assert!(h.wait_for(T, |h| h.app.panel.active_tab().unwrap().is_running()), "restarted");
    // Idle shell: kill without asking.
    h.command("workbench.action.terminal.kill");
    assert!(h.app.panel.tabs.is_empty());
    assert!(!h.app.panel.visible);
}

#[test]
fn quitting_with_running_terminals_asks() {
    let d = tempfile::tempdir().unwrap();
    let mut h = Harness::with_config(SH);
    initial_workspace(
        &mut h.app,
        &Launch { target: Target::Folder(d.path().to_path_buf()), add_roots: vec![], goto: None, restore: false },
    );
    h.settle();
    h.key("ctrl+`");
    assert!(h.wait_for(T, |h| screen_has(h, "$ ")));
    run(&mut h, "sleep 30");
    assert!(h.wait_for(T, |h| h.app.panel.running_jobs() == 1));
    let pid = h.app.panel.active_tab().unwrap().session.as_ref().unwrap().pid();
    h.app.execute("workbench.action.quit", None);
    assert!(matches!(h.app.overlays.last(), Some(Overlay::Confirm(c)) if c.title == "Running processes"));
    h.key("escape");
    assert!(!h.app.quit);
    h.app.execute("workbench.action.quit", None);
    h.key("k"); // Kill and Continue
    assert!(h.app.quit);
    assert!(
        h.wait_for(T, |_| {
            let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
            stat.is_empty() || stat.contains(") Z")
        }),
        "shell stopped, no orphan"
    );
}

#[test]
fn panel_toggle_resize_and_pty_size() {
    let d = tempfile::tempdir().unwrap();
    let mut h = Harness::with_config(SH);
    initial_workspace(
        &mut h.app,
        &Launch { target: Target::Folder(d.path().to_path_buf()), add_roots: vec![], goto: None, restore: false },
    );
    h.settle();
    h.key("ctrl+j");
    assert!(h.app.panel.visible);
    assert!(h.wait_for(T, |h| screen_has(h, "$ ")));
    let rows = yace_app::ui::workbench_layout(&h.app).panel_content.unwrap().height;
    h.command("yace.panel.increaseHeight");
    h.settle();
    let bigger = yace_app::ui::workbench_layout(&h.app).panel_content.unwrap();
    assert!(bigger.height > rows);
    run(&mut h, "stty size");
    let want = format!("{} {}", bigger.height, bigger.width);
    assert!(h.wait_for(T, |h| screen_has(h, &want)), "PTY resized to the panel ({want}):\n{}", h.screen());
    h.command("workbench.action.toggleMaximizedPanel");
    assert!(yace_app::ui::workbench_layout(&h.app).panel_content.unwrap().height > bigger.height);
    h.key("ctrl+j");
    assert!(!h.app.panel.visible);
}

#[test]
fn selection_copy_and_paste() {
    let d = tempfile::tempdir().unwrap();
    let mut h = Harness::with_config(SH);
    initial_workspace(
        &mut h.app,
        &Launch { target: Target::Folder(d.path().to_path_buf()), add_roots: vec![], goto: None, restore: false },
    );
    h.settle();
    h.key("ctrl+`");
    assert!(h.wait_for(T, |h| screen_has(h, "$ ")));
    run(&mut h, "echo SELECT''ME");
    assert!(h.wait_for(T, |h| screen_has(h, "SELECTME")));
    let tab = h.app.panel.active_tab().unwrap();
    let screen = yace_app::app::terminal_snapshot(tab).unwrap();
    let row = (0..screen.rows).find(|r| screen.row_text(*r) == "SELECTME").expect("output row");
    let s = tab.session.as_ref().unwrap();
    s.select_start(row, 0, false);
    s.select_update(row, 7);
    h.key("ctrl+c");
    assert_eq!(h.app.clipboard.get().text, "SELECTME", "Ctrl+C copies a selection instead of interrupting");
    // Paste goes to the shell.
    h.app.handle(yace_app::app::AppEvent::Terminal(ratatui::crossterm::event::Event::Paste("echo PAS''TED".into())));
    h.key("enter");
    assert!(h.wait_for(T, |h| screen_has(h, "PASTED")));
}

#[test]
fn restored_session_starts_new_shells() {
    let m = multi_root();
    let state = tempfile::tempdir().unwrap();
    let cfg = state.path().join("config");
    std::fs::create_dir_all(&cfg).unwrap();
    std::fs::write(cfg.join("config.toml"), SH).unwrap();
    let state_path = state.path().to_path_buf();
    {
        let mut h = Harness::with_state(state);
        open_ws(&mut h, &m.ws_file);
        h.command("yace.terminal.newInRoot");
        h.type_text("Chat API");
        h.key("enter");
        assert!(h.wait_for(T, |h| screen_has(h, "$ ")));
        h.app.save_session();
        std::mem::forget(h.state);
    }
    let reopened = tempfile::TempDir::new_in(state_path.parent().unwrap()).unwrap();
    copy_dir(&state_path, reopened.path());
    let mut h = Harness::with_state(reopened);
    open_ws(&mut h, &m.ws_file);
    assert_eq!(h.app.panel.tabs.len(), 1);
    assert!(h.app.panel.visible);
    // Any event: the visible restored tab starts a fresh shell.
    h.app.handle(yace_app::app::AppEvent::Terminal(ratatui::crossterm::event::Event::Resize(120, 40)));
    assert!(h.wait_for(T, |h| screen_has(h, "$ ")), "{}", h.screen());
    let tab = h.app.panel.active_tab().unwrap();
    assert!(tab.name.contains("(new session)"), "never pretends the old process survived: {}", tab.name);
    h.command("workbench.action.terminal.focus");
    run(&mut h, "pwd");
    let chat = canon(&m.chat);
    assert!(h.wait_for(T, |h| screen_has(h, &chat)));
    let _ = std::fs::remove_dir_all(&state_path);
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let p = to.join(e.file_name());
        if e.path().is_dir() {
            copy_dir(&e.path(), &p);
        } else {
            std::fs::copy(e.path(), p).unwrap();
        }
    }
}

#[test]
fn f6_cycles_editor_terminal_sidebar() {
    let m = multi_root();
    let mut h = Harness::with_config(SH);
    open_ws(&mut h, &m.ws_file);
    h.app.open_file(&m.api.join("package.json"), false);
    h.key("ctrl+`");
    assert!(h.wait_for(T, |h| screen_has(h, "$ ")));
    assert_eq!(h.app.focus, Focus::Terminal);
    h.key("f6");
    assert_eq!(h.app.focus, Focus::Explorer, "terminal → sidebar");
    h.key("f6");
    assert_eq!(h.app.focus, Focus::Editor, "sidebar → editor");
    assert!(h.app.panel.visible, "the panel stays open");
    h.key("f6");
    assert_eq!(h.app.focus, Focus::Terminal, "editor → terminal");
    h.key("shift+f6");
    assert_eq!(h.app.focus, Focus::Editor, "Shift+F6 goes back to the editor");
}

/// Ctrl+F in a terminal searches its scrollback: typing finds the newest
/// match first, Enter goes up to older output, Shift+Enter back down; the
/// match is selected (Ctrl+C copies it) and the shell gets none of the
/// keys typed into the find bar.
#[test]
fn find_in_the_terminal_scrollback() {
    let d = tempfile::tempdir().unwrap();
    let mut h = Harness::with_config(SH);
    initial_workspace(
        &mut h.app,
        &Launch { target: Target::Folder(d.path().to_path_buf()), add_roots: vec![], goto: None, restore: false },
    );
    h.settle();
    h.key("ctrl+`");
    assert!(h.wait_for(T, |h| screen_has(h, "$ ")));
    run(&mut h, "i=0; while [ $i -lt 400 ]; do echo item$i; i=$((i+1)); done; echo LIST''DONE");
    assert!(h.wait_for(T, |h| screen_has(h, "LISTDONE")));
    h.key("ctrl+f");
    assert_eq!(h.app.focus, Focus::TerminalFind);
    h.type_text("item25");
    assert_eq!(h.app.term_find.status(), "11 of 11", "item25, item250–259: the newest is selected");
    assert!(screen_has(&mut h, "item259"), "scrolled to it");
    let tab = h.app.panel.active_tab().unwrap();
    assert_eq!(tab.session.as_ref().unwrap().selection_text().as_deref(), Some("item25"));
    h.key("enter");
    assert_eq!(h.app.term_find.status(), "10 of 11", "Enter goes up");
    h.key("shift+enter");
    h.key("shift+enter");
    assert_eq!(h.app.term_find.status(), "1 of 11", "past the newest wraps to the oldest");
    // Whole word: only item25.
    h.key("alt+w");
    assert_eq!(h.app.term_find.status(), "1 of 1");
    h.key("alt+w");
    h.key("alt+r");
    h.key("ctrl+a");
    h.type_text("item3\\d\\d");
    assert_eq!(h.app.term_find.status(), "100 of 100");
    h.type_text("(");
    assert!(h.app.term_find.error.is_some(), "invalid regex is reported");
    h.key("backspace");
    // Escape closes the bar; the match stays selected, so Ctrl+C copies it.
    h.key("escape");
    assert_eq!(h.app.focus, Focus::Terminal);
    assert!(!h.app.term_find.visible);
    h.key("ctrl+c");
    assert_eq!(h.app.clipboard.get().text, "item399");
    // None of the find-bar typing reached the shell.
    run(&mut h, "echo AFTER''FIND");
    assert!(h.wait_for(T, |h| screen_has(h, "AFTERFIND")));
    assert!(!screen_has(&mut h, "item25item"), "no stray input in the shell");
}

fn pane_rects(h: &Harness) -> Vec<(usize, ratatui::layout::Rect)> {
    let content = yace_app::ui::workbench_layout(&h.app).panel_content.unwrap();
    yace_app::ui::terminal_panes(&h.app, content)
}

/// Split terminals: shells side by side, each PTY sized to its pane, keys
/// to the focused one; Alt+arrows move between panes, Ctrl+Shift+arrows
/// resize; one tab per group; kill, unsplit, and the session round trip.
#[test]
fn split_terminals_side_by_side() {
    let d = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let cfg = state.path().join("config");
    std::fs::create_dir_all(&cfg).unwrap();
    std::fs::write(cfg.join("config.toml"), SH).unwrap();
    let state_path = state.path().to_path_buf();
    let mut h = Harness::with_state(state);
    initial_workspace(
        &mut h.app,
        &Launch { target: Target::Folder(d.path().to_path_buf()), add_roots: vec![], goto: None, restore: false },
    );
    h.settle();
    h.key("ctrl+`");
    assert!(h.wait_for(T, |h| screen_has(h, "$ ")));
    h.key("ctrl+shift+5");
    assert_eq!(h.app.panel.tabs.len(), 2, "Ctrl+Shift+5 splits");
    let panes = pane_rects(&h);
    assert_eq!(panes.len(), 2);
    assert_eq!(panes[1].0, h.app.panel.active, "the new pane has focus");
    let (a, b) = (panes[0].1, panes[1].1);
    assert_eq!(a.right() + 1, b.x, "one-column divider");
    assert!(a.width.abs_diff(b.width) <= 1, "split evenly: {a:?} {b:?}");
    assert!(h.wait_for(T, |h| {
        h.app.panel.tabs[1].session.as_ref().is_some_and(|s| {
            let mut sc = yace_terminal::Screen::default();
            s.screen(&mut sc);
            sc.text().contains('$')
        })
    }));
    // Each PTY matches its pane.
    run(&mut h, "stty size; echo RIGHT''PANE");
    let want_b = format!("{} {}", b.height, b.width);
    assert!(h.wait_for(T, |h| screen_has(h, &want_b) && screen_has(h, "RIGHTPANE")), "{}", h.screen());
    h.key("alt+left");
    assert_eq!(h.app.panel.active, panes[0].0);
    run(&mut h, "stty size; echo LEFT''PANE");
    let want_a = format!("{} {}", a.height, a.width);
    assert!(h.wait_for(T, |h| screen_has(h, &want_a) && screen_has(h, "LEFTPANE")));
    // The tab strip shows one tab for the group.
    assert!(screen_has(&mut h, "1: sh │ sh"), "{}", h.screen());
    // Resize: the left pane grows.
    h.key("ctrl+shift+right");
    h.settle();
    let wider = pane_rects(&h)[0].1;
    assert!(wider.width > a.width, "{wider:?} vs {a:?}");
    run(&mut h, "stty size");
    let want = format!("{} {}", wider.height, wider.width);
    assert!(h.wait_for(T, |h| screen_has(h, &want)), "PTY follows the resize");
    // A second tab, then back: Ctrl+PageDown moves between tabs (groups).
    h.command("workbench.action.terminal.new");
    assert_eq!(pane_rects(&h).len(), 1);
    h.key("ctrl+pagedown");
    assert_eq!(pane_rects(&h).len(), 2, "back to the split tab");
    // Kill one pane: the other stays, alone in its tab.
    h.app.panel.active = pane_rects(&h)[1].0;
    h.command("workbench.action.terminal.kill");
    assert_eq!(h.app.panel.tabs.len(), 2);
    assert_eq!(pane_rects(&h).len(), 1);
    // Split again (Alt+\ in a terminal without the enhanced keyboard
    // protocol) and unsplit: two tabs of one pane each.
    h.app.keyboard_enhanced = false;
    h.key("alt+\\");
    h.app.keyboard_enhanced = true;
    assert_eq!(pane_rects(&h).len(), 2);
    h.command("workbench.action.terminal.unsplit");
    assert_eq!(pane_rects(&h).len(), 1);
    assert_eq!(h.app.panel.groups().len(), 3);
    h.command("workbench.action.terminal.split");
    let groups = h.app.panel.groups().len();
    let tabs = h.app.panel.tabs.len();
    // Session: the group survives a restart.
    h.app.save_session();
    std::mem::forget(h.state);
    let reopened = tempfile::TempDir::new_in(state_path.parent().unwrap()).unwrap();
    copy_dir(&state_path, reopened.path());
    {
        let mut h2 = Harness::with_state(reopened);
        initial_workspace(
            &mut h2.app,
            &Launch { target: Target::Folder(d.path().to_path_buf()), add_roots: vec![], goto: None, restore: true },
        );
        h2.settle();
        assert_eq!(h2.app.panel.tabs.len(), tabs);
        assert_eq!(h2.app.panel.groups().len(), groups, "split groups are restored");
    }
    let _ = std::fs::remove_dir_all(&state_path);
}
