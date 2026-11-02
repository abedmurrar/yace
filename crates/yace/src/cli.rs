//! Command-line interface (spec §24).

use std::path::{Path, PathBuf};

use clap::Parser;

#[derive(Debug, Parser, Default, Clone)]
#[command(
    name = "yace",
    version,
    about = "YACE — Yet Another Code Editor. A fast, terminal-native, multi-root code editor.",
    after_help = "Examples:\n  yace                          reopen the most recent workspace\n  yace .                        open the current directory\n  yace ~/work.code-workspace    open a VS Code multi-root workspace\n  yace --goto src/main.rs:42:8  open a file at line 42, column 8\n  yace api --add-root ../web    open two unrelated folders together"
)]
pub struct Args {
    /// A folder, a file, or a `.code-workspace` file to open.
    pub path: Option<PathBuf>,

    /// Open this `.code-workspace` file.
    #[arg(long, value_name = "FILE")]
    pub workspace: Option<PathBuf>,

    /// Add another folder to the workspace (repeatable).
    #[arg(long = "add-root", value_name = "DIR")]
    pub add_root: Vec<PathBuf>,

    /// Open a file at a position: `path[:line[:column]]`.
    #[arg(long, value_name = "FILE[:LINE[:COL]]")]
    pub goto: Option<String>,

    /// Do not restore the previous session (tabs, layout) for this workspace.
    #[arg(long)]
    pub no_restore: bool,

    /// Print the time to the first rendered frame and exit (benchmarking).
    #[arg(long, hide = true)]
    pub startup_time: bool,
}

/// What to open, after validating paths.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// Reopen the most recent workspace (or an empty window).
    Recent,
    Folder(PathBuf),
    WorkspaceFile(PathBuf),
    /// A file: its folder (repository root if found) is opened too.
    File {
        path: PathBuf,
        line: Option<usize>,
        col: Option<usize>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Launch {
    pub target: Target,
    pub add_roots: Vec<PathBuf>,
    pub goto: Option<(PathBuf, Option<usize>, Option<usize>)>,
    pub restore: bool,
}

/// Parse `path:line:col` (line/col are 1-based). A path that itself contains
/// `:` works as long as the trailing parts are numbers.
pub fn parse_goto(s: &str) -> Result<(PathBuf, Option<usize>, Option<usize>), String> {
    let mut parts: Vec<&str> = s.rsplitn(3, ':').collect();
    parts.reverse();
    let num = |p: &str| p.parse::<usize>().ok().filter(|n| *n > 0);
    let (path, line, col) = match parts.as_slice() {
        [p, l, c] if num(l).is_some() && num(c).is_some() => (p.to_string(), num(l), num(c)),
        [p, l, c] if num(c).is_some() => (format!("{p}:{l}"), num(c), None),
        [p, l] if num(l).is_some() => (p.to_string(), num(l), None),
        _ => (s.to_string(), None, None),
    };
    if path.is_empty() {
        return Err(format!("invalid --goto value `{s}`: missing file path"));
    }
    // Reject explicit zero / non-numeric positions that look intentional.
    if let Some(last) = s.rsplit(':').next()
        && !last.is_empty()
        && last.chars().all(|c| c.is_ascii_digit())
        && last.parse::<usize>().ok() == Some(0)
    {
        return Err(format!("invalid --goto value `{s}`: lines and columns start at 1"));
    }
    Ok((PathBuf::from(path), line, col))
}

fn is_workspace_file(p: &Path) -> bool {
    p.extension().is_some_and(|e| e == "code-workspace")
}

/// Validate arguments and decide what to open.
pub fn plan(args: &Args) -> Result<Launch, String> {
    let goto = args.goto.as_deref().map(parse_goto).transpose()?;
    if let Some((p, _, _)) = &goto
        && !p.exists()
    {
        // Opening a not-yet-existing file is allowed (new file), but its
        // directory must exist.
        let parent = p.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
        if !parent.is_dir() {
            return Err(format!("{}: no such directory", parent.display()));
        }
    }
    let target = if let Some(ws) = &args.workspace {
        if !ws.is_file() {
            return Err(format!("{}: workspace file not found", ws.display()));
        }
        Target::WorkspaceFile(ws.clone())
    } else if let Some(p) = &args.path {
        if is_workspace_file(p) {
            if !p.is_file() {
                return Err(format!("{}: workspace file not found", p.display()));
            }
            Target::WorkspaceFile(p.clone())
        } else if p.is_dir() {
            Target::Folder(p.clone())
        } else if p.is_file() {
            Target::File { path: p.clone(), line: None, col: None }
        } else {
            // Maybe `file:line:col` passed positionally.
            match parse_goto(&p.to_string_lossy()) {
                Ok((fp, line, col)) if fp.is_file() && line.is_some() => Target::File { path: fp, line, col },
                _ => return Err(format!("{}: no such file or directory", p.display())),
            }
        }
    } else if let Some((p, l, c)) = &goto {
        Target::File { path: p.clone(), line: *l, col: *c }
    } else {
        Target::Recent
    };
    for r in &args.add_root {
        if !r.is_dir() {
            return Err(format!("{}: --add-root needs an existing directory", r.display()));
        }
    }
    Ok(Launch { target, add_roots: args.add_root.clone(), goto, restore: !args.no_restore })
}

/// The folder to open for a file: the nearest ancestor with a `.git`
/// entry, otherwise the file's own directory.
pub fn folder_for_file(file: &Path) -> PathBuf {
    let abs = yace_workspace::paths::absolutize(file);
    let dir = abs.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("/"));
    let mut cur = Some(dir.as_path());
    while let Some(d) = cur {
        if d.join(".git").exists() {
            return d.to_path_buf();
        }
        cur = d.parent();
    }
    dir
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn goto_parsing() {
        assert_eq!(parse_goto("a.rs").unwrap(), (PathBuf::from("a.rs"), None, None));
        assert_eq!(parse_goto("a.rs:42").unwrap(), (PathBuf::from("a.rs"), Some(42), None));
        assert_eq!(parse_goto("a.rs:42:8").unwrap(), (PathBuf::from("a.rs"), Some(42), Some(8)));
        assert_eq!(parse_goto("c:dir/a.rs:3").unwrap(), (PathBuf::from("c:dir/a.rs"), Some(3), None));
        assert!(parse_goto("a.rs:0").is_err());
        assert!(parse_goto(":5").is_err());
    }

    #[test]
    fn planning_validates_paths() {
        let dir = tempfile::tempdir().unwrap();
        let args = Args { path: Some(dir.path().join("nope")), ..Default::default() };
        assert!(plan(&args).is_err());
        let args = Args { path: Some(dir.path().to_path_buf()), ..Default::default() };
        assert_eq!(plan(&args).unwrap().target, Target::Folder(dir.path().to_path_buf()));
        let f = dir.path().join("x.txt");
        std::fs::write(&f, "1\n2\n").unwrap();
        let args = Args { path: Some(PathBuf::from(format!("{}:2", f.display()))), ..Default::default() };
        assert!(matches!(plan(&args).unwrap().target, Target::File { line: Some(2), .. }));
        let args = Args { add_root: vec![dir.path().join("missing")], ..Default::default() };
        assert!(plan(&args).is_err());
        assert_eq!(plan(&Args::default()).unwrap().target, Target::Recent);
    }
}
