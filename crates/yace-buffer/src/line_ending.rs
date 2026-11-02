//! Line-ending detection. Policy (TODO.md D6): text is stored byte-exact,
//! new lines use the dominant ending, mixed files are preserved as-is.

use ropey::Rope;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LineEnding {
    #[default]
    Lf,
    CrLf,
    Cr,
}

impl LineEnding {
    pub fn as_str(self) -> &'static str {
        match self {
            LineEnding::Lf => "\n",
            LineEnding::CrLf => "\r\n",
            LineEnding::Cr => "\r",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            LineEnding::Lf => "LF",
            LineEnding::CrLf => "CRLF",
            LineEnding::Cr => "CR",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct LineEndingInfo {
    pub dominant: LineEnding,
    pub mixed: bool,
}

impl LineEndingInfo {
    pub fn label(&self) -> &'static str {
        if self.mixed { "Mixed" } else { self.dominant.label() }
    }
}

/// Count line endings. `default` is used when the text has none.
pub fn detect(rope: &Rope, default: LineEnding) -> LineEndingInfo {
    let (mut lf, mut crlf, mut cr) = (0usize, 0usize, 0usize);
    let mut prev_cr = false;
    for chunk in rope.chunks() {
        for b in chunk.bytes() {
            match b {
                b'\n' => {
                    if prev_cr {
                        crlf += 1;
                        cr -= 1;
                    } else {
                        lf += 1;
                    }
                    prev_cr = false;
                }
                b'\r' => {
                    cr += 1;
                    prev_cr = true;
                }
                _ => prev_cr = false,
            }
        }
    }
    let kinds = [lf > 0, crlf > 0, cr > 0].iter().filter(|k| **k).count();
    let dominant = if lf == 0 && crlf == 0 && cr == 0 {
        default
    } else if crlf > lf && crlf >= cr {
        LineEnding::CrLf
    } else if cr > lf && cr > crlf {
        LineEnding::Cr
    } else {
        LineEnding::Lf
    };
    LineEndingInfo { dominant, mixed: kinds > 1 }
}

/// Convert every line ending to `to`.
pub fn normalize(text: &str, to: LineEnding) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push_str(to.as_str());
            }
            '\n' => out.push_str(to.as_str()),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects() {
        let d = |s: &str| detect(&Rope::from_str(s), LineEnding::Lf);
        assert_eq!(d("a\nb\n"), LineEndingInfo { dominant: LineEnding::Lf, mixed: false });
        assert_eq!(d("a\r\nb\r\n"), LineEndingInfo { dominant: LineEnding::CrLf, mixed: false });
        let m = d("a\r\nb\r\nc\n");
        assert!(m.mixed);
        assert_eq!(m.dominant, LineEnding::CrLf);
        assert_eq!(d("abc").dominant, LineEnding::Lf);
        assert_eq!(detect(&Rope::from_str("x"), LineEnding::CrLf).dominant, LineEnding::CrLf);
    }

    #[test]
    fn normalizes() {
        assert_eq!(normalize("a\r\nb\nc\rd", LineEnding::Lf), "a\nb\nc\nd");
        assert_eq!(normalize("a\nb", LineEnding::CrLf), "a\r\nb");
    }
}
