//! Line diffs between a file's base (index or HEAD) and the editor's
//! current text, for the gutter (spec §15.1), and unified diff text for
//! previews.

use std::ops::Range;

use imara_diff::{Algorithm, UnifiedDiffBuilder, diff, intern::InternedInput};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HunkKind {
    Added,
    Modified,
    /// Lines removed before `lines.start` (an empty range).
    Deleted,
}

/// A changed region in the current text (line numbers, 0-based).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub kind: HunkKind,
    pub lines: Range<u32>,
    /// The base's lines this replaces.
    pub old: Range<u32>,
}

/// Hunks turning `base` into `current` (Histogram algorithm).
pub fn line_hunks(base: &str, current: &str) -> Vec<Hunk> {
    let input = InternedInput::new(base, current);
    let mut out = Vec::new();
    diff(Algorithm::Histogram, &input, |before: Range<u32>, after: Range<u32>| {
        let kind = if before.is_empty() {
            HunkKind::Added
        } else if after.is_empty() {
            HunkKind::Deleted
        } else {
            HunkKind::Modified
        };
        out.push(Hunk { kind, lines: after, old: before });
    });
    out
}

/// The base's text for a hunk (for "Show Change").
pub fn old_text(base: &str, h: &Hunk) -> String {
    base.split_inclusive('\n').skip(h.old.start as usize).take(h.old.len()).collect()
}

/// A unified diff (`--- a/label`, `+++ b/label`, `@@` hunks with three
/// lines of context) turning `old` into `new`; empty when they are equal.
pub fn unified(old: &str, new: &str, label: &str) -> String {
    let input = InternedInput::new(old, new);
    let body = diff(Algorithm::Histogram, &input, UnifiedDiffBuilder::new(&input));
    if body.is_empty() {
        return body;
    }
    format!("--- a/{label}\n+++ b/{label}\n{body}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hunks() {
        let base = "a\nb\nc\nd\ne\n";
        assert!(line_hunks(base, base).is_empty());
        let h = line_hunks(base, "x\na\nb\nC\nd\n");
        assert_eq!(
            h,
            vec![
                Hunk { kind: HunkKind::Added, lines: 0..1, old: 0..0 },
                Hunk { kind: HunkKind::Modified, lines: 3..4, old: 2..3 },
                Hunk { kind: HunkKind::Deleted, lines: 5..5, old: 4..5 },
            ]
        );
        assert_eq!(old_text(base, &h[1]), "c\n");
        assert_eq!(line_hunks("", "a\n")[0].kind, HunkKind::Added);
        assert_eq!(line_hunks("a\n", "")[0].kind, HunkKind::Deleted);
    }

    #[test]
    fn unified_text() {
        assert_eq!(unified("a\n", "a\n", "f"), "");
        let old: String = (1..=20).map(|i| format!("l{i}\n")).collect();
        let new = old.replace("l10\n", "L10\n");
        assert_eq!(
            unified(&old, &new, "src/f.txt"),
            "--- a/src/f.txt\n+++ b/src/f.txt\n@@ -7,7 +7,7 @@\n l7\n l8\n l9\n-l10\n+L10\n l11\n l12\n l13\n"
        );
    }
}
