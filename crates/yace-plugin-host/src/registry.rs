//! The plugin index: one JSON file on the YACE website that lists plugins
//! published from other repositories, with a download link and SHA-256 per
//! package. YACE reads it only when the user asks (Extensions: Browse
//! Plugins…, `yace --install-plugin <id>`), never in the background.
//!
//! Downloads use the `curl` program rather than a built-in HTTPS library:
//! one adds about 2 MB to the binary. HTTPS only (no redirect to plain
//! HTTP), and every package is checked against the index's SHA-256 and its
//! manifest against the index's id and version before the usual install
//! confirmation.

use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use serde::Deserialize;

use crate::{InstallPlan, store};

/// Where the index lives.
pub const DEFAULT_INDEX: &str = "https://abedmurrar.github.io/yace/plugins/index.json";

/// The index format this YACE reads.
pub const FORMAT: u32 = 1;

const MAX_INDEX_BYTES: u64 = 4 << 20;
const MAX_DOWNLOAD_BYTES: u64 = 256 << 20;

/// The index URL: `YACE_PLUGIN_INDEX` overrides it, so plugin authors can
/// try an entry (a `file://` URL works) before publishing it.
pub fn index_url() -> String {
    std::env::var("YACE_PLUGIN_INDEX").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| DEFAULT_INDEX.into())
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Index {
    pub format: u32,
    pub plugins: Vec<Entry>,
}

/// One plugin, as listed in the index.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub publisher: String,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub repository: String,
    /// Required YACE version, as in the manifest (`">=0.11"`).
    #[serde(default = "any_version")]
    pub yace: String,
    /// Whether the plugin runs native code (shown before downloading).
    #[serde(default)]
    pub code: bool,
    pub packages: Vec<Package>,
    /// Read from a `file://` index: its packages may be `file://` too.
    #[serde(skip)]
    pub local: bool,
}

fn any_version() -> String {
    "*".into()
}

/// A downloadable package: for one platform (`linux-x86_64`), or `any`
/// for data-only plugins.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Package {
    pub target: String,
    pub url: String,
    pub sha256: String,
}

/// This machine's package target, e.g. `linux-x86_64`.
pub fn target() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

impl Entry {
    /// The package for this machine: its own target first, then `any`.
    pub fn package(&self) -> Option<&Package> {
        let t = target();
        self.packages.iter().find(|p| p.target == t).or_else(|| self.packages.iter().find(|p| p.target == "any"))
    }

    /// Why this YACE can't install it, if it can't.
    pub fn compatible(&self, yace_version: &str) -> Result<(), String> {
        if self.package().is_none() {
            return Err(format!("no package for {}", target()));
        }
        let req = semver::VersionReq::parse(&self.yace).map_err(|e| e.to_string())?;
        let ours = semver::Version::parse(yace_version).map_err(|e| e.to_string())?;
        if !req.matches(&semver::Version::new(ours.major, ours.minor, ours.patch)) {
            return Err(format!("needs YACE {} (this is {yace_version})", self.yace));
        }
        Ok(())
    }
}

/// Parse and validate an index. Errors name the entry at fault.
pub fn parse(text: &str) -> Result<Index, String> {
    let index: Index = serde_json::from_str(text).map_err(|e| format!("the plugin index is invalid: {e}"))?;
    if index.format != FORMAT {
        return Err(format!(
            "the plugin index has format {} (this YACE reads format {FORMAT}); update YACE",
            index.format
        ));
    }
    let mut seen = std::collections::HashSet::new();
    for e in &index.plugins {
        let bad = |why: String| Err(format!("plugin index entry `{}`: {why}", e.id));
        if !crate::manifest::valid_id(&e.id) {
            return bad("invalid id".into());
        }
        if !seen.insert(e.id.as_str()) {
            return bad("listed twice".into());
        }
        if semver::Version::parse(&e.version).is_err() {
            return bad(format!("invalid version `{}`", e.version));
        }
        if semver::VersionReq::parse(&e.yace).is_err() {
            return bad(format!("invalid YACE requirement `{}`", e.yace));
        }
        if e.packages.is_empty() {
            return bad("no packages".into());
        }
        for p in &e.packages {
            if !(p.url.starts_with("https://") || p.url.starts_with("file://")) {
                // file:// is fetched only from a file:// index (see `fetch`).
                return bad(format!("package URL must use https: {}", p.url));
            }
            if p.sha256.len() != 64 || !p.sha256.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()) {
                return bad("sha256 must be 64 lowercase hex digits".into());
            }
        }
    }
    Ok(index)
}

/// Download `url` to `dest` with curl. `file://` is allowed only when
/// `local` (an index read from a file, for testing an entry locally).
pub fn fetch(url: &str, dest: &Path, max_bytes: u64, local: bool) -> Result<(), String> {
    let protos = if local { "=https,file" } else { "=https" };
    let out = Command::new("curl")
        .args(["--fail", "--silent", "--show-error", "--location", "--tlsv1.2"])
        .args(["--proto", protos, "--proto-redir", "=https"])
        .args(["--connect-timeout", "15", "--max-time", "300"])
        .arg("--max-filesize")
        .arg(max_bytes.to_string())
        .arg("--output")
        .arg(dest)
        .arg("--")
        .arg(url)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| {
            if e.kind() == ErrorKind::NotFound {
                "YACE downloads plugins with the curl program, which isn't installed (sudo apt install curl)".into()
            } else {
                format!("could not run curl: {e}")
            }
        })?;
    if out.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    let err = err.trim().trim_start_matches("curl: ").lines().next().unwrap_or("").to_string();
    Err(format!("could not download {url}: {}", if err.is_empty() { out.status.to_string() } else { err }))
}

/// Download and parse the index at `url` (normally [`index_url`]).
pub fn load_index(url: &str) -> Result<Index, String> {
    let local = url.starts_with("file://");
    let tmp = tempfile::Builder::new().prefix(".yace-index-").tempdir().map_err(|e| e.to_string())?;
    let file = tmp.path().join("index.json");
    fetch(url, &file, MAX_INDEX_BYTES, local)?;
    let mut index = parse(&std::fs::read_to_string(&file).map_err(|e| e.to_string())?)?;
    for e in &mut index.plugins {
        e.local = local;
    }
    Ok(index)
}

/// Download an entry's package for this machine, check its SHA-256 and that
/// its manifest is the plugin the index promised, then plan the install
/// exactly as for a local package. Nothing is installed.
pub fn plan_download(entry: &Entry, plugins_dir: &Path, yace_version: &str) -> Result<InstallPlan, String> {
    entry.compatible(yace_version)?;
    let pkg = entry.package().ok_or_else(|| format!("no package for {}", target()))?;
    let tmp = tempfile::Builder::new().prefix(".yace-download-").tempdir().map_err(|e| e.to_string())?;
    let file: PathBuf = tmp.path().join("plugin.tar.gz");
    fetch(&pkg.url, &file, MAX_DOWNLOAD_BYTES, entry.local)?;
    let sha = store::sha256_file(&file).map_err(|e| e.to_string())?;
    if sha != pkg.sha256 {
        return Err(format!(
            "the download of {} doesn't match the plugin index (sha256 {sha}, expected {}); not installed",
            entry.id, pkg.sha256
        ));
    }
    let mut plan = store::plan_install(&file, plugins_dir, yace_version)?;
    let m = &plan.manifest;
    if m.id != entry.id || m.version != entry.version {
        return Err(format!(
            "the package contains {} {}, but the plugin index lists {} {}; not installed",
            m.id, m.version, entry.id, entry.version
        ));
    }
    plan.source = pkg.url.clone();
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry_json(id: &str, url: &str, sha: &str) -> String {
        format!(
            r#"{{"id":"{id}","name":"Demo","version":"1.0.0","yace":">=0.1","packages":[{{"target":"any","url":"{url}","sha256":"{sha}"}}]}}"#
        )
    }

    #[test]
    fn parses_and_validates() {
        let sha = "a".repeat(64);
        let ok = format!(r#"{{"format":1,"plugins":[{}]}}"#, entry_json("dev.yace.demo", "https://x/y.tar.gz", &sha));
        let idx = parse(&ok).unwrap();
        assert_eq!(idx.plugins[0].package().unwrap().url, "https://x/y.tar.gz");
        assert!(idx.plugins[0].compatible("0.11.4").is_ok());
        assert!(idx.plugins[0].compatible("0.0.9").unwrap_err().contains("needs YACE"));

        let cases = [
            (r#"{"format":2,"plugins":[]}"#.to_string(), "format 2"),
            (format!(r#"{{"format":1,"plugins":[{}]}}"#, entry_json("Bad", "https://x", &sha)), "invalid id"),
            (format!(r#"{{"format":1,"plugins":[{}]}}"#, entry_json("dev.a", "http://x", &sha)), "https"),
            (format!(r#"{{"format":1,"plugins":[{}]}}"#, entry_json("dev.a", "https://x", "abc")), "sha256"),
            (
                format!(
                    r#"{{"format":1,"plugins":[{},{}]}}"#,
                    entry_json("dev.a", "https://x", &sha),
                    entry_json("dev.a", "https://x", &sha)
                ),
                "twice",
            ),
            (r#"{"format":1,"plugins":[],"extra":1}"#.to_string(), "unknown field"),
        ];
        for (text, want) in cases {
            let err = parse(&text).unwrap_err();
            assert!(err.contains(want), "{err}");
        }
    }

    /// The whole download path through curl, with `file://` URLs.
    #[test]
    fn downloads_checks_and_plans() {
        let work = tempfile::tempdir().unwrap();
        let src = work.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("yace-plugin.toml"), "id = \"dev.yace.demo\"\nname = \"Demo\"\nversion = \"1.2.0\"\n")
            .unwrap();
        let pkg = work.path().join("demo.tar.gz");
        let sha = store::package(&src, &pkg, "0.11.4").unwrap();
        let url = format!("file://{}", pkg.display());
        let write_index = |id: &str, version: &str, sha: &str| {
            let text = format!(
                r#"{{"format":1,"plugins":[{{"id":"{id}","name":"Demo","version":"{version}","packages":[{{"target":"any","url":"{url}","sha256":"{sha}"}}]}}]}}"#
            );
            let p = work.path().join("index.json");
            std::fs::write(&p, text).unwrap();
            format!("file://{}", p.display())
        };
        let plugins = work.path().join("plugins");

        let index = load_index(&write_index("dev.yace.demo", "1.2.0", &sha)).unwrap();
        let plan = plan_download(&index.plugins[0], &plugins, "0.11.4").unwrap();
        assert_eq!((plan.manifest.id.as_str(), plan.source.as_str()), ("dev.yace.demo", url.as_str()));

        let index = load_index(&write_index("dev.yace.demo", "1.2.0", &"0".repeat(64))).unwrap();
        assert!(plan_download(&index.plugins[0], &plugins, "0.11.4").unwrap_err().contains("doesn't match"));
        let index = load_index(&write_index("dev.yace.other", "1.2.0", &sha)).unwrap();
        assert!(plan_download(&index.plugins[0], &plugins, "0.11.4").unwrap_err().contains("lists dev.yace.other"));

        // An https index can't point at local files.
        let mut e = load_index(&write_index("dev.yace.demo", "1.2.0", &sha)).unwrap().plugins.remove(0);
        e.local = false;
        assert!(plan_download(&e, &plugins, "0.11.4").unwrap_err().contains("could not download"));
        assert!(load_index("file:///nonexistent/index.json").unwrap_err().contains("could not download"));
    }

    #[test]
    fn picks_this_machines_package_before_any() {
        let sha = "b".repeat(64);
        let text = format!(
            r#"{{"format":1,"plugins":[{{"id":"dev.a","name":"A","version":"1.0.0","packages":[
                {{"target":"any","url":"https://x/any","sha256":"{sha}"}},
                {{"target":"{}","url":"https://x/native","sha256":"{sha}"}}]}}]}}"#,
            target()
        );
        assert_eq!(parse(&text).unwrap().plugins[0].package().unwrap().url, "https://x/native");
        let text = text.replace(&target(), "plan9-mips");
        assert_eq!(parse(&text).unwrap().plugins[0].package().unwrap().url, "https://x/any");
    }
}
