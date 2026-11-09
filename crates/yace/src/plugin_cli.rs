//! Plugin commands that run without the UI: `--install-plugin`,
//! `--list-plugins`, `--package-plugin`.

use std::{
    io::{BufRead, Write},
    path::Path,
    process::ExitCode,
};

use yace_config::Paths;
use yace_plugin_host::{InstallPlan, TrustRecord, manifest::CAPABILITIES, registry, store};

use crate::{app::App, cli::Args};

/// What the user approves when installing.
pub fn trust_summary(plan: &InstallPlan) -> Vec<String> {
    let m = &plan.manifest;
    let mut out = vec![
        format!("{} {} ({})", m.name, m.version, m.id),
        format!("Publisher: {}", if m.publisher.is_empty() { "(not stated)" } else { &m.publisher }),
        format!("From: {}", plan.source),
    ];
    if let Some(old) = &plan.replaces {
        out.push(format!("Replaces the installed version {old}"));
    }
    if !m.description.is_empty() {
        out.push(m.description.clone());
    }
    if m.library.is_some() {
        out.push(String::new());
        out.push("Native code: runs inside YACE with your user's permissions.".into());
        out.push("It is NOT sandboxed: it can read and change your files, start".into());
        out.push("programs and crash the editor. Install only code you trust.".into());
        for f in m.code_files() {
            if let Some(h) = plan.files.get(&f) {
                out.push(format!("  {f}  sha256:{}", &h[..16.min(h.len())]));
            }
        }
        if !m.capabilities.is_empty() {
            out.push("Declares it will:".into());
            for c in &m.capabilities {
                let d = CAPABILITIES.iter().find(|(k, _)| k == c).map(|(_, d)| *d).unwrap_or("");
                out.push(format!("  - {d} ({c})"));
            }
        }
    } else if m.lsp_servers.is_empty() {
        out.push("Data only (themes, grammars without code, settings): no code runs.".into());
    } else {
        out.push("No plugin code, but it starts the language servers below: programs".into());
        out.push("on your computer that run with your user's permissions.".into());
    }
    for s in &m.lsp_servers {
        let from = if s.local_bin { "the project's node_modules/.bin, else PATH" } else { "PATH" };
        out.push(format!("Starts a language server: {} (from {from})", s.command_line()));
        let mut when = format!("  for {}", s.languages.join(", "));
        if !s.root_files.is_empty() {
            when.push_str(&format!(", in folders with {}", s.root_files.join(" or ")));
        }
        out.push(when);
    }
    if !m.grammars.is_empty() {
        out.push(format!(
            "Grammars: {} (native parser libraries)",
            m.grammars.iter().map(|g| g.language.as_str()).collect::<Vec<_>>().join(", ")
        ));
    }
    if !m.external_programs.is_empty() {
        out.push(format!("Uses programs: {}", m.external_programs.join(", ")));
    }
    out
}

pub fn run(args: &Args, paths: &Paths) -> ExitCode {
    let plugins_dir = paths.config_dir.join("plugins");
    let version = App::yace_version();
    if let Some(dir) = &args.package_plugin {
        let id_version = std::fs::read_to_string(dir.join(yace_plugin_host::MANIFEST_FILE))
            .ok()
            .and_then(|t| yace_plugin_host::manifest::parse(&t, None).ok())
            .map(|m| format!("{}-{}", m.id, m.version))
            .unwrap_or_else(|| "plugin".into());
        let out = args.output.clone().unwrap_or_else(|| format!("{id_version}.tar.gz").into());
        return match store::package(dir, &out, version) {
            Ok(sha) => {
                println!("{}\nsha256:{sha}", out.display());
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("yace: {e}");
                ExitCode::FAILURE
            }
        };
    }
    if args.list_plugins {
        let trust = TrustRecord::load(&plugins_dir);
        let found = store::discover(&plugins_dir);
        if found.is_empty() {
            println!("No plugins installed in {}", plugins_dir.display());
        }
        for d in found {
            match d.manifest {
                Ok(m) => {
                    let state = if let Err(why) = m.compatible(version) {
                        format!("incompatible: {why}")
                    } else if m.library.is_some() && !trust.plugins.contains_key(&m.id) {
                        "not approved (install it through YACE)".into()
                    } else {
                        "installed".into()
                    };
                    println!("{}  {}  {}  [{state}]", m.id, m.version, m.name);
                }
                Err(e) => println!("{}  [invalid: {}]", d.folder, e.join("; ")),
            }
        }
        return ExitCode::SUCCESS;
    }
    let Some(src) = &args.install_plugin else { return ExitCode::SUCCESS };
    match install_interactive(src, &plugins_dir, version, args.yes) {
        Ok(msg) => {
            println!("{msg}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("yace: {e}");
            ExitCode::FAILURE
        }
    }
}

/// A plugin id that isn't a file or folder here: install it from the index.
fn index_id(src: &Path) -> Option<&str> {
    src.to_str().filter(|s| !src.exists() && yace_plugin_host::manifest::valid_id(s))
}

fn install_interactive(src: &Path, plugins_dir: &Path, version: &str, yes: bool) -> Result<String, String> {
    let plan = if let Some(id) = index_id(src) {
        let url = registry::index_url();
        eprintln!("Reading the plugin index ({url})…");
        let index = registry::load_index(&url)?;
        let entry = index
            .plugins
            .iter()
            .find(|e| e.id == id)
            .ok_or_else(|| format!("{id} is not a file, a folder or a plugin in the index ({url})"))?;
        eprintln!("Downloading {} {}…", entry.name, entry.version);
        registry::plan_download(entry, plugins_dir, version)?
    } else {
        store::plan_install(src, plugins_dir, version)?
    };
    for l in trust_summary(&plan) {
        println!("{l}");
    }
    if !yes {
        print!("\nInstall? [y/N] ");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line).map_err(|e| e.to_string())?;
        if !matches!(line.trim(), "y" | "Y" | "yes") {
            return Err("not installed".into());
        }
    }
    let dir = store::install(&plan, plugins_dir)?;
    Ok(format!("Installed {} {} to {}", plan.manifest.id, plan.manifest.version, dir.display()))
}
