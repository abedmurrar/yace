//! Which server serves which language. YACE never installs servers: it
//! starts what is on `PATH` (or configured), and explains how to install a
//! missing one. Node-based servers are external tools like any other.

use std::path::{Path, PathBuf};

use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
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
    }
}

/// The built-in table, in preference order per language.
pub fn builtin() -> Vec<ServerDef> {
    vec![
        def("rust-analyzer", "rust-analyzer", &[], &["rust"], "rustup component add rust-analyzer", false),
        def(
            "typescript-language-server",
            "typescript-language-server",
            &["--stdio"],
            &["typescript", "typescriptreact", "javascript", "javascriptreact"],
            "npm install -g typescript typescript-language-server",
            true,
        ),
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
    fn candidates_respect_user_overrides_and_disabled() {
        let mut mine = def("typescript-language-server", "/opt/tsls", &["--stdio"], &["typescript"], "", true);
        mine.settings = serde_json::json!({"typescript": {"format": {"semicolons": "remove"}}});
        let c = candidates("typescript", std::slice::from_ref(&mine), &[]);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].command, "/opt/tsls");
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
