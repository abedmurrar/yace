//! Encoding keyboard, paste and mouse input for the shell (xterm
//! conventions, spec §11.3, §11.6).

use yace_keymap::{KeyCode, KeyPress, Mods};

use crate::session::Modes;

/// xterm modifier parameter: 1 + shift + 2·alt + 4·ctrl.
fn modifier_param(m: Mods) -> u8 {
    1 + u8::from(m.contains(Mods::SHIFT)) + 2 * u8::from(m.contains(Mods::ALT)) + 4 * u8::from(m.contains(Mods::CTRL))
}

/// Bytes for a key press, or `None` if the key produces nothing.
/// `ch` is the character the terminal reported (for printable keys).
pub fn encode_key(key: KeyPress, ch: Option<char>, modes: Modes) -> Option<Vec<u8>> {
    let m = key.mods;
    let ctrl = m.contains(Mods::CTRL);
    let alt = m.contains(Mods::ALT);
    let shift = m.contains(Mods::SHIFT);
    let with_alt = |mut v: Vec<u8>| {
        if alt {
            v.insert(0, 0x1b);
        }
        Some(v)
    };
    let csi_mod = |final_byte: char, app: bool| -> Vec<u8> {
        let p = modifier_param(m);
        if p > 1 {
            format!("\x1b[1;{p}{final_byte}").into_bytes()
        } else if app {
            format!("\x1bO{final_byte}").into_bytes()
        } else {
            format!("\x1b[{final_byte}").into_bytes()
        }
    };
    let tilde = |n: u8| -> Vec<u8> {
        let p = modifier_param(m);
        if p > 1 { format!("\x1b[{n};{p}~").into_bytes() } else { format!("\x1b[{n}~").into_bytes() }
    };
    match key.code {
        KeyCode::Char(c) => {
            if ctrl {
                let base = ch.unwrap_or(c).to_ascii_lowercase();
                let b = match base {
                    'a'..='z' => base as u8 - b'a' + 1,
                    '@' | ' ' | '2' => 0,
                    '[' | '3' => 0x1b,
                    '\\' | '4' => 0x1c,
                    ']' | '5' => 0x1d,
                    '^' | '6' => 0x1e,
                    '_' | '-' | '/' | '7' => 0x1f,
                    '8' | '?' => 0x7f,
                    _ => return None,
                };
                return with_alt(vec![b]);
            }
            let c = match ch {
                Some(c) => c,
                None if shift => c.to_ascii_uppercase(),
                None => c,
            };
            let mut buf = [0u8; 4];
            with_alt(c.encode_utf8(&mut buf).as_bytes().to_vec())
        }
        KeyCode::Space => with_alt(vec![if ctrl { 0 } else { b' ' }]),
        KeyCode::Enter => with_alt(vec![b'\r']),
        KeyCode::Tab if shift => Some(b"\x1b[Z".to_vec()),
        KeyCode::Tab => with_alt(vec![b'\t']),
        KeyCode::Backspace => with_alt(vec![if ctrl { 0x08 } else { 0x7f }]),
        KeyCode::Escape => with_alt(vec![0x1b]),
        KeyCode::Up => Some(csi_mod('A', modes.app_cursor)),
        KeyCode::Down => Some(csi_mod('B', modes.app_cursor)),
        KeyCode::Right => Some(csi_mod('C', modes.app_cursor)),
        KeyCode::Left => Some(csi_mod('D', modes.app_cursor)),
        KeyCode::Home => Some(csi_mod('H', modes.app_cursor)),
        KeyCode::End => Some(csi_mod('F', modes.app_cursor)),
        KeyCode::Insert => Some(tilde(2)),
        KeyCode::Delete => Some(tilde(3)),
        KeyCode::PageUp => Some(tilde(5)),
        KeyCode::PageDown => Some(tilde(6)),
        KeyCode::F(n @ 1..=4) => {
            let f = b"PQRS"[usize::from(n - 1)] as char;
            let p = modifier_param(m);
            Some(if p > 1 { format!("\x1b[1;{p}{f}").into_bytes() } else { format!("\x1bO{f}").into_bytes() })
        }
        KeyCode::F(n @ 5..=12) => {
            let code = [15u8, 17, 18, 19, 20, 21, 23, 24][usize::from(n - 5)];
            Some(tilde(code))
        }
        KeyCode::F(_) => None,
    }
}

/// Bytes for pasted text. Escape characters are removed so a paste can't
/// inject control sequences; bracketed paste wraps it when enabled.
pub fn encode_paste(text: &str, modes: Modes) -> Vec<u8> {
    let clean: String = text.chars().filter(|c| *c != '\x1b').collect();
    let clean = clean.replace("\r\n", "\r").replace('\n', "\r");
    if modes.bracketed_paste {
        let mut v = b"\x1b[200~".to_vec();
        v.extend_from_slice(clean.as_bytes());
        v.extend_from_slice(b"\x1b[201~");
        v
    } else {
        clean.into_bytes()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    WheelUp,
    WheelDown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseKind {
    Press,
    Release,
    Drag,
}

/// SGR (1006) mouse report for a 0-based cell position. Returns `None`
/// when the application did not request this kind of event.
pub fn encode_mouse(
    button: MouseButton,
    kind: MouseKind,
    row: usize,
    col: usize,
    mods: Mods,
    modes: Modes,
) -> Option<Vec<u8>> {
    if !modes.any_mouse() {
        return None;
    }
    if kind == MouseKind::Drag && !(modes.mouse_drag || modes.mouse_motion) {
        return None;
    }
    let mut b: u32 = match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
        MouseButton::WheelUp => 64,
        MouseButton::WheelDown => 65,
    };
    if kind == MouseKind::Drag {
        b += 32;
    }
    if mods.contains(Mods::SHIFT) {
        b += 4;
    }
    if mods.contains(Mods::ALT) {
        b += 8;
    }
    if mods.contains(Mods::CTRL) {
        b += 16;
    }
    let (x, y) = (col + 1, row + 1);
    if modes.sgr_mouse {
        let end = if kind == MouseKind::Release { 'm' } else { 'M' };
        return Some(format!("\x1b[<{b};{x};{y}{end}").into_bytes());
    }
    // Legacy X10 encoding (limited to 223 columns/rows).
    if x > 223 || y > 223 {
        return None;
    }
    let code = if kind == MouseKind::Release { 3 } else { b };
    Some(vec![0x1b, b'[', b'M', (32 + code) as u8, (32 + x) as u8, (32 + y) as u8])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(s: &str) -> KeyPress {
        s.parse().unwrap()
    }
    fn enc(s: &str) -> Vec<u8> {
        encode_key(k(s), None, Modes::default()).unwrap()
    }

    #[test]
    fn control_and_text_keys() {
        assert_eq!(enc("ctrl+c"), vec![3]);
        assert_eq!(enc("ctrl+d"), vec![4]);
        assert_eq!(enc("ctrl+["), vec![0x1b]);
        assert_eq!(enc("ctrl+space"), vec![0]);
        assert_eq!(enc("enter"), b"\r");
        assert_eq!(enc("backspace"), vec![0x7f]);
        assert_eq!(enc("shift+tab"), b"\x1b[Z");
        assert_eq!(enc("alt+b"), b"\x1bb");
        assert_eq!(encode_key(k("x"), Some('é'), Modes::default()).unwrap(), "é".as_bytes());
        assert_eq!(enc("escape"), vec![0x1b]);
    }

    #[test]
    fn cursor_keys_normal_app_and_modified() {
        assert_eq!(enc("up"), b"\x1b[A");
        let app = Modes { app_cursor: true, ..Default::default() };
        assert_eq!(encode_key(k("up"), None, app).unwrap(), b"\x1bOA");
        assert_eq!(enc("ctrl+right"), b"\x1b[1;5C");
        assert_eq!(enc("shift+up"), b"\x1b[1;2A");
        assert_eq!(enc("pageup"), b"\x1b[5~");
        assert_eq!(enc("ctrl+delete"), b"\x1b[3;5~");
        assert_eq!(enc("f1"), b"\x1bOP");
        assert_eq!(enc("f5"), b"\x1b[15~");
        assert_eq!(enc("f12"), b"\x1b[24~");
    }

    #[test]
    fn paste_is_sanitised_and_bracketed() {
        let plain = encode_paste("a\nb\x1b[31m", Modes::default());
        assert_eq!(plain, b"a\rb[31m");
        let br = encode_paste("x", Modes { bracketed_paste: true, ..Default::default() });
        assert_eq!(br, b"\x1b[200~x\x1b[201~");
    }

    #[test]
    fn mouse_reports() {
        assert!(encode_mouse(MouseButton::Left, MouseKind::Press, 0, 0, Mods::NONE, Modes::default()).is_none());
        let sgr = Modes { mouse_report: true, sgr_mouse: true, ..Default::default() };
        assert_eq!(encode_mouse(MouseButton::Left, MouseKind::Press, 4, 9, Mods::NONE, sgr).unwrap(), b"\x1b[<0;10;5M");
        assert_eq!(
            encode_mouse(MouseButton::Left, MouseKind::Release, 4, 9, Mods::NONE, sgr).unwrap(),
            b"\x1b[<0;10;5m"
        );
        assert_eq!(
            encode_mouse(MouseButton::WheelUp, MouseKind::Press, 0, 0, Mods::CTRL, sgr).unwrap(),
            b"\x1b[<80;1;1M"
        );
        assert!(encode_mouse(MouseButton::Left, MouseKind::Drag, 0, 0, Mods::NONE, sgr).is_none(), "no drag mode");
        let x10 = Modes { mouse_report: true, ..Default::default() };
        assert_eq!(
            encode_mouse(MouseButton::Left, MouseKind::Press, 0, 0, Mods::NONE, x10).unwrap(),
            vec![0x1b, b'[', b'M', 32, 33, 33]
        );
    }
}
