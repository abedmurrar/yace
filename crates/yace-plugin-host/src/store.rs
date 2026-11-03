//! Installed plugins on disk (spec §18.5, §18.9, §18.13):
//! `<plugins dir>/<id>/yace-plugin.toml` plus files, and a trust record
//! (`installed.toml`) with the SHA-256 of every file at install time. Code
//! is loaded only if its hash still matches, so a plugin changed on disk
//! after it was approved is refused until it is installed again.

use std::{
    collections::BTreeMap,
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::manifest::{self, MANIFEST_FILE, Manifest};

pub const TRUST_FILE: &str = "installed.toml";

/// Largest package accepted (uncompressed), and largest single file.
const MAX_PACKAGE_BYTES: u64 = 512 << 20;

/// One plugin folder found on disk.
#[derive(Debug, Clone)]
pub struct Discovered {
    pub dir: PathBuf,
    /// The folder name (the id for installed plugins).
    pub folder: String,
    pub manifest: Result<Manifest, Vec<String>>,
}

impl Discovered {
    pub fn id(&self) -> &str {
        match &self.manifest {
            Ok(m) => &m.id,
            Err(_) => &self.folder,
        }
    }
}

/// Read every plugin manifest under `dir` (no code is loaded).
pub fn discover(dir: &Path) -> Vec<Discovered> {
    let Ok(rd) = fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<Discovered> = rd
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let path = e.path();
            let folder = e.file_name().to_string_lossy().into_owned();
            if folder.starts_with('.') {
                return None; // staging folders
            }
            let text = fs::read_to_string(path.join(MANIFEST_FILE));
            let manifest = match text {
                Ok(t) => manifest::parse(&t, Some(&path)).and_then(|m| {
                    if m.id == folder {
                        Ok(m)
                    } else {
                        Err(vec![format!("folder name `{folder}` differs from id `{}`", m.id)])
                    }
                }),
                Err(e) => Err(vec![format!("{MANIFEST_FILE}: {e}")]),
            };
            Some(Discovered { dir: path, folder, manifest })
        })
        .collect();
    out.sort_by(|a, b| a.folder.cmp(&b.folder));
    out
}

/// What was approved for one plugin.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TrustEntry {
    pub version: String,
    /// Where it was installed from.
    pub source: String,
    /// Seconds since the epoch.
    pub installed: u64,
    /// Relative path → SHA-256 (hex) of every file.
    pub files: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TrustRecord {
    #[serde(default)]
    pub plugins: BTreeMap<String, TrustEntry>,
}

impl TrustRecord {
    pub fn load(plugins_dir: &Path) -> Self {
        fs::read_to_string(plugins_dir.join(TRUST_FILE)).ok().and_then(|t| toml::from_str(&t).ok()).unwrap_or_default()
    }
    pub fn save(&self, plugins_dir: &Path) -> io::Result<()> {
        let text = format!(
            "# Plugins installed through YACE and the SHA-256 of their files when\n\
             # they were approved. Code whose hash differs is not loaded.\n\n{}",
            toml::to_string(self).map_err(io::Error::other)?
        );
        write_atomic(&plugins_dir.join(TRUST_FILE), text.as_bytes())
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    io::Write::write_all(&mut tmp, bytes)?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

pub fn sha256_file(path: &Path) -> io::Result<String> {
    let mut f = fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex(&h.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Every regular file under `dir`, as sorted relative paths (`/`
/// separated). Symlinks are refused: a plugin must be self-contained.
pub fn list_files(dir: &Path) -> Result<Vec<String>, String> {
    fn walk(base: &Path, d: &Path, out: &mut Vec<String>) -> Result<(), String> {
        let rd = fs::read_dir(d).map_err(|e| format!("{}: {e}", d.display()))?;
        for e in rd.flatten() {
            let p = e.path();
            let ft = e.file_type().map_err(|e| e.to_string())?;
            if ft.is_symlink() {
                return Err(format!("{} is a symbolic link; plugins must not contain links", p.display()));
            }
            if ft.is_dir() {
                walk(base, &p, out)?;
            } else if ft.is_file() {
                let rel = p.strip_prefix(base).map_err(|e| e.to_string())?;
                out.push(rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/"));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out)?;
    out.sort();
    Ok(out)
}

pub fn hash_files(dir: &Path) -> Result<BTreeMap<String, String>, String> {
    list_files(dir)?
        .into_iter()
        .map(|rel| sha256_file(&dir.join(&rel)).map(|h| (rel.clone(), h)).map_err(|e| format!("{rel}: {e}")))
        .collect()
}

/// Check that the plugin's code files still match the trust record.
pub fn verify_code(dir: &Path, m: &Manifest, trust: &TrustRecord) -> Result<(), String> {
    let Some(entry) = trust.plugins.get(&m.id) else {
        return Err("it was not installed through YACE (use Extensions: Install from Directory…)".into());
    };
    for rel in m.code_files() {
        let want = entry.files.get(&rel).ok_or_else(|| format!("{rel} was not part of the approved install"))?;
        let have = sha256_file(&dir.join(&rel)).map_err(|e| format!("{rel}: {e}"))?;
        if &have != want {
            return Err(format!(
                "{rel} changed on disk since it was installed; install the plugin again to approve it"
            ));
        }
    }
    Ok(())
}

/// What an install would do, for the confirmation prompt.
#[derive(Debug, Clone)]
pub struct InstallPlan {
    pub manifest: Manifest,
    /// Folder with the plugin's files (a temporary one for packages).
    pub staged: PathBuf,
    pub source: String,
    pub files: BTreeMap<String, String>,
    /// The installed version this replaces, if any.
    pub replaces: Option<String>,
    /// Keeps a package's extraction folder alive until the install.
    _tmp: Option<std::sync::Arc<tempfile::TempDir>>,
}

/// Prepare installing from a folder or a `.tar.gz` / `.yacepkg` package:
/// validate the manifest and every file, hash them. Nothing is changed.
pub fn plan_install(source: &Path, plugins_dir: &Path, yace_version: &str) -> Result<InstallPlan, String> {
    let (staged, tmp) = if source.is_dir() {
        (source.to_path_buf(), None)
    } else if source.is_file() {
        let tmp = tempfile::Builder::new().prefix(".yace-pkg-").tempdir().map_err(|e| e.to_string())?;
        unpack(source, tmp.path())?;
        // A package may wrap everything in one top-level folder.
        let root = if tmp.path().join(MANIFEST_FILE).is_file() {
            tmp.path().to_path_buf()
        } else {
            let dirs: Vec<PathBuf> = fs::read_dir(tmp.path())
                .map_err(|e| e.to_string())?
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect();
            match dirs.as_slice() {
                [one] if one.join(MANIFEST_FILE).is_file() => one.clone(),
                _ => return Err(format!("the package has no {MANIFEST_FILE}")),
            }
        };
        (root, Some(std::sync::Arc::new(tmp)))
    } else {
        return Err(format!("{} does not exist", source.display()));
    };
    let text = fs::read_to_string(staged.join(MANIFEST_FILE)).map_err(|e| format!("{MANIFEST_FILE}: {e}"))?;
    let manifest = manifest::parse(&text, Some(&staged)).map_err(|e| e.join("; "))?;
    manifest.compatible(yace_version)?;
    let files = hash_files(&staged)?;
    let replaces = TrustRecord::load(plugins_dir).plugins.get(&manifest.id).map(|e| e.version.clone());
    Ok(InstallPlan { manifest, staged, source: source.display().to_string(), files, replaces, _tmp: tmp })
}

fn copy_tree(from: &Path, to: &Path, files: &BTreeMap<String, String>) -> Result<(), String> {
    for rel in files.keys() {
        let dst = to.join(rel);
        if let Some(p) = dst.parent() {
            fs::create_dir_all(p).map_err(|e| format!("{}: {e}", p.display()))?;
        }
        fs::copy(from.join(rel), &dst).map_err(|e| format!("{rel}: {e}"))?;
    }
    Ok(())
}

/// Install a planned plugin: copy into a staging folder, verify the copy,
/// swap it in place of any older version, record the hashes.
pub fn install(plan: &InstallPlan, plugins_dir: &Path) -> Result<PathBuf, String> {
    fs::create_dir_all(plugins_dir).map_err(|e| format!("{}: {e}", plugins_dir.display()))?;
    let id = &plan.manifest.id;
    let staging = plugins_dir.join(format!(".installing-{id}"));
    let _ = fs::remove_dir_all(&staging);
    copy_tree(&plan.staged, &staging, &plan.files)?;
    let copied = hash_files(&staging)?;
    if copied != plan.files {
        let _ = fs::remove_dir_all(&staging);
        return Err("the files changed while installing; try again".into());
    }
    let target = plugins_dir.join(id);
    let old = plugins_dir.join(format!(".old-{id}"));
    let _ = fs::remove_dir_all(&old);
    if target.exists() {
        fs::rename(&target, &old).map_err(|e| format!("{}: {e}", target.display()))?;
    }
    if let Err(e) = fs::rename(&staging, &target) {
        let _ = fs::rename(&old, &target);
        return Err(format!("{}: {e}", target.display()));
    }
    let _ = fs::remove_dir_all(&old);
    let mut trust = TrustRecord::load(plugins_dir);
    trust.plugins.insert(
        id.clone(),
        TrustEntry {
            version: plan.manifest.version.clone(),
            source: plan.source.clone(),
            installed: SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
            files: plan.files.clone(),
        },
    );
    trust.save(plugins_dir).map_err(|e| e.to_string())?;
    Ok(target)
}

/// Approve a plugin folder that is already in the plugins folder (copied
/// there by hand): record its current hashes.
pub fn trust_in_place(dir: &Path, plugins_dir: &Path, yace_version: &str) -> Result<Manifest, String> {
    let text = fs::read_to_string(dir.join(MANIFEST_FILE)).map_err(|e| format!("{MANIFEST_FILE}: {e}"))?;
    let m = manifest::parse(&text, Some(dir)).map_err(|e| e.join("; "))?;
    m.compatible(yace_version)?;
    let files = hash_files(dir)?;
    let mut trust = TrustRecord::load(plugins_dir);
    trust.plugins.insert(
        m.id.clone(),
        TrustEntry {
            version: m.version.clone(),
            source: dir.display().to_string(),
            installed: SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
            files,
        },
    );
    trust.save(plugins_dir).map_err(|e| e.to_string())?;
    Ok(m)
}

/// Remove an installed plugin's folder and trust entry.
pub fn uninstall(plugins_dir: &Path, id: &str) -> Result<(), String> {
    if manifest::safe_relative(id).is_none() || id.contains('/') {
        return Err(format!("invalid plugin id `{id}`"));
    }
    let dir = plugins_dir.join(id);
    if dir.exists() {
        fs::remove_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let mut trust = TrustRecord::load(plugins_dir);
    if trust.plugins.remove(id).is_some() {
        trust.save(plugins_dir).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Build a package (`.tar.gz`) from a plugin folder after validating it.
/// Returns the package's SHA-256.
pub fn package(dir: &Path, out: &Path, yace_version: &str) -> Result<String, String> {
    let text = fs::read_to_string(dir.join(MANIFEST_FILE)).map_err(|e| format!("{MANIFEST_FILE}: {e}"))?;
    let m = manifest::parse(&text, Some(dir)).map_err(|e| e.join("; "))?;
    m.compatible(yace_version)?;
    let files = list_files(dir)?;
    let f = fs::File::create(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let gz = flate2::write::GzEncoder::new(f, flate2::Compression::best());
    let mut tar = tar::Builder::new(gz);
    tar.mode(tar::HeaderMode::Deterministic);
    for rel in &files {
        tar.append_path_with_name(dir.join(rel), rel).map_err(|e| format!("{rel}: {e}"))?;
    }
    tar.into_inner().and_then(|gz| gz.finish()).map_err(|e| e.to_string())?;
    sha256_file(out).map_err(|e| e.to_string())
}

/// Extract a package safely: regular files and folders only, no absolute
/// paths, no `..`, bounded size.
pub fn unpack(pkg: &Path, into: &Path) -> Result<(), String> {
    let f = fs::File::open(pkg).map_err(|e| format!("{}: {e}", pkg.display()))?;
    let mut ar = tar::Archive::new(flate2::read::GzDecoder::new(f));
    let mut total = 0u64;
    for entry in ar.entries().map_err(|e| format!("not a .tar.gz package: {e}"))? {
        let mut entry = entry.map_err(|e| format!("not a .tar.gz package: {e}"))?;
        let path = entry.path().map_err(|e| e.to_string())?.into_owned();
        let rel = path.to_string_lossy().to_string();
        let Some(safe) = manifest::safe_relative(&rel) else {
            return Err(format!("the package contains an unsafe path `{rel}`"));
        };
        let kind = entry.header().entry_type();
        total += entry.header().size().unwrap_or(0);
        if total > MAX_PACKAGE_BYTES {
            return Err("the package is too large".into());
        }
        let dst = into.join(&safe);
        if kind.is_dir() {
            fs::create_dir_all(&dst).map_err(|e| e.to_string())?;
        } else if kind.is_file() {
            if let Some(p) = dst.parent() {
                fs::create_dir_all(p).map_err(|e| e.to_string())?;
            }
            let mut out = fs::File::create(&dst).map_err(|e| e.to_string())?;
            io::copy(&mut (&mut entry).take(MAX_PACKAGE_BYTES), &mut out).map_err(|e| e.to_string())?;
        } else {
            return Err(format!("the package contains `{rel}`, which is not a regular file"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_plugin(dir: &Path, id: &str, version: &str) {
        fs::create_dir_all(dir.join("lib")).unwrap();
        fs::create_dir_all(dir.join("themes")).unwrap();
        fs::write(dir.join("lib/libx.so"), b"not really a library").unwrap();
        fs::write(dir.join("themes/t.toml"), "name = \"T\"\n").unwrap();
        fs::write(
            dir.join(MANIFEST_FILE),
            format!(
                "id = \"{id}\"\nname = \"X\"\nversion = \"{version}\"\nlibrary = \"lib/libx.so\"\n\
                 activation = [\"onStartupFinished\"]\n[[themes]]\npath = \"themes/t.toml\"\n"
            ),
        )
        .unwrap();
    }

    #[test]
    fn install_verify_update_uninstall() {
        let src = tempfile::tempdir().unwrap();
        let plugins = tempfile::tempdir().unwrap();
        make_plugin(src.path(), "dev.x.one", "1.0.0");
        let plan = plan_install(src.path(), plugins.path(), "0.6.0").unwrap();
        assert_eq!(plan.files.len(), 3);
        assert_eq!(plan.replaces, None);
        let dir = install(&plan, plugins.path()).unwrap();
        assert!(dir.join("lib/libx.so").is_file());
        let found = discover(plugins.path());
        assert_eq!(found.len(), 1);
        let m = found[0].manifest.clone().unwrap();
        let trust = TrustRecord::load(plugins.path());
        assert!(verify_code(&dir, &m, &trust).is_ok());
        // Tampering is detected.
        fs::write(dir.join("lib/libx.so"), b"evil").unwrap();
        assert!(verify_code(&dir, &m, &trust).unwrap_err().contains("changed on disk"));
        // A newer version replaces the old one.
        make_plugin(src.path(), "dev.x.one", "1.1.0");
        let plan = plan_install(src.path(), plugins.path(), "0.6.0").unwrap();
        assert_eq!(plan.replaces.as_deref(), Some("1.0.0"));
        install(&plan, plugins.path()).unwrap();
        let trust = TrustRecord::load(plugins.path());
        assert_eq!(trust.plugins["dev.x.one"].version, "1.1.0");
        assert!(verify_code(&dir, &discover(plugins.path())[0].manifest.clone().unwrap(), &trust).is_ok());
        // A copied-in folder without a trust entry is refused.
        let manual = plugins.path().join("dev.x.two");
        make_plugin(&manual, "dev.x.two", "1.0.0");
        let found = discover(plugins.path());
        let two = found.iter().find(|d| d.folder == "dev.x.two").unwrap();
        assert!(
            verify_code(&manual, two.manifest.as_ref().unwrap(), &trust)
                .unwrap_err()
                .contains("not installed through YACE")
        );
        uninstall(plugins.path(), "dev.x.one").unwrap();
        assert!(!dir.exists());
        assert!(!TrustRecord::load(plugins.path()).plugins.contains_key("dev.x.one"));
        assert!(uninstall(plugins.path(), "../etc").is_err());
    }

    #[test]
    fn packages_round_trip_and_bad_packages_are_refused() {
        let src = tempfile::tempdir().unwrap();
        make_plugin(src.path(), "dev.x.pkg", "2.0.0");
        let out = tempfile::tempdir().unwrap();
        let pkg = out.path().join("x.tar.gz");
        let sha = package(src.path(), &pkg, "0.6.0").unwrap();
        assert_eq!(sha.len(), 64);
        // Deterministic: packaging again gives the same bytes.
        let pkg2 = out.path().join("y.tar.gz");
        assert_eq!(package(src.path(), &pkg2, "0.6.0").unwrap(), sha);
        let plugins = tempfile::tempdir().unwrap();
        let plan = plan_install(&pkg, plugins.path(), "0.6.0").unwrap();
        assert_eq!(plan.manifest.id, "dev.x.pkg");
        install(&plan, plugins.path()).unwrap();
        assert!(plugins.path().join("dev.x.pkg/themes/t.toml").is_file());

        // Path traversal in a package.
        let evil = out.path().join("evil.tar.gz");
        {
            let f = fs::File::create(&evil).unwrap();
            let mut b = tar::Builder::new(flate2::write::GzEncoder::new(f, flate2::Compression::fast()));
            let mut h = tar::Header::new_gnu();
            let data = b"x";
            h.set_size(1);
            h.set_mode(0o644);
            h.set_entry_type(tar::EntryType::Regular);
            // `append_data` rejects "..", so write the raw name.
            h.as_old_mut().name[..9].copy_from_slice(b"../x.toml");
            h.set_cksum();
            b.append(&h, &data[..]).unwrap();
            b.into_inner().unwrap().finish().unwrap();
        }
        let err = plan_install(&evil, plugins.path(), "0.6.0").unwrap_err();
        assert!(err.contains("unsafe path"), "{err}");
        // Not a package at all.
        let junk = out.path().join("junk.tar.gz");
        fs::write(&junk, b"hello").unwrap();
        assert!(plan_install(&junk, plugins.path(), "0.6.0").unwrap_err().contains("package"));
        // Incompatible.
        assert!(plan_install(src.path(), plugins.path(), "0.0.1").is_ok(), "no yace requirement given");
    }

    #[test]
    fn discovery_reports_broken_manifests() {
        let plugins = tempfile::tempdir().unwrap();
        fs::create_dir_all(plugins.path().join("dev.x.bad")).unwrap();
        fs::write(plugins.path().join("dev.x.bad").join(MANIFEST_FILE), "id = 1").unwrap();
        fs::create_dir_all(plugins.path().join("other-name")).unwrap();
        make_plugin(&plugins.path().join("other-name"), "dev.x.real", "1.0.0");
        fs::create_dir_all(plugins.path().join(".installing-x")).unwrap();
        let found = discover(plugins.path());
        assert_eq!(found.len(), 2);
        assert!(found.iter().all(|d| d.manifest.is_err()));
        assert!(found[1].manifest.as_ref().unwrap_err()[0].contains("differs from id"));
    }
}
