//! YACE application crate: state machine, commands, rendering and the
//! event loop. The binary (`main.rs`) wires these to a real terminal;
//! tests drive [`app::App`] directly.

pub mod actions;
pub mod app;
pub mod cli;
pub mod clipboard;
pub mod commands;
pub mod editor;
pub mod explorer;
pub mod metrics;
pub mod overlay;
pub mod run;
pub mod terminal;
pub mod theme;
pub mod ui;
pub mod workers;

use std::path::Path;

use app::{App, Severity};
use cli::{Launch, Target};
use yace_workspace::{Workspace, session::RecentKind};

/// Build the initial workspace from the launch plan.
pub fn initial_workspace(app: &mut App, launch: &Launch) {
    let mut goto = launch.goto.clone();
    let ws = match &launch.target {
        Target::Folder(p) => Some(Workspace::from_folder(p)),
        Target::WorkspaceFile(p) => match Workspace::open_file(p) {
            Ok(ws) => Some(ws),
            Err(e) => {
                app.notify(Severity::Error, e.to_string());
                None
            }
        },
        Target::File { path, line, col } => {
            goto = Some((path.clone(), *line, *col));
            Some(Workspace::from_folder(&cli::folder_for_file(path)))
        }
        Target::Recent => {
            let recent =
                if app.settings.workbench.restore_workspace { app.recent.most_recent().cloned() } else { None };
            recent.and_then(|e| {
                if !e.path.exists() {
                    return None;
                }
                match e.kind {
                    RecentKind::Folder => Some(Workspace::from_folder(&e.path)),
                    RecentKind::Workspace => Workspace::open_file(&e.path).ok(),
                }
            })
        }
    };
    let ws = ws.unwrap_or_else(|| Workspace::empty_untitled(app.new_untitled_backing()));
    app.install_workspace(ws, launch.restore);
    if !launch.add_roots.is_empty() {
        for r in &launch.add_roots {
            if app.workspace.kind == yace_workspace::WorkspaceKind::Folder {
                let backing = app.new_untitled_backing();
                app.workspace.make_untitled(backing);
            }
            app.workspace.add_root(r, None);
        }
        app.after_roots_changed();
    }
    if let Some((path, line, col)) = goto {
        open_at(app, &path, line, col);
    }
}

fn open_at(app: &mut App, path: &Path, line: Option<usize>, col: Option<usize>) {
    app.open_file(path, false);
    if let Some(l) = line {
        app.set_cursor_line_col(l.saturating_sub(1), col.unwrap_or(1).saturating_sub(1));
    }
}
