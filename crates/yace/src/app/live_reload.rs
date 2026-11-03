//! Live reload of configuration (spec §17.5): user settings, keybindings,
//! theme files and per-folder `.vscode/settings.json` apply when they change
//! on disk, through the same watcher as the Explorer. Files whose content
//! is unchanged (for example YACE's own writes, already applied) are skipped.

use std::{
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
};

use super::{App, Severity};
use crate::editor::path_key;

fn content_hash(path: &Path) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    match std::fs::read(path) {
        Ok(bytes) => bytes.hash(&mut h),
        Err(_) => 0u8.hash(&mut h), // missing: hashes like an empty marker
    }
    h.finish()
}

impl App {
    /// Record a config file's content as applied.
    pub(crate) fn remember_config_file(&mut self, path: &Path) {
        self.config_hashes.insert(path_key(path), content_hash(path));
    }

    /// True if `path` differs from what was last applied.
    fn config_file_changed(&mut self, path: &Path) -> bool {
        let key = path_key(path);
        let now = content_hash(path);
        if self.config_hashes.get(&key) == Some(&now) {
            return false;
        }
        self.config_hashes.insert(key, now);
        true
    }

    /// Directories watched for configuration changes (only those that
    /// exist; a directory created later is picked up on a later sync).
    pub(crate) fn config_watch_dirs(&self) -> Vec<PathBuf> {
        let mut dirs = vec![self.paths.config_dir.clone(), self.paths.themes_dir()];
        for r in self.workspace.roots() {
            dirs.push(r.path.join(".vscode"));
        }
        dirs.retain(|d| d.is_dir());
        dirs
    }

    /// Apply configuration files among `paths` that changed. Returns true
    /// if any of them was a configuration file.
    pub(crate) fn on_config_files_changed(&mut self, paths: &[PathBuf]) -> bool {
        let user = path_key(&self.paths.user_settings());
        let keys = path_key(&self.paths.keybindings());
        let themes = path_key(&self.paths.themes_dir());
        let root_settings: Vec<PathBuf> =
            self.workspace.roots().iter().map(|r| path_key(&r.path.join(".vscode").join("settings.json"))).collect();
        let (mut settings, mut keymap, mut theme, mut any) = (false, false, false, false);
        for p in paths {
            let k = path_key(p);
            let is_theme = k.parent() == Some(themes.as_path()) && k.extension().is_some_and(|e| e == "toml");
            if k == user {
                any = true;
                if self.config_file_changed(p) {
                    settings = true;
                    keymap = true; // the chord timeout lives in settings
                }
            } else if k == keys {
                any = true;
                keymap |= self.config_file_changed(p);
            } else if is_theme {
                any = true;
                theme |= self.config_file_changed(p);
            } else if root_settings.contains(&k) {
                any = true;
                settings |= self.config_file_changed(p);
            }
        }
        if theme {
            self.invalidate_theme();
        }
        if settings || theme {
            self.reload_settings();
            if settings && self.config_errors.is_empty() {
                self.notify(Severity::Info, "Settings reloaded");
            }
        }
        if keymap {
            self.reload_keymap();
            if !settings {
                self.notify(Severity::Info, "Keybindings reloaded");
            }
        }
        any
    }
}
