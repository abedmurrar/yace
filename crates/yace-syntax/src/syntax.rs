//! Incremental parsing straight from a rope (no copy of the document), with
//! a time budget so the UI thread is never blocked (spec §12.1, §21.4).

use std::{
    ops::ControlFlow,
    time::{Duration, Instant},
};

use ropey::Rope;
use tree_sitter::{InputEdit, Node, ParseOptions, Parser, Point, TextProvider, Tree};

use crate::language::LanguageConfig;

/// Feeds tree-sitter from rope chunks.
fn read_rope(rope: &Rope, byte: usize) -> &[u8] {
    if byte >= rope.len_bytes() {
        return &[];
    }
    let (chunk, chunk_byte, _, _) = rope.chunk_at_byte(byte);
    &chunk.as_bytes()[byte - chunk_byte..]
}

/// `TextProvider` for queries with text predicates (`#eq?`, `#match?`).
pub struct RopeText<'a>(pub &'a Rope);

pub struct ChunkIter<'a> {
    chunks: ropey::iter::Chunks<'a>,
}

impl<'a> Iterator for ChunkIter<'a> {
    type Item = &'a [u8];
    fn next(&mut self) -> Option<&'a [u8]> {
        self.chunks.next().map(str::as_bytes)
    }
}

impl<'a> TextProvider<&'a [u8]> for RopeText<'a> {
    type I = ChunkIter<'a>;
    fn text(&mut self, node: Node) -> Self::I {
        let r = node.byte_range();
        let end = r.end.min(self.0.len_bytes());
        let start = r.start.min(end);
        ChunkIter { chunks: self.0.byte_slice(start..end).chunks() }
    }
}

fn point_at(rope: &Rope, byte: usize) -> Point {
    let byte = byte.min(rope.len_bytes());
    let row = rope.byte_to_line(byte);
    Point { row, column: byte - rope.line_to_byte(row) }
}

/// Advance a point over inserted text.
fn point_after(start: Point, text: &str) -> Point {
    match text.rfind('\n') {
        Some(i) => Point { row: start.row + text.matches('\n').count(), column: text.len() - i - 1 },
        None => Point { row: start.row, column: start.column + text.len() },
    }
}

/// Tree-sitter edits for a set of non-overlapping replacements given in
/// *pre-edit char coordinates* (sorted). They are returned in application
/// order (last first), so each one's positions are valid when applied.
pub fn input_edits(before: &Rope, changes: &[(usize, usize, &str)]) -> Vec<InputEdit> {
    changes
        .iter()
        .rev()
        .map(|&(from, to, insert)| {
            let start_byte = before.char_to_byte(from.min(before.len_chars()));
            let old_end_byte = before.char_to_byte(to.min(before.len_chars()));
            let start_position = point_at(before, start_byte);
            InputEdit {
                start_byte,
                old_end_byte,
                new_end_byte: start_byte + insert.len(),
                start_position,
                old_end_position: point_at(before, old_end_byte),
                new_end_position: point_after(start_position, insert),
            }
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseOutcome {
    Done,
    /// The budget ran out; the old tree (edited) is kept.
    TimedOut,
    Failed,
}

/// A document's syntax tree.
pub struct Syntax {
    pub lang: &'static LanguageConfig,
    parser: Parser,
    tree: Option<Tree>,
    /// Document version the tree corresponds to.
    pub version: u64,
    /// The tree is behind the document (edits applied, reparse pending).
    pub stale: bool,
}

impl std::fmt::Debug for Syntax {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Syntax")
            .field("lang", &self.lang.id)
            .field("version", &self.version)
            .field("stale", &self.stale)
            .finish()
    }
}

impl Syntax {
    pub fn new(lang: &'static LanguageConfig) -> Option<Self> {
        let language = lang.language()?;
        let mut parser = Parser::new();
        parser.set_language(language).ok()?;
        Some(Self { lang, parser, tree: None, version: 0, stale: true })
    }

    pub fn tree(&self) -> Option<&Tree> {
        self.tree.as_ref()
    }

    /// Apply edits to the existing tree (cheap; positions stay valid for
    /// unchanged regions until the next parse).
    pub fn edit(&mut self, edits: &[InputEdit]) {
        if let Some(t) = self.tree.as_mut() {
            for e in edits {
                t.edit(e);
            }
        }
        self.stale = true;
    }

    /// Parse (incrementally when a tree exists). With a budget, parsing
    /// stops early and returns `TimedOut`.
    pub fn parse(&mut self, rope: &Rope, version: u64, budget: Option<Duration>) -> ParseOutcome {
        let start = Instant::now();
        let mut progress = |_: &tree_sitter::ParseState| {
            if budget.is_some_and(|b| start.elapsed() > b) { ControlFlow::Break(()) } else { ControlFlow::Continue(()) }
        };
        let options = ParseOptions::new().progress_callback(&mut progress);
        let result =
            self.parser.parse_with_options(&mut |byte, _| read_rope(rope, byte), self.tree.as_ref(), Some(options));
        match result {
            Some(tree) => {
                self.tree = Some(tree);
                self.version = version;
                self.stale = false;
                ParseOutcome::Done
            }
            None => {
                self.parser.reset();
                if budget.is_some() { ParseOutcome::TimedOut } else { ParseOutcome::Failed }
            }
        }
    }

    /// Install a tree parsed elsewhere (the syntax worker).
    pub fn set_tree(&mut self, tree: Tree, version: u64) {
        self.tree = Some(tree);
        self.version = version;
        self.stale = false;
    }

    /// The tree to hand to a background reparse.
    pub fn tree_for_worker(&self) -> Option<Tree> {
        self.tree.clone()
    }
}

/// One-shot parse (for the background worker).
pub fn parse_full(lang: &'static LanguageConfig, rope: &Rope, old: Option<&Tree>) -> Option<Tree> {
    let mut parser = Parser::new();
    parser.set_language(lang.language()?).ok()?;
    parser.parse_with_options(&mut |byte, _| read_rope(rope, byte), old, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::by_id;

    #[test]
    fn parses_from_chunks_and_incrementally() {
        let src = "fn main() {\n    let x = 1;\n}\n".repeat(2000);
        let mut rope = Rope::from_str(&src);
        let mut s = Syntax::new(by_id("rust").unwrap()).unwrap();
        assert_eq!(s.parse(&rope, 1, None), ParseOutcome::Done);
        assert!(!s.tree().unwrap().root_node().has_error());
        // Edit: insert a statement inside the first function.
        let at = rope.line_to_char(1);
        let before = rope.clone();
        rope.insert(at, "    let y = 2;\n");
        s.edit(&input_edits(&before, &[(at, at, "    let y = 2;\n")]));
        assert!(s.stale);
        let start = Instant::now();
        assert_eq!(s.parse(&rope, 2, Some(Duration::from_millis(50))), ParseOutcome::Done);
        assert!(start.elapsed() < Duration::from_millis(50), "incremental reparse is fast");
        let full = parse_full(by_id("rust").unwrap(), &rope, None).unwrap();
        assert_eq!(s.tree().unwrap().root_node().to_sexp(), full.root_node().to_sexp(), "incremental == full parse");
    }

    #[test]
    fn budget_times_out_on_huge_input() {
        let src = "let x = [1, 2, 3, 4];\n".repeat(200_000);
        let rope = Rope::from_str(&src);
        let mut s = Syntax::new(by_id("javascript").unwrap()).unwrap();
        assert_eq!(s.parse(&rope, 1, Some(Duration::from_micros(200))), ParseOutcome::TimedOut);
        assert!(s.tree().is_none());
    }

    #[test]
    fn edits_with_multibyte_text() {
        let before = Rope::from_str("a = '中'\nb");
        let e = input_edits(&before, &[(5, 6, "文字"), (8, 9, "c\nd")]);
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].start_byte, before.char_to_byte(8));
        assert_eq!(e[1].start_byte, 5);
        assert_eq!(e[1].old_end_byte, 8, "'中' is 3 bytes");
        assert_eq!(e[1].new_end_byte, 5 + "文字".len());
        assert_eq!(e[0].new_end_position, Point { row: 2, column: 1 });
    }
}
