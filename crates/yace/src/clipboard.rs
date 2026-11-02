//! Clipboard (TODO.md D10): system clipboard (lazy) → OSC 52 → internal.
//! The internal copy keeps multi-cursor pieces and line mode, which are
//! used when the system clipboard still holds the same text.

use yace_buffer::ops::ClipboardText;

pub struct Clipboard {
    internal: ClipboardText,
    system: Option<arboard::Clipboard>,
    tried_system: bool,
    /// Whether to also emit OSC 52 (works over SSH / without a display).
    pub use_osc52: bool,
    enabled: bool,
}

impl std::fmt::Debug for Clipboard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Clipboard").field("internal", &self.internal).finish()
    }
}

impl Clipboard {
    /// `enabled = false` keeps everything in-process (tests).
    pub fn new(enabled: bool) -> Self {
        Self { internal: ClipboardText::default(), system: None, tried_system: false, use_osc52: enabled, enabled }
    }

    fn system(&mut self) -> Option<&mut arboard::Clipboard> {
        if !self.enabled {
            return None;
        }
        if !self.tried_system {
            self.tried_system = true;
            let has_display = std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some();
            if has_display {
                self.system = arboard::Clipboard::new().ok();
            }
        }
        self.system.as_mut()
    }

    /// Store a copy. Returns the OSC 52 sequence to write, if any.
    pub fn set(&mut self, clip: ClipboardText) -> Option<Vec<u8>> {
        let text = clip.text.clone();
        self.internal = clip;
        let system_ok = self.system().is_some_and(|c| c.set_text(text.clone()).is_ok());
        (self.use_osc52 && (!system_ok || std::env::var_os("SSH_TTY").is_some())).then(|| osc52(&text))
    }

    pub fn set_text(&mut self, text: String) -> Option<Vec<u8>> {
        self.set(ClipboardText { text, pieces: None, line_mode: false })
    }

    pub fn get(&mut self) -> ClipboardText {
        if let Some(sys) = self.system().and_then(|c| c.get_text().ok())
            && sys != self.internal.text
        {
            return ClipboardText { text: sys, pieces: None, line_mode: false };
        }
        self.internal.clone()
    }
}

fn osc52(text: &str) -> Vec<u8> {
    let mut out = b"\x1b]52;c;".to_vec();
    out.extend(base64(text.as_bytes()).into_bytes());
    out.push(0x07);
    out
}

fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_encodes() {
        assert_eq!(base64(b"hello"), "aGVsbG8=");
        assert_eq!(base64(b"hi!"), "aGkh");
        assert_eq!(base64(b"a"), "YQ==");
    }

    #[test]
    fn internal_roundtrip_keeps_pieces() {
        let mut c = Clipboard::new(false);
        let clip = ClipboardText { text: "a\nb".into(), pieces: Some(vec!["a".into(), "b".into()]), line_mode: false };
        assert!(c.set(clip.clone()).is_none());
        assert_eq!(c.get(), clip);
    }
}
