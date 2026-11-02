//! Path helpers: `~` expansion, lexical normalisation, root containment.

use std::path::{Component, Path, PathBuf};

/// Expand a leading `~` or `~/` using `$HOME`.
pub fn expand_tilde(p: &str) -> PathBuf {
    if (p == "~" || p.starts_with("~/"))
        && let Some(home) = std::env::var_os("HOME")
    {
        let rest = p.trim_start_matches('~').trim_start_matches('/');
        return PathBuf::from(home).join(rest);
    }
    PathBuf::from(p)
}

/// Normalise `.` and `..` lexically (without touching the filesystem, so
/// missing roots and symlinks are kept as the user wrote them).
pub fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Resolve a configured folder path against `base` (the workspace file's
/// directory for relative paths, per VS Code semantics).
pub fn resolve(configured: &str, base: Option<&Path>) -> PathBuf {
    let p = expand_tilde(configured);
    let abs = if p.is_absolute() {
        p
    } else if let Some(b) = base {
        b.join(p)
    } else {
        std::env::current_dir().unwrap_or_default().join(p)
    };
    normalize(&abs)
}

/// Make an absolute path (current dir based) and normalise it.
pub fn absolutize(p: &Path) -> PathBuf {
    if p.is_absolute() { normalize(p) } else { normalize(&std::env::current_dir().unwrap_or_default().join(p)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_relative_against_base() {
        let base = Path::new("/ws/files");
        assert_eq!(resolve("../repo a", Some(base)), PathBuf::from("/ws/repo a"));
        assert_eq!(resolve("./x/./y", Some(base)), PathBuf::from("/ws/files/x/y"));
        assert_eq!(resolve("/abs/ünï", Some(base)), PathBuf::from("/abs/ünï"));
        assert!(resolve("~/code", None).is_absolute() || std::env::var_os("HOME").is_none());
    }
}
