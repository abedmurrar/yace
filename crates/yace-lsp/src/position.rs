//! Rope char index ↔ LSP `Position` in the negotiated encoding.

use ropey::Rope;

use crate::types::Position;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Encoding {
    Utf8,
    #[default]
    Utf16,
    Utf32,
}

impl Encoding {
    pub fn from_name(s: &str) -> Encoding {
        match s {
            "utf-8" => Encoding::Utf8,
            "utf-32" => Encoding::Utf32,
            _ => Encoding::Utf16,
        }
    }
}

fn units(c: char, enc: Encoding) -> u32 {
    match enc {
        Encoding::Utf8 => c.len_utf8() as u32,
        Encoding::Utf16 => c.len_utf16() as u32,
        Encoding::Utf32 => 1,
    }
}

/// Char index → position.
pub fn to_lsp(rope: &Rope, pos: usize, enc: Encoding) -> Position {
    let pos = pos.min(rope.len_chars());
    let line = rope.char_to_line(pos);
    let start = rope.line_to_char(line);
    let character = match enc {
        Encoding::Utf32 => (pos - start) as u32,
        _ => rope.slice(start..pos).chars().map(|c| units(c, enc)).sum(),
    };
    Position { line: line as u32, character }
}

/// Position → char index, clamped to the line (a position past the end of
/// the line, or inside a code point, lands on a valid boundary).
pub fn from_lsp(rope: &Rope, p: Position, enc: Encoding) -> usize {
    let line = p.line as usize;
    if line >= rope.len_lines() {
        return rope.len_chars();
    }
    let start = rope.line_to_char(line);
    let slice = rope.line(line);
    let mut used = 0u32;
    let mut chars = 0usize;
    for c in slice.chars() {
        if c == '\n' || c == '\r' || used >= p.character {
            break;
        }
        let u = units(c, enc);
        if used + u > p.character {
            break;
        }
        used += u;
        chars += 1;
    }
    start + chars
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_utf8_and_clamping() {
        let r = Rope::from_str("a😀b\r\n中x\n");
        let b = 2; // char index of 'b' (the emoji is one char)
        assert_eq!(to_lsp(&r, b, Encoding::Utf16), Position { line: 0, character: 3 });
        assert_eq!(to_lsp(&r, b, Encoding::Utf8), Position { line: 0, character: 5 });
        assert_eq!(to_lsp(&r, b, Encoding::Utf32), Position { line: 0, character: 2 });
        assert_eq!(from_lsp(&r, Position { line: 0, character: 3 }, Encoding::Utf16), b);
        assert_eq!(from_lsp(&r, Position { line: 0, character: 5 }, Encoding::Utf8), b);
        // Inside the surrogate pair → before the emoji.
        assert_eq!(from_lsp(&r, Position { line: 0, character: 2 }, Encoding::Utf16), 1);
        // Past the end of the line (CRLF) → end of content.
        assert_eq!(from_lsp(&r, Position { line: 0, character: 99 }, Encoding::Utf16), 3);
        let x = r.line_to_char(1) + 1;
        assert_eq!(to_lsp(&r, x, Encoding::Utf8), Position { line: 1, character: 3 });
        assert_eq!(from_lsp(&r, Position { line: 9, character: 0 }, Encoding::Utf16), r.len_chars());
    }
}
