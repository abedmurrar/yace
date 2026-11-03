//! `file://` URIs (RFC 3986 percent-encoding of the path).

use std::path::{Path, PathBuf};

fn unreserved(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/')
}

pub fn from_path(p: &Path) -> String {
    let mut s = String::from("file://");
    for &b in p.as_os_str().as_encoded_bytes() {
        if unreserved(b) {
            s.push(b as char);
        } else {
            s.push_str(&format!("%{b:02X}"));
        }
    }
    s
}

pub fn to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    // Drop an authority (`file://localhost/…`).
    let rest = if rest.starts_with('/') { rest } else { &rest[rest.find('/')?..] };
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(h) = rest.get(i + 1..i + 3)
            && let Ok(v) = u8::from_str_radix(h, 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Some(PathBuf::from(std::ffi::OsString::from_vec(out)))
    }
    #[cfg(not(unix))]
    {
        Some(PathBuf::from(String::from_utf8_lossy(&out).into_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let p = Path::new("/home/a b/ü/x#1.ts");
        let u = from_path(p);
        assert_eq!(u, "file:///home/a%20b/%C3%BC/x%231.ts");
        assert_eq!(to_path(&u).unwrap(), p);
        assert_eq!(to_path("file://localhost/tmp/x").unwrap(), Path::new("/tmp/x"));
        assert!(to_path("untitled:1").is_none());
    }
}
