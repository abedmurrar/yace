//! Loading and safe saving (spec §9.4, §9.9).
//!
//! - UTF-8 and UTF-8-with-BOM are supported; the BOM is preserved.
//! - Invalid UTF-8 or binary content is refused instead of opened lossily.
//! - Saving is atomic where possible: temp file in the target's real
//!   directory → fsync → rename → fsync directory. Permissions are kept,
//!   symlinks stay symlinks, hard links fall back to an in-place write.
//! - A disk snapshot taken at load/save detects external modification.

use std::{
    fs::{self, File, OpenOptions},
    hash::Hasher,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::SystemTime,
};

use ropey::Rope;
use thiserror::Error;

const BOM: &[u8] = b"\xEF\xBB\xBF";
const BINARY_SNIFF: usize = 8 * 1024;

/// What we last knew about the file on disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiskSnapshot {
    pub mtime: Option<SystemTime>,
    pub len: u64,
    pub hash: u64,
}

impl DiskSnapshot {
    fn of(meta: &fs::Metadata, bytes: &[u8]) -> Self {
        Self { mtime: meta.modified().ok(), len: meta.len(), hash: hash_bytes(bytes) }
    }
}

pub fn hash_bytes(bytes: &[u8]) -> u64 {
    // FNV-1a: stable, dependency-free, fast enough for change detection.
    let mut h = Fnv(0xcbf29ce484222325);
    h.write(bytes);
    h.finish()
}

struct Fnv(u64);
impl Hasher for Fnv {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 ^= u64::from(*b);
            self.0 = self.0.wrapping_mul(0x100000001b3);
        }
    }
}

#[derive(Debug, Error)]
pub enum LoadError {
    #[error("{path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("{path} looks like a binary file; YACE will not open it as text")]
    Binary { path: PathBuf },
    #[error("{path} is not valid UTF-8 (first invalid byte at offset {offset}); opening it could corrupt it")]
    InvalidUtf8 { path: PathBuf, offset: usize },
    #[error("{path} is a directory")]
    IsDirectory { path: PathBuf },
}

#[derive(Debug)]
pub struct Loaded {
    pub rope: Rope,
    pub has_bom: bool,
    pub snapshot: DiskSnapshot,
}

/// Size of a file without reading it (used for the large-file prompt).
pub fn file_size(path: &Path) -> io::Result<u64> {
    fs::metadata(path).map(|m| m.len())
}

pub fn load(path: &Path) -> Result<Loaded, LoadError> {
    let io_err = |source| LoadError::Io { path: path.to_path_buf(), source };
    let meta = fs::metadata(path).map_err(io_err)?;
    if meta.is_dir() {
        return Err(LoadError::IsDirectory { path: path.to_path_buf() });
    }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    File::open(path).and_then(|mut f| f.read_to_end(&mut bytes)).map_err(io_err)?;
    let snapshot = DiskSnapshot::of(&meta, &bytes);
    let (has_bom, body) = match bytes.strip_prefix(BOM) {
        Some(rest) => (true, rest),
        None => (false, &bytes[..]),
    };
    if body[..body.len().min(BINARY_SNIFF)].contains(&0) {
        return Err(LoadError::Binary { path: path.to_path_buf() });
    }
    let text = std::str::from_utf8(body).map_err(|e| LoadError::InvalidUtf8 {
        path: path.to_path_buf(),
        offset: e.valid_up_to() + usize::from(has_bom) * BOM.len(),
    })?;
    Ok(Loaded { rope: Rope::from_str(text), has_bom, snapshot })
}

/// Snapshot of the file as it is on disk now (`None` if it doesn't exist).
pub fn current_snapshot(path: &Path) -> io::Result<Option<DiskSnapshot>> {
    match fs::metadata(path) {
        Ok(meta) => {
            let bytes = fs::read(path)?;
            Ok(Some(DiskSnapshot::of(&meta, &bytes)))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Cheap check: has the file changed relative to `known`? Compares
/// mtime+len first and only hashes when those are ambiguous.
pub fn changed_on_disk(path: &Path, known: Option<&DiskSnapshot>) -> io::Result<bool> {
    let now = match fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(known.is_some()),
        Err(e) => return Err(e),
    };
    let Some(known) = known else { return Ok(true) };
    if now.len() != known.len {
        return Ok(true);
    }
    if now.modified().ok() == known.mtime {
        return Ok(false);
    }
    Ok(hash_bytes(&fs::read(path)?) != known.hash)
}

#[derive(Debug, Error)]
pub enum SaveError {
    #[error("{path} was changed on disk since it was opened")]
    Conflict { path: PathBuf },
    #[error("could not save {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SaveOptions<'a> {
    /// What we believe is on disk; a mismatch yields `SaveError::Conflict`.
    pub expected: Option<&'a DiskSnapshot>,
    /// Skip the conflict check (user chose "Overwrite").
    pub force: bool,
    pub bom: bool,
}

/// Serialize a rope to bytes (BOM + UTF-8).
pub fn encode(rope: &Rope, bom: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(rope.len_bytes() + 3);
    if bom {
        out.extend_from_slice(BOM);
    }
    for chunk in rope.chunks() {
        out.extend_from_slice(chunk.as_bytes());
    }
    out
}

pub fn save(path: &Path, rope: &Rope, opts: SaveOptions) -> Result<DiskSnapshot, SaveError> {
    let io_err = |source| SaveError::Io { path: path.to_path_buf(), source };
    if !opts.force {
        let changed = changed_on_disk(path, opts.expected).map_err(io_err)?;
        // A brand new file (no expectation, does not exist) is not a conflict.
        let new_file = opts.expected.is_none() && !path.exists();
        if changed && !new_file {
            return Err(SaveError::Conflict { path: path.to_path_buf() });
        }
    }
    let bytes = encode(rope, opts.bom);
    write_safely(path, &bytes).map_err(io_err)?;
    let meta = fs::metadata(path).map_err(io_err)?;
    Ok(DiskSnapshot::of(&meta, &bytes))
}

/// Write `bytes` to `path` atomically when possible.
pub fn write_safely(path: &Path, bytes: &[u8]) -> io::Result<()> {
    // Resolve symlinks so we replace the target, not the link.
    let target = match fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => fs::canonicalize(path)?,
        _ => path.to_path_buf(),
    };
    let existing = fs::metadata(&target).ok();

    #[cfg(unix)]
    let hard_linked = {
        use std::os::unix::fs::MetadataExt;
        existing.as_ref().is_some_and(|m| m.nlink() > 1)
    };
    #[cfg(not(unix))]
    let hard_linked = false;

    if !hard_linked {
        match write_atomic(&target, bytes, existing.as_ref()) {
            Ok(()) => return Ok(()),
            // Directory not writable but the file may be: fall back.
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied && existing.is_some() => {}
            Err(e) => return Err(e),
        }
    }
    write_in_place(&target, bytes)
}

fn write_atomic(target: &Path, bytes: &[u8], existing: Option<&fs::Metadata>) -> io::Result<()> {
    let dir = target.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let mut tmp = tempfile::Builder::new().prefix(".yace-save-").tempfile_in(dir)?;
    tmp.write_all(bytes)?;
    if let Some(meta) = existing {
        fs::set_permissions(tmp.path(), meta.permissions())?;
    }
    tmp.as_file().sync_all()?;
    tmp.persist(target).map_err(|e| e.error)?;
    #[cfg(unix)]
    if let Ok(d) = File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

fn write_in_place(target: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut f = OpenOptions::new().write(true).create(true).truncate(true).open(target)?;
    f.write_all(bytes)?;
    f.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_bom_crlf_unicode() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("ünïcode file.txt");
        let raw = b"\xEF\xBB\xBFline 1\r\n\xE4\xB8\xAD\r\nmixed\n";
        fs::write(&p, raw).unwrap();
        let l = load(&p).unwrap();
        assert!(l.has_bom);
        assert_eq!(l.rope.to_string(), "line 1\r\n中\r\nmixed\n");
        let snap = save(&p, &l.rope, SaveOptions { expected: Some(&l.snapshot), force: false, bom: true }).unwrap();
        assert_eq!(fs::read(&p).unwrap(), raw, "bytes preserved exactly");
        assert_eq!(snap.hash, hash_bytes(raw));
    }

    #[test]
    fn refuses_binary_and_invalid_utf8() {
        let dir = tempfile::tempdir().unwrap();
        let b = dir.path().join("bin");
        fs::write(&b, b"abc\0def").unwrap();
        assert!(matches!(load(&b), Err(LoadError::Binary { .. })));
        let u = dir.path().join("latin1");
        fs::write(&u, b"caf\xe9").unwrap();
        assert!(matches!(load(&u), Err(LoadError::InvalidUtf8 { offset: 3, .. })));
    }

    #[test]
    fn detects_conflict_and_force_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.txt");
        fs::write(&p, "one").unwrap();
        let l = load(&p).unwrap();
        fs::write(&p, "external change").unwrap();
        let rope = Rope::from_str("mine");
        let err = save(&p, &rope, SaveOptions { expected: Some(&l.snapshot), ..Default::default() });
        assert!(matches!(err, Err(SaveError::Conflict { .. })));
        assert_eq!(fs::read_to_string(&p).unwrap(), "external change");
        save(&p, &rope, SaveOptions { expected: Some(&l.snapshot), force: true, bom: false }).unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "mine");
    }

    #[test]
    fn new_file_is_not_a_conflict() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("new.txt");
        save(&p, &Rope::from_str("x"), SaveOptions::default()).unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "x");
    }

    #[cfg(unix)]
    #[test]
    fn preserves_permissions_and_symlinks() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real.sh");
        fs::write(&real, "#!/bin/sh\n").unwrap();
        fs::set_permissions(&real, fs::Permissions::from_mode(0o755)).unwrap();
        let link = dir.path().join("link.sh");
        symlink(&real, &link).unwrap();
        let l = load(&link).unwrap();
        save(
            &link,
            &Rope::from_str("#!/bin/sh\necho hi\n"),
            SaveOptions { expected: Some(&l.snapshot), ..Default::default() },
        )
        .unwrap();
        assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink(), "link stays a link");
        assert_eq!(fs::read_to_string(&real).unwrap(), "#!/bin/sh\necho hi\n");
        assert_eq!(fs::metadata(&real).unwrap().permissions().mode() & 0o777, 0o755);
    }

    #[cfg(unix)]
    #[test]
    fn hard_links_are_written_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        fs::write(&a, "old").unwrap();
        fs::hard_link(&a, &b).unwrap();
        write_safely(&a, b"new").unwrap();
        assert_eq!(fs::read_to_string(&b).unwrap(), "new");
    }

    #[test]
    fn failed_save_reports_error() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("missing-dir").join("x.txt");
        let err = save(&p, &Rope::from_str("x"), SaveOptions::default());
        assert!(matches!(err, Err(SaveError::Io { .. })));
    }
}
