//! The multi-root workspace model (spec §4) and `.code-workspace`
//! compatibility (spec §4.2).
//!
//! Saving never rewrites a workspace file wholesale: the file is re-parsed
//! as a concrete syntax tree and only the `folders` array is edited, so
//! comments, formatting, `settings`, `extensions`, `launch` and any other
//! fields survive byte-for-byte (spec §33.12).

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use jsonc_parser::{
    ParseOptions,
    cst::{CstInputValue, CstRootNode},
};
use thiserror::Error;

use crate::paths;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RootId(pub u32);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RootStatus {
    Available,
    /// Not usable; the reason is shown to the user (spec §4.10).
    Missing(String),
    /// Same directory as an earlier root (spec §4.11). Not traversed twice.
    Duplicate(RootId),
}

#[derive(Clone, Debug)]
pub struct Root {
    pub id: RootId,
    /// Display name from the workspace file (`"name"`), if any.
    pub name: Option<String>,
    /// The path exactly as written in the workspace file.
    pub configured_path: String,
    /// Absolute, lexically normalised path.
    pub path: PathBuf,
    /// Canonical path (symlinks resolved) when the directory exists.
    pub canonical: Option<PathBuf>,
    pub status: RootStatus,
    /// Set when this root lies inside another root.
    pub nested_in: Option<RootId>,
}

impl Root {
    pub fn display_name(&self) -> String {
        self.name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| {
            self.path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| self.path.display().to_string())
        })
    }
    pub fn is_available(&self) -> bool {
        self.status == RootStatus::Available
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkspaceKind {
    /// A single folder opened directly (`yace .`).
    Folder,
    /// A `.code-workspace` file chosen by the user.
    File(PathBuf),
    /// A multi-root workspace not yet saved by the user; backed by a file
    /// in YACE's state directory so it can be reopened.
    Untitled(PathBuf),
}

#[derive(Debug, Error)]
pub enum WorkspaceError {
    #[error("{path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("{path}: invalid workspace file: {message}")]
    Parse { path: PathBuf, message: String },
    #[error("{0}")]
    Invalid(String),
}

#[derive(Clone, Debug)]
pub struct Workspace {
    pub kind: WorkspaceKind,
    roots: Vec<Root>,
    /// Raw `settings` object from the workspace file (read-only).
    pub settings: serde_json::Value,
    next_id: u32,
    /// Roots changed since the workspace file was last written.
    pub dirty: bool,
}

impl Workspace {
    pub fn empty_untitled(backing: PathBuf) -> Self {
        Self {
            kind: WorkspaceKind::Untitled(backing),
            roots: Vec::new(),
            settings: serde_json::Value::Null,
            next_id: 1,
            dirty: false,
        }
    }

    pub fn from_folder(path: &Path) -> Self {
        let mut ws = Self {
            kind: WorkspaceKind::Folder,
            roots: Vec::new(),
            settings: serde_json::Value::Null,
            next_id: 1,
            dirty: false,
        };
        let abs = paths::absolutize(path);
        ws.push_root(abs.display().to_string(), abs, None);
        ws.refresh_status();
        ws
    }

    /// Open and parse a `.code-workspace` file.
    pub fn open_file(path: &Path) -> Result<Self, WorkspaceError> {
        let path = paths::absolutize(path);
        let text = fs::read_to_string(&path).map_err(|source| WorkspaceError::Io { path: path.clone(), source })?;
        let mut ws = Self::parse(&text, &path)?;
        ws.kind = WorkspaceKind::File(path);
        Ok(ws)
    }

    /// Parse workspace JSONC; `file` is used to resolve relative paths.
    pub fn parse(text: &str, file: &Path) -> Result<Self, WorkspaceError> {
        let json = yace_config::parse_jsonc(text)
            .map_err(|message| WorkspaceError::Parse { path: file.to_path_buf(), message })?;
        let obj = json.as_object().ok_or_else(|| WorkspaceError::Parse {
            path: file.to_path_buf(),
            message: "expected a JSON object".into(),
        })?;
        let base = file.parent();
        let mut ws = Self {
            kind: WorkspaceKind::File(file.to_path_buf()),
            roots: Vec::new(),
            settings: obj.get("settings").cloned().unwrap_or(serde_json::Value::Null),
            next_id: 1,
            dirty: false,
        };
        let folders = obj.get("folders").and_then(|f| f.as_array()).cloned().unwrap_or_default();
        for entry in folders {
            let name = entry.get("name").and_then(|n| n.as_str()).map(str::to_string);
            if let Some(p) = entry.get("path").and_then(|p| p.as_str()) {
                let resolved = paths::resolve(p, base);
                ws.push_root(p.to_string(), resolved, name);
            } else if let Some(uri) = entry.get("uri").and_then(|u| u.as_str()) {
                if let Some(local) = uri.strip_prefix("file://") {
                    let decoded = percent_decode(local);
                    let resolved = paths::normalize(Path::new(&decoded));
                    ws.push_root(uri.to_string(), resolved, name);
                } else {
                    let id = ws.push_root(uri.to_string(), PathBuf::from(uri), name);
                    ws.root_mut(id).unwrap().status = RootStatus::Missing("remote folders are not supported".into());
                }
            }
        }
        ws.refresh_status();
        Ok(ws)
    }

    fn push_root(&mut self, configured: String, path: PathBuf, name: Option<String>) -> RootId {
        let id = RootId(self.next_id);
        self.next_id += 1;
        self.roots.push(Root {
            id,
            name,
            configured_path: configured,
            path,
            canonical: None,
            status: RootStatus::Available,
            nested_in: None,
        });
        id
    }

    pub fn roots(&self) -> &[Root] {
        &self.roots
    }
    pub fn root(&self, id: RootId) -> Option<&Root> {
        self.roots.iter().find(|r| r.id == id)
    }
    fn root_mut(&mut self, id: RootId) -> Option<&mut Root> {
        self.roots.iter_mut().find(|r| r.id == id)
    }
    pub fn file(&self) -> Option<&Path> {
        match &self.kind {
            WorkspaceKind::File(p) | WorkspaceKind::Untitled(p) => Some(p),
            WorkspaceKind::Folder => None,
        }
    }

    /// Human-readable workspace name for the title area.
    pub fn name(&self) -> String {
        match &self.kind {
            WorkspaceKind::Folder => self.roots.first().map(Root::display_name).unwrap_or_default(),
            WorkspaceKind::File(p) => p
                .file_stem()
                .map(|s| s.to_string_lossy().trim_end_matches(".code-workspace").to_string())
                .unwrap_or_else(|| "Workspace".into()),
            WorkspaceKind::Untitled(_) => "Untitled (Workspace)".into(),
        }
    }

    /// Stable identity used to key session state.
    pub fn identity(&self) -> String {
        match &self.kind {
            WorkspaceKind::Folder => {
                let r = self.roots.first();
                let p = r.and_then(|r| r.canonical.clone()).or_else(|| r.map(|r| r.path.clone())).unwrap_or_default();
                format!("folder:{}", p.display())
            }
            WorkspaceKind::File(p) | WorkspaceKind::Untitled(p) => {
                let c = fs::canonicalize(p).unwrap_or_else(|_| p.clone());
                format!("file:{}", c.display())
            }
        }
    }

    /// Re-check every root's availability, duplicates and nesting.
    pub fn refresh_status(&mut self) {
        for r in &mut self.roots {
            if matches!(&r.status, RootStatus::Missing(m) if m.starts_with("remote")) {
                continue;
            }
            r.nested_in = None;
            r.canonical = None;
            r.status = match fs::metadata(&r.path) {
                Ok(m) if m.is_dir() => match fs::read_dir(&r.path) {
                    Ok(_) => {
                        r.canonical = fs::canonicalize(&r.path).ok();
                        RootStatus::Available
                    }
                    Err(e) => RootStatus::Missing(describe_io(&e)),
                },
                Ok(_) => RootStatus::Missing("not a directory".into()),
                Err(e) => RootStatus::Missing(describe_io(&e)),
            };
        }
        let snapshot: Vec<(RootId, Option<PathBuf>)> = self.roots.iter().map(|r| (r.id, r.canonical.clone())).collect();
        for (i, r) in self.roots.iter_mut().enumerate() {
            let Some(c) = &r.canonical else { continue };
            if let Some((dup, _)) = snapshot[..i].iter().find(|(_, oc)| oc.as_ref() == Some(c)) {
                r.status = RootStatus::Duplicate(*dup);
            }
        }
        // Nesting is computed among distinct (non-duplicate) roots only.
        let distinct: Vec<(RootId, PathBuf)> =
            self.roots.iter().filter(|r| r.is_available()).filter_map(|r| Some((r.id, r.canonical.clone()?))).collect();
        for r in &mut self.roots {
            let Some(c) = r.canonical.clone().filter(|_| r.status == RootStatus::Available) else { continue };
            r.nested_in = distinct
                .iter()
                .filter(|(id, o)| *id != r.id && c != *o && c.starts_with(o))
                .max_by_key(|(_, o)| o.components().count())
                .map(|(id, _)| *id);
        }
    }

    /// Add a folder. The path is stored absolute (or relative when it is
    /// inside the workspace file's directory, as VS Code does).
    pub fn add_root(&mut self, path: &Path, name: Option<String>) -> RootId {
        let abs = paths::absolutize(path);
        let configured = match self.file().and_then(Path::parent) {
            Some(dir) if !matches!(self.kind, WorkspaceKind::Untitled(_)) && abs.starts_with(dir) => {
                let rel = abs.strip_prefix(dir).unwrap_or(&abs);
                if rel.as_os_str().is_empty() { ".".to_string() } else { rel.display().to_string() }
            }
            _ => abs.display().to_string(),
        };
        let id = self.push_root(configured, abs, name);
        self.dirty = true;
        self.refresh_status();
        id
    }

    /// Remove a root from the workspace. **Never touches the filesystem.**
    pub fn remove_root(&mut self, id: RootId) -> bool {
        let before = self.roots.len();
        self.roots.retain(|r| r.id != id);
        let removed = self.roots.len() != before;
        if removed {
            self.dirty = true;
            self.refresh_status();
        }
        removed
    }

    pub fn rename_root(&mut self, id: RootId, name: Option<String>) {
        if let Some(r) = self.root_mut(id) {
            r.name = name.filter(|n| !n.trim().is_empty());
            self.dirty = true;
        }
    }

    /// Point a (missing) root at a new directory.
    pub fn relocate_root(&mut self, id: RootId, path: &Path) {
        let abs = paths::absolutize(path);
        if let Some(r) = self.root_mut(id) {
            r.configured_path = abs.display().to_string();
            r.path = abs;
            self.dirty = true;
        }
        self.refresh_status();
    }

    /// Move a root up (`-1`) or down (`+1`) in the list.
    pub fn move_root(&mut self, id: RootId, delta: isize) -> bool {
        let Some(i) = self.roots.iter().position(|r| r.id == id) else { return false };
        let j = i as isize + delta;
        if j < 0 || j as usize >= self.roots.len() {
            return false;
        }
        self.roots.swap(i, j as usize);
        self.dirty = true;
        true
    }

    /// The deepest available root containing `path`.
    pub fn root_for_path(&self, path: &Path) -> Option<&Root> {
        self.roots
            .iter()
            .filter(|r| {
                r.is_available()
                    && (path.starts_with(&r.path) || r.canonical.as_ref().is_some_and(|c| path.starts_with(c)))
            })
            .max_by_key(|r| r.path.components().count())
    }

    /// `(root, path relative to that root)`.
    pub fn relative(&self, path: &Path) -> Option<(&Root, PathBuf)> {
        let root = self.root_for_path(path)?;
        let rel = path
            .strip_prefix(&root.path)
            .ok()
            .or_else(|| root.canonical.as_ref().and_then(|c| path.strip_prefix(c).ok()))?
            .to_path_buf();
        Some((root, rel))
    }

    /// Convert to an untitled multi-root workspace (when a second folder is
    /// added to a single-folder session).
    pub fn make_untitled(&mut self, backing: PathBuf) {
        if self.kind == WorkspaceKind::Folder {
            self.kind = WorkspaceKind::Untitled(backing);
            self.dirty = true;
        }
    }

    /// Save to the current file (`File`/`Untitled` kinds).
    pub fn save(&mut self) -> Result<(), WorkspaceError> {
        let Some(path) = self.file().map(Path::to_path_buf) else {
            return Err(WorkspaceError::Invalid(
                "a single-folder workspace has no workspace file; use Save Workspace As".into(),
            ));
        };
        self.write_to(&path)?;
        self.dirty = false;
        Ok(())
    }

    /// Save to a new `.code-workspace` file and make it the workspace file.
    /// Relative root paths are rewritten so they still resolve.
    pub fn save_as(&mut self, path: &Path) -> Result<(), WorkspaceError> {
        let path = paths::absolutize(path);
        for r in &mut self.roots {
            if !Path::new(&r.configured_path).is_absolute() && !r.configured_path.contains("://") {
                r.configured_path = r.path.display().to_string();
            }
        }
        // Carry unrelated fields over from the previous file, if any.
        if let Some(old) = self.file().map(Path::to_path_buf)
            && old != path
            && old.exists()
            && !path.exists()
        {
            let _ = fs::copy(&old, &path);
        }
        self.write_to(&path)?;
        self.kind = WorkspaceKind::File(path);
        self.dirty = false;
        Ok(())
    }

    fn folder_value(r: &Root) -> CstInputValue {
        let mut props = Vec::new();
        if let Some(n) = &r.name {
            props.push(("name".to_string(), CstInputValue::String(n.clone())));
        }
        props.push(("path".to_string(), CstInputValue::String(r.configured_path.clone())));
        CstInputValue::Object(props)
    }

    /// Write the roots into `path`, editing only the `folders` array.
    fn write_to(&self, path: &Path) -> Result<(), WorkspaceError> {
        let io_err = |source| WorkspaceError::Io { path: path.to_path_buf(), source };
        let existing = match fs::read_to_string(path) {
            Ok(t) => Some(t),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(io_err(e)),
        };
        let text = match existing {
            Some(text) => update_folders(&text, &self.roots)
                .map_err(|message| WorkspaceError::Parse { path: path.to_path_buf(), message })?,
            None => {
                let root = CstRootNode::parse("{}", &ParseOptions::default())
                    .map_err(|e| WorkspaceError::Invalid(e.to_string()))?;
                let obj = root.object_value_or_set();
                obj.append("folders", CstInputValue::Array(self.roots.iter().map(Self::folder_value).collect()));
                obj.append("settings", CstInputValue::Object(vec![]));
                let mut s = root.to_string();
                if !s.ends_with('\n') {
                    s.push('\n');
                }
                s
            }
        };
        yace_config::atomic_write(path, text.as_bytes()).map_err(io_err)
    }
}

/// Minimal edit of the `folders` array of a workspace document.
pub fn update_folders(text: &str, roots: &[Root]) -> Result<String, String> {
    let root = CstRootNode::parse(text, &ParseOptions::default()).map_err(|e| e.to_string())?;
    let obj = root.object_value().ok_or("expected a JSON object")?;
    let arr = obj.array_value_or_set("folders");
    // Existing entries: (configured path/uri, name, node)
    let existing: Vec<(String, Option<String>, jsonc_parser::cst::CstNode)> = arr
        .elements()
        .into_iter()
        .map(|n| {
            let v = n.to_serde_value().unwrap_or_default();
            let key = v.get("path").or_else(|| v.get("uri")).and_then(|p| p.as_str()).unwrap_or_default().to_string();
            let name = v.get("name").and_then(|n| n.as_str()).map(str::to_string);
            (key, name, n)
        })
        .collect();
    let wanted: Vec<&str> = roots.iter().map(|r| r.configured_path.as_str()).collect();
    let current: Vec<&str> = existing.iter().map(|(k, _, _)| k.as_str()).collect();
    if current == wanted && existing.iter().zip(roots).all(|((_, n, _), r)| *n == r.name) {
        return Ok(text.to_string());
    }
    // Remove entries no longer wanted.
    let mut kept: Vec<(String, Option<String>, jsonc_parser::cst::CstNode)> = Vec::new();
    for (k, n, node) in existing {
        if wanted.contains(&k.as_str()) && !kept.iter().any(|(kk, _, _)| *kk == k) {
            kept.push((k, n, node));
        } else {
            node.remove();
        }
    }
    let kept_order: Vec<&str> = kept.iter().map(|(k, _, _)| k.as_str()).collect();
    let wanted_kept: Vec<&str> = wanted.iter().copied().filter(|w| kept_order.contains(w)).collect();
    if kept_order != wanted_kept {
        // Reordered: rebuild the array (comments inside it are lost; the rest
        // of the document is untouched).
        arr.replace_with(CstInputValue::Array(roots.iter().map(Workspace::folder_value).collect()));
        return Ok(root.to_string());
    }
    // Update names in place.
    for (k, n, node) in &kept {
        let r = roots.iter().find(|r| r.configured_path == *k).unwrap();
        if *n != r.name
            && let Some(o) = node.as_object()
        {
            match (&r.name, o.get("name")) {
                (Some(name), Some(prop)) => prop.set_value(CstInputValue::String(name.clone())),
                (Some(name), None) => {
                    o.insert(0, "name", CstInputValue::String(name.clone()));
                }
                (None, Some(prop)) => prop.remove(),
                (None, None) => {}
            }
        }
    }
    // Insert new entries at their positions.
    for (i, r) in roots.iter().enumerate() {
        if !kept_order.contains(&r.configured_path.as_str()) {
            arr.insert(i, Workspace::folder_value(r));
        }
    }
    Ok(root.to_string())
}

fn describe_io(e: &io::Error) -> String {
    match e.kind() {
        io::ErrorKind::NotFound => "not found".into(),
        io::ErrorKind::PermissionDenied => "permission denied".into(),
        _ => e.to_string(),
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        _dirs: Vec<tempfile::TempDir>,
        a: PathBuf,
        b: PathBuf,
        ws_dir: tempfile::TempDir,
    }

    fn fixture() -> Fixture {
        let d1 = tempfile::tempdir().unwrap();
        let d2 = tempfile::tempdir().unwrap();
        let a = d1.path().join("global api");
        let b = d2.path().join("lërn-app");
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        let ws_dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(ws_dir.path().join("sibling")).unwrap();
        Fixture { _dirs: vec![d1, d2], a, b, ws_dir }
    }

    fn sample_text(f: &Fixture) -> String {
        format!(
            r#"// My workspace
{{
  "folders": [
    // backend
    {{ "name": "Global API", "path": "{}" }},
    {{ "path": "{}" }},
    {{ "path": "./sibling" }},
    {{ "name": "Gone", "path": "/definitely/not/here" }},
  ],
  "settings": {{
    "files.autoSave": "off", // keep me
  }},
  "extensions": {{ "recommendations": ["x.y"] }},
}}
"#,
            f.a.display(),
            f.b.display()
        )
    }

    #[test]
    fn parses_all_cases() {
        let f = fixture();
        let file = f.ws_dir.path().join("my.code-workspace");
        fs::write(&file, sample_text(&f)).unwrap();
        let ws = Workspace::open_file(&file).unwrap();
        let roots = ws.roots();
        assert_eq!(roots.len(), 4);
        assert_eq!(roots[0].display_name(), "Global API");
        assert_eq!(roots[1].display_name(), "lërn-app");
        assert_eq!(roots[2].path, f.ws_dir.path().join("sibling"), "relative to the workspace file");
        assert!(roots[0].is_available() && roots[1].is_available() && roots[2].is_available());
        assert_eq!(roots[3].status, RootStatus::Missing("not found".into()));
        assert_eq!(ws.settings["files.autoSave"], "off");
        assert_eq!(ws.name(), "my");
    }

    #[test]
    fn edits_preserve_everything_else() {
        let f = fixture();
        let file = f.ws_dir.path().join("my.code-workspace");
        let original = sample_text(&f);
        fs::write(&file, &original).unwrap();
        let mut ws = Workspace::open_file(&file).unwrap();
        // No-op save keeps bytes identical.
        ws.save().unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), original);

        let gone = ws.roots()[3].id;
        assert!(ws.remove_root(gone));
        let extra = tempfile::tempdir().unwrap();
        ws.add_root(extra.path(), None);
        ws.rename_root(ws.roots()[1].id, Some("Learn App".into()));
        ws.save().unwrap();
        let saved = fs::read_to_string(&file).unwrap();
        assert!(saved.starts_with("// My workspace"));
        assert!(saved.contains("// backend"));
        assert!(saved.contains("// keep me"));
        assert!(saved.contains("\"recommendations\""));
        assert!(!saved.contains("/definitely/not/here"));
        let reparsed = Workspace::open_file(&file).unwrap();
        let names: Vec<String> = reparsed.roots().iter().map(Root::display_name).collect();
        assert_eq!(names[0], "Global API");
        assert_eq!(names[1], "Learn App");
        assert_eq!(reparsed.roots().len(), 4);
        assert_eq!(reparsed.roots()[3].path, paths::absolutize(extra.path()));

        // Reorder keeps the rest of the document.
        let mut ws = reparsed;
        let first = ws.roots()[0].id;
        assert!(ws.move_root(first, 1));
        ws.save().unwrap();
        let saved = fs::read_to_string(&file).unwrap();
        assert!(saved.contains("// keep me"));
        let reparsed = Workspace::open_file(&file).unwrap();
        assert_eq!(reparsed.roots()[1].display_name(), "Global API");
    }

    #[test]
    fn removing_a_root_never_deletes_files() {
        let f = fixture();
        fs::write(f.a.join("keep.txt"), "x").unwrap();
        let mut ws = Workspace::from_folder(&f.a);
        let id = ws.roots()[0].id;
        ws.remove_root(id);
        assert!(f.a.join("keep.txt").exists());
    }

    #[test]
    fn duplicates_and_nested_roots() {
        let f = fixture();
        let inner = f.a.join("packages/core");
        fs::create_dir_all(&inner).unwrap();
        let mut ws = Workspace::from_folder(&f.a);
        ws.add_root(&f.a, Some("again".into()));
        ws.add_root(&inner, None);
        let r = ws.roots();
        assert_eq!(r[1].status, RootStatus::Duplicate(r[0].id));
        assert_eq!(r[2].nested_in, Some(r[0].id));
        let deep = inner.join("lib.rs");
        assert_eq!(ws.root_for_path(&deep).unwrap().id, r[2].id, "deepest root wins");
        let (root, rel) = ws.relative(&f.a.join("src/x.rs")).unwrap();
        assert_eq!(root.id, r[0].id);
        assert_eq!(rel, PathBuf::from("src/x.rs"));
    }

    #[test]
    fn save_as_creates_new_file_and_rewrites_relative_paths() {
        let f = fixture();
        let file = f.ws_dir.path().join("my.code-workspace");
        fs::write(&file, sample_text(&f)).unwrap();
        let mut ws = Workspace::open_file(&file).unwrap();
        let other = tempfile::tempdir().unwrap();
        let new_file = other.path().join("copy.code-workspace");
        ws.save_as(&new_file).unwrap();
        let reparsed = Workspace::open_file(&new_file).unwrap();
        assert_eq!(reparsed.roots()[2].path, f.ws_dir.path().join("sibling"));
        assert!(fs::read_to_string(&new_file).unwrap().contains("recommendations"));
    }

    #[test]
    fn new_file_from_scratch() {
        let f = fixture();
        let file = f.ws_dir.path().join("fresh.code-workspace");
        let mut ws = Workspace::empty_untitled(file.clone());
        ws.add_root(&f.a, Some("A".into()));
        ws.add_root(&f.b, None);
        ws.save().unwrap();
        let reparsed = Workspace::open_file(&file).unwrap();
        assert_eq!(reparsed.roots().len(), 2);
        assert_eq!(reparsed.roots()[0].display_name(), "A");
    }

    #[test]
    fn invalid_json_is_reported() {
        let err = Workspace::parse("{ \"folders\": [ ", Path::new("/x.code-workspace")).unwrap_err();
        assert!(matches!(err, WorkspaceError::Parse { .. }));
    }

    #[test]
    fn file_uri_folders() {
        let f = fixture();
        let text = format!(
            r#"{{"folders":[{{"uri":"file://{}"}},{{"uri":"vscode-remote://ssh/x"}}]}}"#,
            f.a.display().to_string().replace(' ', "%20")
        );
        let ws = Workspace::parse(&text, &f.ws_dir.path().join("w.code-workspace")).unwrap();
        assert!(ws.roots()[0].is_available());
        assert!(matches!(ws.roots()[1].status, RootStatus::Missing(_)));
    }
}
