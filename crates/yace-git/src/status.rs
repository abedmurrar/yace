//! Working-tree status from `git status --porcelain=v2 --branch -z`
//! (spec §15.1, §4.7).

use std::path::Path;

use crate::exec::{Git, GitError};

/// What a file's state means for decorations and the status view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Conflicted,
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
    Ignored,
}

impl Kind {
    /// One-letter badge (VS Code's).
    pub fn letter(self) -> &'static str {
        match self {
            Kind::Conflicted => "!",
            Kind::Modified => "M",
            Kind::Added => "A",
            Kind::Deleted => "D",
            Kind::Renamed => "R",
            Kind::Untracked => "U",
            Kind::Ignored => "I",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStatus {
    /// Relative to the repository's top level, `/`-separated. Ignored and
    /// untracked folders end with `/`.
    pub path: String,
    /// Index (staged) state: `b'.'` when unchanged, else M A D R C T U.
    pub x: u8,
    /// Working-tree state, same letters.
    pub y: u8,
    /// Original path of a rename or copy.
    pub orig: Option<String>,
    pub conflict: bool,
    pub untracked: bool,
    pub ignored: bool,
}

impl FileStatus {
    pub fn staged(&self) -> bool {
        !self.conflict && !self.untracked && !self.ignored && self.x != b'.'
    }
    /// Changed in the working tree (tracked files).
    pub fn unstaged(&self) -> bool {
        !self.conflict && !self.untracked && !self.ignored && self.y != b'.'
    }
    /// The state shown on the file (working tree first, like VS Code).
    pub fn kind(&self) -> Kind {
        if self.conflict {
            return Kind::Conflicted;
        }
        if self.untracked {
            return Kind::Untracked;
        }
        if self.ignored {
            return Kind::Ignored;
        }
        let letter = |c: u8| match c {
            b'D' => Some(Kind::Deleted),
            b'A' => Some(Kind::Added),
            b'R' | b'C' => Some(Kind::Renamed),
            b'M' | b'T' => Some(Kind::Modified),
            _ => None,
        };
        letter(self.y).or_else(|| letter(self.x)).unwrap_or(Kind::Modified)
    }
    /// Kind of the staged change.
    pub fn staged_kind(&self) -> Kind {
        match self.x {
            b'D' => Kind::Deleted,
            b'A' => Kind::Added,
            b'R' | b'C' => Kind::Renamed,
            _ => Kind::Modified,
        }
    }
    /// Kind of the unstaged change.
    pub fn unstaged_kind(&self) -> Kind {
        match self.y {
            b'D' => Kind::Deleted,
            b'A' => Kind::Added,
            _ => Kind::Modified,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Status {
    /// Branch name; `None` when detached.
    pub branch: Option<String>,
    /// Short commit id (`None` before the first commit).
    pub head: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub files: Vec<FileStatus>,
}

impl Status {
    pub fn conflicts(&self) -> usize {
        self.files.iter().filter(|f| f.conflict).count()
    }
    /// Any change other than ignored files.
    pub fn is_dirty(&self) -> bool {
        self.files.iter().any(|f| !f.ignored)
    }
    /// Text for the status bar: `main`, `main*`, `a1b2c3d` (detached), with
    /// `↑n ↓m`.
    pub fn label(&self) -> String {
        let mut s = match (&self.branch, &self.head) {
            (Some(b), _) => b.clone(),
            (None, Some(h)) => format!("({h})"),
            (None, None) => "(no commits)".into(),
        };
        if self.is_dirty() {
            s.push('*');
        }
        if self.ahead > 0 {
            s.push_str(&format!(" ↑{}", self.ahead));
        }
        if self.behind > 0 {
            s.push_str(&format!(" ↓{}", self.behind));
        }
        s
    }
}

/// `git status` arguments used for every refresh.
pub const STATUS_ARGS: &[&str] =
    &["status", "--porcelain=v2", "--branch", "-z", "--untracked-files=normal", "--ignored=matching"];

pub fn status(git: &Git, toplevel: &Path) -> Result<Status, GitError> {
    let out = git.read(toplevel, STATUS_ARGS)?;
    Ok(parse(&out))
}

fn field<'a>(it: &mut impl Iterator<Item = &'a [u8]>) -> &'a [u8] {
    it.next().unwrap_or(b"")
}

/// Parse porcelain v2 `-z` output. Unknown records are skipped, so newer
/// git versions never break it.
pub fn parse(out: &[u8]) -> Status {
    let mut st = Status::default();
    let text = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
    let mut records = out.split(|b| *b == 0).filter(|r| !r.is_empty());
    while let Some(rec) = records.next() {
        let (kind, rest) = match rec.split_first() {
            Some((k, r)) => (*k, r.strip_prefix(b" ").unwrap_or(r)),
            None => continue,
        };
        match kind {
            b'#' => {
                let mut parts = rest.splitn(2, |b| *b == b' ');
                let key = field(&mut parts);
                let val = field(&mut parts);
                match key {
                    b"branch.oid" if val != b"(initial)" => st.head = Some(text(&val[..val.len().min(7)])),
                    b"branch.head" if val != b"(detached)" => st.branch = Some(text(val)),
                    b"branch.upstream" => st.upstream = Some(text(val)),
                    b"branch.ab" => {
                        for n in val.split(|b| *b == b' ') {
                            let num =
                                |s: &[u8]| std::str::from_utf8(&s[1..]).ok().and_then(|x| x.parse().ok()).unwrap_or(0);
                            match n.first() {
                                Some(b'+') => st.ahead = num(n),
                                Some(b'-') => st.behind = num(n),
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
            }
            b'1' | b'2' | b'u' => {
                // 1 XY sub mH mI mW hH hI path
                // 2 XY sub mH mI mW hH hI Xscore path \0 orig
                // u XY sub m1 m2 m3 mW h1 h2 h3 path
                let skip = match kind {
                    b'1' => 7,
                    b'2' => 8,
                    _ => 9,
                };
                let mut parts = rest.splitn(skip + 1, |b| *b == b' ');
                let xy = field(&mut parts);
                for _ in 1..skip {
                    parts.next();
                }
                let path = field(&mut parts);
                let orig = (kind == b'2').then(|| records.next().map(text)).flatten();
                st.files.push(FileStatus {
                    path: text(path),
                    x: xy.first().copied().unwrap_or(b'.'),
                    y: xy.get(1).copied().unwrap_or(b'.'),
                    orig,
                    conflict: kind == b'u',
                    untracked: false,
                    ignored: false,
                });
            }
            b'?' | b'!' => st.files.push(FileStatus {
                path: text(rest),
                x: b'.',
                y: b'.',
                orig: None,
                conflict: false,
                untracked: kind == b'?',
                ignored: kind == b'!',
            }),
            _ => {}
        }
    }
    st
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_record_kind() {
        let out =
            b"# branch.oid 1234567890abcdef\0# branch.head main\0# branch.upstream origin/main\0# branch.ab +2 -1\0\
1 .M N... 100644 100644 100644 aaa aaa src/a b.rs\0\
1 A. N... 000000 100644 100644 000 bbb new.rs\0\
2 R. N... 100644 100644 100644 ccc ccc R100 renamed.rs\0old name.rs\0\
u UU N... 100644 100644 100644 100644 d1 d2 d3 conflict.rs\0\
? untracked dir/\0\
! node_modules/\0";
        let st = parse(out);
        assert_eq!(st.branch.as_deref(), Some("main"));
        assert_eq!(st.head.as_deref(), Some("1234567"));
        assert_eq!(st.upstream.as_deref(), Some("origin/main"));
        assert_eq!((st.ahead, st.behind), (2, 1));
        let kinds: Vec<(&str, Kind)> = st.files.iter().map(|f| (f.path.as_str(), f.kind())).collect();
        assert_eq!(
            kinds,
            [
                ("src/a b.rs", Kind::Modified),
                ("new.rs", Kind::Added),
                ("renamed.rs", Kind::Renamed),
                ("conflict.rs", Kind::Conflicted),
                ("untracked dir/", Kind::Untracked),
                ("node_modules/", Kind::Ignored),
            ]
        );
        assert_eq!(st.files[2].orig.as_deref(), Some("old name.rs"));
        assert!(st.files[0].unstaged() && !st.files[0].staged());
        assert!(st.files[1].staged() && !st.files[1].unstaged());
        assert_eq!(st.conflicts(), 1);
        assert_eq!(st.label(), "main* ↑2 ↓1");
        let detached = parse(b"# branch.oid abcdef1234\0# branch.head (detached)\0");
        assert_eq!(detached.label(), "(abcdef1)");
        assert_eq!(parse(b"# branch.oid (initial)\0# branch.head main\0").label(), "main");
        // Garbage never panics.
        let _ = parse(b"1\0u\0\x32 RR\0# \0zzz\0\0\0");
    }
}
