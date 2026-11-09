//! Which server serves which language. YACE never installs servers: it
//! starts what is on `PATH` (or configured), and explains how to install a
//! missing one. Node-based servers are external tools like any other.

use std::path::{Path, PathBuf};

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ServerDef {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    /// Language ids (VS Code ids, as used by `yace-syntax`).
    pub languages: Vec<String>,
    pub env: Vec<(String, String)>,
    pub init_options: Value,
    /// Answers `workspace/configuration` (looked up by section).
    pub settings: Value,
    pub install_hint: String,
    /// Runs on Node.js (reported as an external runtime).
    pub needs_node: bool,
    /// How the program is found.
    pub kind: DefKind,
    /// The plugin that declares it (`None`: built in or from settings).
    /// Plugin servers run alongside a language's main server.
    pub origin: Option<String>,
    /// Start only for files with one of these names in their folder or a
    /// folder above it, up to the workspace folder (`biome.json`).
    pub root_files: Vec<String>,
}

/// How a server's program is located.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DefKind {
    /// `command` on `PATH` (or as given).
    #[default]
    Plain,
    /// TypeScript 7's own server (`tsc --lsp --stdio`), from the project's
    /// TypeScript when it is ≥ 7, else a global TypeScript ≥ 7. The native
    /// binary is started directly (no Node.js process in between).
    TypeScriptNative,
    /// `typescript-language-server`: needs `tsserver.js`, which TypeScript 7
    /// no longer ships, so it is skipped for projects on TypeScript ≥ 7.
    TypeScriptTsserver,
    /// The project's own copy first (`node_modules/.bin/<command>` in the
    /// workspace folder or a folder above it), then `PATH`.
    ProjectBin,
}

fn def(name: &str, command: &str, args: &[&str], languages: &[&str], hint: &str, node: bool) -> ServerDef {
    ServerDef {
        name: name.into(),
        command: command.into(),
        args: args.iter().map(|s| s.to_string()).collect(),
        languages: languages.iter().map(|s| s.to_string()).collect(),
        env: Vec::new(),
        init_options: Value::Null,
        settings: Value::Null,
        install_hint: hint.into(),
        needs_node: node,
        ..Default::default()
    }
}

/// The built-in table, in preference order per language.
pub fn builtin() -> Vec<ServerDef> {
    vec![
        def("rust-analyzer", "rust-analyzer", &[], &["rust"], "rustup component add rust-analyzer", false),
        ServerDef {
            kind: DefKind::TypeScriptNative,
            ..def(
                "typescript",
                "tsc",
                &["--lsp", "--stdio"],
                &["typescript", "typescriptreact", "javascript", "javascriptreact"],
                "npm install -g typescript   (TypeScript 7 includes its own language server)",
                false,
            )
        },
        ServerDef {
            kind: DefKind::TypeScriptTsserver,
            ..def(
                "typescript-language-server",
                "typescript-language-server",
                &["--stdio"],
                &["typescript", "typescriptreact", "javascript", "javascriptreact"],
                "npm install -g typescript-language-server   (for projects on TypeScript 6 or older)",
                true,
            )
        },
        def("pyright", "pyright-langserver", &["--stdio"], &["python"], "npm install -g pyright", true),
        def("pylsp", "pylsp", &[], &["python"], "pip install python-lsp-server", false),
        def("gopls", "gopls", &[], &["go"], "go install golang.org/x/tools/gopls@latest", false),
        def("clangd", "clangd", &[], &["c", "cpp"], "install clangd (e.g. `sudo apt install clangd`)", false),
        def(
            "bash-language-server",
            "bash-language-server",
            &["start"],
            &["shellscript"],
            "npm install -g bash-language-server",
            true,
        ),
        def(
            "yaml-language-server",
            "yaml-language-server",
            &["--stdio"],
            &["yaml"],
            "npm install -g yaml-language-server",
            true,
        ),
        def(
            "vscode-json-language-server",
            "vscode-json-language-server",
            &["--stdio"],
            &["json", "jsonc"],
            "npm install -g vscode-langservers-extracted",
            true,
        ),
        def(
            "vscode-css-language-server",
            "vscode-css-language-server",
            &["--stdio"],
            &["css", "scss"],
            "npm install -g vscode-langservers-extracted",
            true,
        ),
        def(
            "vscode-html-language-server",
            "vscode-html-language-server",
            &["--stdio"],
            &["html"],
            "npm install -g vscode-langservers-extracted",
            true,
        ),
        def("taplo", "taplo", &["lsp", "stdio"], &["toml"], "cargo install taplo-cli --locked", false),
        def(
            "marksman",
            "marksman",
            &["server"],
            &["markdown"],
            "install marksman (github.com/artempyanykh/marksman)",
            false,
        ),
        def(
            "dockerfile-language-server",
            "docker-langserver",
            &["--stdio"],
            &["dockerfile"],
            "npm install -g dockerfile-language-server-nodejs",
            true,
        ),
    ]
}

/// Resolve a command like a shell would: paths as given, names via `PATH`.
pub fn which(command: &str) -> Option<PathBuf> {
    let is_exec = |p: &Path| {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            p.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        }
        #[cfg(not(unix))]
        {
            p.is_file()
        }
    };
    if command.contains('/') {
        let p = PathBuf::from(command);
        return is_exec(&p).then_some(p);
    }
    std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join(command)).find(|p| is_exec(p))
}

/// Major version and directory of a `typescript` package.
fn ts_package(dir: &Path) -> Option<(u32, PathBuf)> {
    let text = std::fs::read_to_string(dir.join("package.json")).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    let major = v.get("version")?.as_str()?.split('.').next()?.parse().ok()?;
    Some((major, dir.to_path_buf()))
}

/// The TypeScript a project uses: the nearest `node_modules/typescript`.
pub fn workspace_typescript(root: &Path) -> Option<(u32, PathBuf)> {
    root.ancestors().find_map(|d| ts_package(&d.join("node_modules").join("typescript")))
}

/// A global TypeScript (the package behind `tsc` on `PATH`).
fn global_typescript() -> Option<(u32, PathBuf)> {
    let tsc = std::fs::canonicalize(which("tsc")?).ok()?;
    ts_package(tsc.parent()?.parent()?)
}

/// TypeScript 7's native executable for a `typescript` package directory.
fn native_tsc(pkg: &Path) -> Option<PathBuf> {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        o => o,
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        a => a,
    };
    let platform = format!("typescript-{os}-{arch}");
    let exe = if cfg!(windows) { "tsc.exe" } else { "tsc" };
    let mut dirs = vec![pkg.join("node_modules")];
    dirs.extend(pkg.ancestors().skip(1).filter(|d| d.ends_with("node_modules")).map(Path::to_path_buf));
    dirs.into_iter().map(|d| d.join("@typescript").join(&platform).join("lib").join(exe)).find(|p| p.is_file())
}

fn ts7_program(pkg: &Path) -> Option<PathBuf> {
    native_tsc(pkg).or_else(|| Some(pkg.join("bin").join("tsc")).filter(|p| p.is_file()))
}

/// The program to start for `def` in workspace folder `root`, or why it
/// cannot be used there.
pub fn resolve(def: &ServerDef, root: &Path) -> Result<PathBuf, String> {
    match def.kind {
        DefKind::Plain => which(&def.command).ok_or_else(|| format!("{} not found on PATH", def.command)),
        DefKind::TypeScriptNative => match workspace_typescript(root) {
            Some((major, pkg)) if major >= 7 => ts7_program(&pkg).ok_or_else(|| "TypeScript 7 binary missing".into()),
            Some((major, _)) => Err(format!("the project uses TypeScript {major}")),
            None => match global_typescript() {
                Some((major, pkg)) if major >= 7 => {
                    ts7_program(&pkg).ok_or_else(|| "TypeScript 7 binary missing".into())
                }
                _ => Err("no TypeScript 7 found".into()),
            },
        },
        DefKind::TypeScriptTsserver => match workspace_typescript(root) {
            Some((major, _)) if major >= 7 => Err("TypeScript 7 has no tsserver.js".into()),
            _ => which(&def.command).ok_or_else(|| format!("{} not found on PATH", def.command)),
        },
        DefKind::ProjectBin => root
            .ancestors()
            .map(|d| d.join("node_modules").join(".bin").join(&def.command))
            .find(|p| p.is_file())
            .or_else(|| which(&def.command))
            .ok_or_else(|| format!("{} not found in node_modules/.bin or on PATH", def.command)),
    }
}

/// Whether a server with `root_files` applies to `file` in workspace folder
/// `root`: one of the files exists in the file's folder or a folder above
/// it, up to `root`. No `root_files`: always.
pub fn applies(def: &ServerDef, file: &Path, root: &Path) -> bool {
    def.root_files.is_empty()
        || file
            .ancestors()
            .skip(1)
            .take_while(|d| d.starts_with(root))
            .any(|d| def.root_files.iter().any(|f| d.join(f).exists()))
}

/// `${workspaceFolder}`, `${workspaceFolderUri}` and `${workspaceFolderName}`
/// in the strings of `v`, for a server started in `root`.
pub fn expand_placeholders(v: &Value, root: &Path, name: &str) -> Value {
    match v {
        Value::String(s) if s.contains("${") => Value::String(
            s.replace("${workspaceFolderUri}", &crate::uri::from_path(root))
                .replace("${workspaceFolderName}", name)
                .replace("${workspaceFolder}", &root.display().to_string()),
        ),
        Value::Array(a) => Value::Array(a.iter().map(|x| expand_placeholders(x, root, name)).collect()),
        Value::Object(o) => {
            Value::Object(o.iter().map(|(k, x)| (k.clone(), expand_placeholders(x, root, name))).collect())
        }
        other => other.clone(),
    }
}

/// Servers for a language: user definitions (which replace built-ins of
/// the same name) first, then built-ins; disabled names removed.
pub fn candidates(language: &str, user: &[ServerDef], disabled: &[String]) -> Vec<ServerDef> {
    let mut all: Vec<ServerDef> = user.to_vec();
    all.extend(builtin().into_iter().filter(|b| !user.iter().any(|u| u.name == b.name)));
    all.into_iter().filter(|d| d.languages.iter().any(|l| l == language) && !disabled.contains(&d.name)).collect()
}

/// Answer one `workspace/configuration` item from a settings object.
pub fn config_section(settings: &Value, section: Option<&str>) -> Value {
    let Some(section) = section.filter(|s| !s.is_empty()) else { return settings.clone() };
    let mut cur = settings;
    for part in section.split('.') {
        match cur.get(part) {
            Some(v) => cur = v,
            None => return Value::Null,
        }
    }
    cur.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typescript_resolution_follows_the_project_version() {
        let d = std::env::temp_dir().join(format!("yace-ts-{}", std::process::id()));
        let ts = d.join("proj/node_modules/typescript");
        let native = d.join("proj/node_modules/@typescript/typescript-linux-x64/lib");
        std::fs::create_dir_all(ts.join("bin")).unwrap();
        std::fs::create_dir_all(&native).unwrap();
        std::fs::write(ts.join("package.json"), r#"{"name":"typescript","version":"7.0.2"}"#).unwrap();
        std::fs::write(ts.join("bin/tsc"), "").unwrap();
        std::fs::write(native.join("tsc"), "").unwrap();
        let root = d.join("proj/packages/app");
        std::fs::create_dir_all(&root).unwrap();
        let defs = builtin();
        let native_def = defs.iter().find(|x| x.kind == DefKind::TypeScriptNative).unwrap();
        let tsls = defs.iter().find(|x| x.kind == DefKind::TypeScriptTsserver).unwrap();
        let p = resolve(native_def, &root).unwrap();
        if std::env::consts::OS == "linux" && std::env::consts::ARCH == "x86_64" {
            assert!(p.ends_with("@typescript/typescript-linux-x64/lib/tsc"), "native binary, no Node: {p:?}");
        }
        assert!(resolve(tsls, &root).unwrap_err().contains("tsserver"));
        std::fs::write(ts.join("package.json"), r#"{"name":"typescript","version":"5.9.3"}"#).unwrap();
        assert!(resolve(native_def, &root).unwrap_err().contains("TypeScript 5"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn candidates_respect_user_overrides_and_disabled() {
        let mut mine = def("typescript-language-server", "/opt/tsls", &["--stdio"], &["typescript"], "", true);
        mine.settings = serde_json::json!({"typescript": {"format": {"semicolons": "remove"}}});
        let c = candidates("typescript", std::slice::from_ref(&mine), &[]);
        assert_eq!(c.len(), 2, "user definition first, then the native TypeScript server");
        assert_eq!(c[0].command, "/opt/tsls");
        assert_eq!(c[1].kind, DefKind::TypeScriptNative);
        assert_eq!(
            candidates("python", &[], &[]).iter().map(|d| d.name.as_str()).collect::<Vec<_>>(),
            ["pyright", "pylsp"]
        );
        assert!(candidates("python", &[], &["pyright".into(), "pylsp".into()]).is_empty());
        assert_eq!(config_section(&mine.settings, Some("typescript.format"))["semicolons"], "remove");
        assert!(config_section(&mine.settings, Some("nope")).is_null());
    }

    #[test]
    fn which_finds_sh() {
        assert!(which("sh").is_some());
        assert!(which("definitely-not-a-command-yace").is_none());
        assert!(which("/bin/sh").is_some());
    }
}

#[cfg(test)]
mod plugin_server_tests {
    use super::*;

    #[test]
    fn project_bin_root_files_and_placeholders() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        let pkg = root.join("packages/web");
        std::fs::create_dir_all(pkg.join("src")).unwrap();
        std::fs::create_dir_all(root.join("node_modules/.bin")).unwrap();
        let bin = root.join("node_modules/.bin/demo-lsp");
        std::fs::write(&bin, "").unwrap();
        let def = ServerDef {
            name: "demo".into(),
            command: "demo-lsp".into(),
            kind: DefKind::ProjectBin,
            root_files: vec!["demo.json".into()],
            ..Default::default()
        };
        // Found in the node_modules/.bin of a folder above the workspace folder.
        assert_eq!(resolve(&def, &pkg).unwrap(), bin);
        let missing = ServerDef { command: "no-such-demo-lsp".into(), ..def.clone() };
        assert!(resolve(&missing, &pkg).unwrap_err().contains("node_modules/.bin"));

        let file = pkg.join("src/a.ts");
        assert!(!applies(&def, &file, &pkg), "no demo.json yet");
        std::fs::write(pkg.join("demo.json"), "{}").unwrap();
        assert!(applies(&def, &file, &pkg));
        // Above the workspace folder doesn't count.
        std::fs::remove_file(pkg.join("demo.json")).unwrap();
        std::fs::write(root.join("demo.json"), "{}").unwrap();
        assert!(!applies(&def, &file, &pkg));
        assert!(applies(&def, &file, &root));
        assert!(applies(&ServerDef::default(), &file, &pkg), "no root_files: always");

        let v = serde_json::json!({ "workspaceFolder": { "uri": "${workspaceFolderUri}", "name": "${workspaceFolderName}" },
                                    "cwd": "${workspaceFolder}/x", "n": 1, "list": ["${workspaceFolderName}"] });
        let out = expand_placeholders(&v, Path::new("/w/app"), "app");
        assert_eq!(
            out,
            serde_json::json!({ "workspaceFolder": { "uri": "file:///w/app", "name": "app" },
                                "cwd": "/w/app/x", "n": 1, "list": ["app"] })
        );
    }
}
