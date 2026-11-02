//! Key presses and sequences, with VS Code-style text syntax:
//! `ctrl+shift+p`, `ctrl+k ctrl+t`, `` ctrl+` ``, `ctrl+\`, `alt+up`.

use std::fmt;

use smallvec::SmallVec;
use thiserror::Error;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, PartialOrd, Ord)]
pub struct Mods(u8);

impl Mods {
    pub const NONE: Mods = Mods(0);
    pub const CTRL: Mods = Mods(1);
    pub const SHIFT: Mods = Mods(2);
    pub const ALT: Mods = Mods(4);
    pub const META: Mods = Mods(8);

    pub const fn contains(self, other: Mods) -> bool {
        self.0 & other.0 == other.0
    }
    pub const fn union(self, other: Mods) -> Mods {
        Mods(self.0 | other.0)
    }
    pub const fn without(self, other: Mods) -> Mods {
        Mods(self.0 & !other.0)
    }
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl std::ops::BitOr for Mods {
    type Output = Mods;
    fn bitor(self, rhs: Mods) -> Mods {
        self.union(rhs)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub enum KeyCode {
    /// A printable character, stored lowercase for ASCII letters.
    Char(char),
    F(u8),
    Enter,
    Tab,
    Backspace,
    Escape,
    Space,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct KeyPress {
    pub code: KeyCode,
    pub mods: Mods,
}

impl KeyPress {
    pub const fn new(code: KeyCode, mods: Mods) -> Self {
        Self { code, mods }
    }

    /// Normalise a key as delivered by the terminal so it can be matched
    /// against bindings: ASCII letters become lowercase + Shift; Shift is
    /// dropped from other printable characters typed without Ctrl/Alt
    /// (the character itself already reflects Shift).
    pub fn normalized(code: KeyCode, mods: Mods) -> Self {
        match code {
            KeyCode::Char(' ') => Self::new(KeyCode::Space, mods),
            KeyCode::Char(c) if c.is_ascii_uppercase() => {
                Self::new(KeyCode::Char(c.to_ascii_lowercase()), mods | Mods::SHIFT)
            }
            KeyCode::Char(c)
                if !c.is_ascii_alphabetic()
                    && mods.contains(Mods::SHIFT)
                    && !mods.contains(Mods::CTRL)
                    && !mods.contains(Mods::ALT) =>
            {
                Self::new(code, mods.without(Mods::SHIFT))
            }
            _ => Self::new(code, mods),
        }
    }

    /// The character this key would insert as text, if any.
    pub fn text_char(&self, original: Option<char>) -> Option<char> {
        if self.mods.contains(Mods::CTRL) || self.mods.contains(Mods::ALT) || self.mods.contains(Mods::META) {
            return None;
        }
        match self.code {
            KeyCode::Char(_) => original,
            KeyCode::Space => Some(' '),
            _ => None,
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("empty key")]
    Empty,
    #[error("unknown key `{0}`")]
    UnknownKey(String),
    #[error("unknown modifier `{0}`")]
    UnknownModifier(String),
}

fn parse_code(s: &str) -> Result<KeyCode, ParseError> {
    let lower = s.to_ascii_lowercase();
    Ok(match lower.as_str() {
        "enter" | "return" => KeyCode::Enter,
        "tab" => KeyCode::Tab,
        "backspace" => KeyCode::Backspace,
        "escape" | "esc" => KeyCode::Escape,
        "space" => KeyCode::Space,
        "up" | "uparrow" => KeyCode::Up,
        "down" | "downarrow" => KeyCode::Down,
        "left" | "leftarrow" => KeyCode::Left,
        "right" | "rightarrow" => KeyCode::Right,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" | "pgup" => KeyCode::PageUp,
        "pagedown" | "pgdn" => KeyCode::PageDown,
        "insert" => KeyCode::Insert,
        "delete" | "del" => KeyCode::Delete,
        "backtick" => KeyCode::Char('`'),
        "backslash" => KeyCode::Char('\\'),
        "plus" => KeyCode::Char('+'),
        f if f.len() >= 2 && f.starts_with('f') && f[1..].chars().all(|c| c.is_ascii_digit()) => {
            let n: u8 = f[1..].parse().map_err(|_| ParseError::UnknownKey(s.into()))?;
            if !(1..=24).contains(&n) {
                return Err(ParseError::UnknownKey(s.into()));
            }
            KeyCode::F(n)
        }
        _ => {
            let mut chars = s.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => KeyCode::Char(c.to_ascii_lowercase()),
                _ => return Err(ParseError::UnknownKey(s.into())),
            }
        }
    })
}

impl std::str::FromStr for KeyPress {
    type Err = ParseError;
    /// Parse `ctrl+shift+p`. A trailing `+` key is written `ctrl++` or `ctrl+plus`.
    fn from_str(s: &str) -> Result<Self, ParseError> {
        let s = s.trim();
        if s.is_empty() {
            return Err(ParseError::Empty);
        }
        let (mods_part, key_part) = if let Some(stripped) = s.strip_suffix("++") {
            (stripped, "+")
        } else if s == "+" {
            ("", "+")
        } else {
            match s.rfind('+') {
                Some(i) => (&s[..i], &s[i + 1..]),
                None => ("", s),
            }
        };
        let mut mods = Mods::NONE;
        for m in mods_part.split('+').filter(|m| !m.is_empty()) {
            mods = mods
                | match m.to_ascii_lowercase().as_str() {
                    "ctrl" | "control" => Mods::CTRL,
                    "shift" => Mods::SHIFT,
                    "alt" | "option" => Mods::ALT,
                    "meta" | "cmd" | "super" | "win" => Mods::META,
                    other => return Err(ParseError::UnknownModifier(other.into())),
                };
        }
        Ok(KeyPress::new(parse_code(key_part)?, mods))
    }
}

impl fmt::Display for KeyPress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.mods.contains(Mods::CTRL) {
            f.write_str("Ctrl+")?;
        }
        if self.mods.contains(Mods::SHIFT) {
            f.write_str("Shift+")?;
        }
        if self.mods.contains(Mods::ALT) {
            f.write_str("Alt+")?;
        }
        if self.mods.contains(Mods::META) {
            f.write_str("Meta+")?;
        }
        match self.code {
            KeyCode::Char(c) => write!(f, "{}", c.to_ascii_uppercase()),
            KeyCode::F(n) => write!(f, "F{n}"),
            KeyCode::Enter => f.write_str("Enter"),
            KeyCode::Tab => f.write_str("Tab"),
            KeyCode::Backspace => f.write_str("Backspace"),
            KeyCode::Escape => f.write_str("Escape"),
            KeyCode::Space => f.write_str("Space"),
            KeyCode::Up => f.write_str("Up"),
            KeyCode::Down => f.write_str("Down"),
            KeyCode::Left => f.write_str("Left"),
            KeyCode::Right => f.write_str("Right"),
            KeyCode::Home => f.write_str("Home"),
            KeyCode::End => f.write_str("End"),
            KeyCode::PageUp => f.write_str("PageUp"),
            KeyCode::PageDown => f.write_str("PageDown"),
            KeyCode::Insert => f.write_str("Insert"),
            KeyCode::Delete => f.write_str("Delete"),
        }
    }
}

/// A sequence of one or more presses (chords).
#[derive(Clone, PartialEq, Eq, Hash, Debug, Default, PartialOrd, Ord)]
pub struct KeySeq(pub SmallVec<[KeyPress; 2]>);

impl KeySeq {
    pub fn single(k: KeyPress) -> Self {
        let mut v = SmallVec::new();
        v.push(k);
        Self(v)
    }
    pub fn presses(&self) -> &[KeyPress] {
        &self.0
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn starts_with(&self, prefix: &[KeyPress]) -> bool {
        self.0.starts_with(prefix)
    }
}

impl std::str::FromStr for KeySeq {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<Self, ParseError> {
        let presses: Result<SmallVec<_>, _> = s.split_whitespace().map(str::parse).collect();
        let presses = presses?;
        if presses.is_empty() {
            return Err(ParseError::Empty);
        }
        Ok(KeySeq(presses))
    }
}

impl fmt::Display for KeySeq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, k) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str(" ")?;
            }
            write!(f, "{k}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_presses_and_sequences() {
        let k: KeyPress = "ctrl+shift+p".parse().unwrap();
        assert_eq!(k, KeyPress::new(KeyCode::Char('p'), Mods::CTRL | Mods::SHIFT));
        let k: KeyPress = "Ctrl+`".parse().unwrap();
        assert_eq!(k.code, KeyCode::Char('`'));
        let k: KeyPress = "ctrl+\\".parse().unwrap();
        assert_eq!(k.code, KeyCode::Char('\\'));
        let k: KeyPress = "ctrl++".parse().unwrap();
        assert_eq!(k.code, KeyCode::Char('+'));
        assert_eq!("f12".parse::<KeyPress>().unwrap().code, KeyCode::F(12));
        let s: KeySeq = "ctrl+k ctrl+t".parse().unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!(s.to_string(), "Ctrl+K Ctrl+T");
        assert!("hyper+x".parse::<KeyPress>().is_err());
        assert!("ctrl+nope".parse::<KeyPress>().is_err());
    }

    #[test]
    fn normalization() {
        let k = KeyPress::normalized(KeyCode::Char('P'), Mods::CTRL);
        assert_eq!(k, KeyPress::new(KeyCode::Char('p'), Mods::CTRL | Mods::SHIFT));
        let k = KeyPress::normalized(KeyCode::Char('!'), Mods::SHIFT);
        assert_eq!(k, KeyPress::new(KeyCode::Char('!'), Mods::NONE));
        let k = KeyPress::normalized(KeyCode::Char(' '), Mods::CTRL);
        assert_eq!(k.code, KeyCode::Space);
    }
}
