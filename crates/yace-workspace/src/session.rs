//! Persistent per-workspace session state (spec §4.9) and the recent
//! workspaces list. Files are JSON, written atomically; a corrupt file is
//! reported and ignored (spec §26.2).

use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

pub const STATE_VERSION: u32 = 1;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkspaceState {
    pub version: u32,
    pub sidebar_visible: Option<bool>,
    pub sidebar_width: Option<u16>,
    /// Keyed by the root's absolute path (survives renames and reordering).
    pub explorer: BTreeMap<String, ExplorerRootState>,
    /// Selected explorer row: (root path, relative path).
    pub explorer_selected: Option<(String, String)>,
    pub explorer_scroll: usize,
    pub editor: Option<LayoutState>,
    /// Index into the flattened group order.
    pub active_group: usize,
    pub focus_explorer: bool,
    /// Most recent first.
    pub recent_files: Vec<PathBuf>,
    /// Quick Open picks as (root path, relative path), most recent first.
    pub quick_open_picked: Vec<(String, String)>,
    /// Recently used command ids, most recent first.
    pub recent_commands: Vec<String>,
    pub panel_visible: bool,
    pub panel_height: Option<u16>,
    pub panel_maximized: bool,
    /// Terminal tabs (metadata only: processes never survive a restart).
    pub terminals: Vec<TerminalState>,
    pub active_terminal: usize,
    /// Plugins enabled / disabled for this workspace only (spec §18.11);
    /// kept here so the workspace file itself is never changed.
    pub plugins_enabled: Vec<String>,
    pub plugins_disabled: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TerminalState {
    pub name: String,
    pub cwd: PathBuf,
    /// Absolute path of the workspace root it belonged to.
    pub root_path: Option<String>,
    pub profile: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExplorerRootState {
    pub collapsed: bool,
    /// Expanded directories, relative to the root.
    pub expanded: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SplitAxis {
    /// Children side by side.
    Horizontal,
    /// Children stacked.
    Vertical,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum LayoutState {
    Group(GroupState),
    Split { axis: SplitAxis, ratios: Vec<f32>, children: Vec<LayoutState> },
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GroupState {
    pub tabs: Vec<TabState>,
    pub active: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TabState {
    pub path: PathBuf,
    pub preview: bool,
    /// Primary cursor as (line, column-in-chars); robust to small external edits.
    pub cursor: (usize, usize),
    pub anchor: Option<(usize, usize)>,
    pub scroll_line: usize,
    pub scroll_col: usize,
    /// Collapsed regions as (header line, last hidden line).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub folds: Vec<(usize, usize)>,
}

/// State file path for a workspace identity.
pub fn state_file(dir: &Path, identity: &str) -> PathBuf {
    let h = fnv(identity.as_bytes());
    dir.join(format!("{h:016x}.json"))
}

fn fnv(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Load state. `Ok(None)` if absent; `Err` (with a message) if corrupt.
pub fn load(path: &Path) -> Result<Option<WorkspaceState>, String> {
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text)
            .map(Some)
            .map_err(|e| format!("{}: ignoring corrupt session state ({e})", path.display())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

pub fn save(path: &Path, state: &WorkspaceState) -> io::Result<()> {
    let mut s = state.clone();
    s.version = STATE_VERSION;
    let json = serde_json::to_vec_pretty(&s).map_err(io::Error::other)?;
    yace_config::atomic_write(path, &json)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecentKind {
    Folder,
    Workspace,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentEntry {
    pub kind: RecentKind,
    pub path: PathBuf,
    /// Display label (workspace or folder name).
    pub label: String,
    pub opened_at: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RecentList {
    pub entries: Vec<RecentEntry>,
}

const RECENT_LIMIT: usize = 30;

impl RecentList {
    pub fn load(path: &Path) -> (Self, Option<String>) {
        match fs::read_to_string(path) {
            Ok(t) => match serde_json::from_str(&t) {
                Ok(l) => (l, None),
                Err(e) => (Self::default(), Some(format!("{}: ignoring corrupt recent list ({e})", path.display()))),
            },
            Err(_) => (Self::default(), None),
        }
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        let json = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        yace_config::atomic_write(path, &json)
    }

    /// Record an open (moves the entry to the front).
    pub fn touch(&mut self, kind: RecentKind, path: PathBuf, label: String) {
        self.entries.retain(|e| e.path != path);
        let opened_at = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        self.entries.insert(0, RecentEntry { kind, path, label, opened_at });
        self.entries.truncate(RECENT_LIMIT);
    }

    pub fn remove(&mut self, path: &Path) {
        self.entries.retain(|e| e.path != path);
    }

    pub fn most_recent(&self) -> Option<&RecentEntry> {
        self.entries.first()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_corruption() {
        let dir = tempfile::tempdir().unwrap();
        let p = state_file(dir.path(), "folder:/x");
        assert_eq!(load(&p).unwrap(), None);
        let mut st = WorkspaceState { sidebar_width: Some(40), ..Default::default() };
        st.editor = Some(LayoutState::Split {
            axis: SplitAxis::Horizontal,
            ratios: vec![0.5, 0.5],
            children: vec![
                LayoutState::Group(GroupState {
                    tabs: vec![TabState { path: "/a".into(), cursor: (3, 4), ..Default::default() }],
                    active: 0,
                }),
                LayoutState::Group(GroupState::default()),
            ],
        });
        save(&p, &st).unwrap();
        let back = load(&p).unwrap().unwrap();
        assert_eq!(back.sidebar_width, Some(40));
        assert_eq!(back.editor, st.editor);
        fs::write(&p, "{ not json").unwrap();
        assert!(load(&p).is_err());
    }

    #[test]
    fn recent_list_dedupes_and_caps() {
        let mut r = RecentList::default();
        for i in 0..40 {
            r.touch(RecentKind::Folder, PathBuf::from(format!("/p{i}")), format!("p{i}"));
        }
        r.touch(RecentKind::Folder, PathBuf::from("/p5"), "p5".into());
        assert_eq!(r.entries.len(), RECENT_LIMIT);
        assert_eq!(r.most_recent().unwrap().path, PathBuf::from("/p5"));
        assert_eq!(r.entries.iter().filter(|e| e.path == Path::new("/p5")).count(), 1);
    }
}
