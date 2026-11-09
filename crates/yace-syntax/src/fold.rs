//! Folding ranges (spec §12): from the syntax tree when there is one,
//! otherwise from indentation. A range is `(header, last)`: lines
//! `header + 1 ..= last` are hidden. As in VS Code, a closing line
//! (`}`, `]`, `)`, `</tag>`, `end`) stays visible.
//!
//! Ranges are computed per line on demand (O(tree depth)), so only the
//! visible lines are examined while rendering.

use ropey::{Rope, RopeSlice};
use tree_sitter::{Node, Point, Tree};

fn line_text(rope: &Rope, line: usize) -> RopeSlice<'_> {
    let s = rope.line(line);
    let mut n = s.len_chars();
    while n > 0 && matches!(s.char(n - 1), '\n' | '\r') {
        n -= 1;
    }
    s.slice(..n)
}

fn first_non_ws(s: RopeSlice) -> Option<usize> {
    s.chars().position(|c| !c.is_whitespace())
}

fn is_blank(rope: &Rope, line: usize) -> bool {
    first_non_ws(line_text(rope, line)).is_none()
}

/// The closing line stays visible when it starts with a closer.
fn keeps_last_line(rope: &Rope, line: usize) -> bool {
    let s = line_text(rope, line);
    let Some(i) = first_non_ws(s) else { return false };
    let rest: String = s.slice(i..).chars().take(4).collect();
    rest.starts_with(['}', ']', ')']) || rest.starts_with("</") || rest == "end" || rest.starts_with("end ")
}

fn finish(rope: &Rope, header: usize, end_row: usize) -> Option<(usize, usize)> {
    let last = if keeps_last_line(rope, end_row) { end_row.saturating_sub(1) } else { end_row };
    (last > header).then_some((header, last))
}

/// The end row of a node, ignoring a trailing newline that belongs to it.
fn end_row(n: Node) -> usize {
    let e = n.end_position();
    if e.column == 0 && e.row > n.start_position().row { e.row - 1 } else { e.row }
}

/// The fold range whose header is `line`, from the tree.
pub fn tree_range_at(tree: &Tree, rope: &Rope, line: usize) -> Option<(usize, usize)> {
    if line >= rope.len_lines() {
        return None;
    }
    let text = line_text(rope, line);
    let col = first_non_ws(text)?;
    let byte_col = text.char_to_byte(col);
    let root = tree.root_node();
    let p = Point { row: line, column: byte_col };
    let mut node = root.descendant_for_point_range(p, p)?;
    let mut best: Option<usize> = None;
    // Walk up while the ancestors start on this line; the outermost
    // multi-line one wins (one fold per header line, like VS Code).
    loop {
        if node.id() == root.id() {
            break;
        }
        if node.start_position().row != line {
            break;
        }
        let er = end_row(node);
        if er > line && node.is_named() {
            best = Some(best.map_or(er, |b: usize| b.max(er)));
        }
        match node.parent() {
            Some(p) => node = p,
            None => break,
        }
    }
    // A block opened at the end of the line (`if x {`, `foo({`) starts after
    // the first token: look at the last token too.
    if best.is_none() {
        let last_col = text.len_chars().checked_sub(1)?;
        let lb = text.char_to_byte(last_col);
        let p = Point { row: line, column: lb };
        let mut n = root.descendant_for_point_range(p, p)?;
        while let Some(parent) = n.parent() {
            if parent.id() == root.id() || parent.start_position().row != line {
                break;
            }
            let er = end_row(parent);
            if er > line {
                best = Some(er);
                break;
            }
            n = parent;
        }
    }
    finish(rope, line, best?)
}

fn indent_of(rope: &Rope, line: usize, tab: usize) -> usize {
    let mut w = 0;
    for c in line_text(rope, line).chars() {
        match c {
            ' ' => w += 1,
            '\t' => w += tab - w % tab,
            _ => break,
        }
    }
    w
}

/// The fold range whose header is `line`, from indentation: the following
/// lines indented deeper (blank lines inside count, trailing ones don't).
pub fn indent_range_at(rope: &Rope, line: usize, tab: usize) -> Option<(usize, usize)> {
    let n = rope.len_lines();
    if line + 1 >= n || is_blank(rope, line) {
        return None;
    }
    let base = indent_of(rope, line, tab.max(1));
    let mut last = None;
    let mut l = line + 1;
    // Cap the scan so a huge flat file stays cheap.
    let cap = (line + 100_000).min(n);
    while l < cap {
        if !is_blank(rope, l) {
            if indent_of(rope, l, tab.max(1)) <= base {
                break;
            }
            last = Some(l);
        }
        l += 1;
    }
    let last = last?;
    // Include a closing line at the base indent that starts with a closer,
    // then let `finish` keep it visible.
    let end = if l < n && !is_blank(rope, l) && indent_of(rope, l, tab.max(1)) == base && keeps_last_line(rope, l) {
        l
    } else {
        last
    };
    finish(rope, line, end)
}

/// The fold range at `line` (tree first, indentation otherwise).
pub fn range_at(tree: Option<&Tree>, rope: &Rope, line: usize, tab: usize) -> Option<(usize, usize)> {
    match tree {
        Some(t) => tree_range_at(t, rope, line),
        None => indent_range_at(rope, line, tab),
    }
}

/// True when the range with header `line` is a comment (Fold All Block
/// Comments): a comment node starts the line in the tree, or, without a
/// tree, the line starts with the language's block-comment opener.
pub fn is_comment_at(tree: Option<&Tree>, rope: &Rope, line: usize, block_open: Option<&str>) -> bool {
    let text = line_text(rope, line);
    let Some(col) = first_non_ws(text) else { return false };
    match tree {
        Some(t) => {
            let p = Point { row: line, column: text.char_to_byte(col) };
            let Some(mut node) = t.root_node().descendant_for_point_range(p, p) else { return false };
            loop {
                if node.start_position().row != line {
                    return false;
                }
                if node.kind().contains("comment") {
                    return end_row(node) > line;
                }
                match node.parent() {
                    Some(p) => node = p,
                    None => return false,
                }
            }
        }
        None => block_open.is_some_and(|o| {
            let start: String = text.slice(col..).chars().take(o.chars().count()).collect();
            start == o
        }),
    }
}

/// Nesting level of each range (1 = outermost), for ranges sorted by
/// header line: a range is inside the closest earlier one that covers it.
pub fn nesting_levels(ranges: &[(usize, usize)]) -> Vec<usize> {
    let mut stack: Vec<usize> = Vec::new(); // last lines of the open ranges
    ranges
        .iter()
        .map(|(h, l)| {
            while stack.last().is_some_and(|end| *end < *h) {
                stack.pop();
            }
            stack.push(*l);
            stack.len()
        })
        .collect()
}

/// Every fold range (Fold All). Linear in the number of lines.
pub fn all_ranges(tree: Option<&Tree>, rope: &Rope, tab: usize) -> Vec<(usize, usize)> {
    (0..rope.len_lines()).filter_map(|l| range_at(tree, rope, l, tab)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{language::by_id, syntax::parse_full};

    #[test]
    fn rust_blocks_keep_the_closing_line() {
        let src = "fn main() {\n    let x = 1;\n    if x > 0 {\n        go();\n    }\n}\n";
        let rope = Rope::from_str(src);
        let tree = parse_full(by_id("rust").unwrap(), &rope, None).unwrap();
        assert_eq!(tree_range_at(&tree, &rope, 0), Some((0, 4)));
        assert_eq!(tree_range_at(&tree, &rope, 2), Some((2, 3)));
        assert_eq!(tree_range_at(&tree, &rope, 1), None);
    }

    #[test]
    fn typescript_objects_and_trailing_openers() {
        let src = "const cfg = {\n  a: 1,\n  b: [\n    2,\n  ],\n};\nfoo(() => {\n  bar();\n});\n";
        let rope = Rope::from_str(src);
        let tree = parse_full(by_id("typescript").unwrap(), &rope, None).unwrap();
        assert_eq!(tree_range_at(&tree, &rope, 0), Some((0, 4)));
        assert_eq!(tree_range_at(&tree, &rope, 2), Some((2, 3)));
        assert_eq!(tree_range_at(&tree, &rope, 6), Some((6, 7)));
    }

    #[test]
    fn python_and_indentation() {
        let src = "def f():\n    a = 1\n\n    return a\nx = 2\n";
        let rope = Rope::from_str(src);
        let tree = parse_full(by_id("python").unwrap(), &rope, None).unwrap();
        assert_eq!(tree_range_at(&tree, &rope, 0), Some((0, 3)));
        assert_eq!(indent_range_at(&rope, 0, 4), Some((0, 3)));
        let yaml = Rope::from_str("a:\n  b: 1\n  c:\n    d: 2\ne: 3\n");
        assert_eq!(indent_range_at(&yaml, 0, 2), Some((0, 3)));
        assert_eq!(indent_range_at(&yaml, 2, 2), Some((2, 3)));
        assert_eq!(indent_range_at(&yaml, 4, 2), None);
        let braces = Rope::from_str("a {\n  b\n}\n");
        assert_eq!(indent_range_at(&braces, 0, 2), Some((0, 1)));
    }

    #[test]
    fn markdown_sections() {
        let src = "# A\n\ntext\n\n## B\n\nmore\n";
        let rope = Rope::from_str(src);
        let tree = parse_full(by_id("markdown").unwrap(), &rope, None).unwrap();
        assert!(tree_range_at(&tree, &rope, 0).is_some_and(|(h, l)| h == 0 && l >= 5));
    }

    #[test]
    fn levels_and_comments() {
        assert_eq!(nesting_levels(&[(0, 10), (1, 4), (2, 3), (5, 9), (12, 14)]), vec![1, 2, 3, 2, 1]);
        let src = "/* a\n   b */\nfn f() {\n    if x {\n        y();\n    }\n}\n";
        let rope = Rope::from_str(src);
        let tree = parse_full(by_id("rust").unwrap(), &rope, None).unwrap();
        assert!(is_comment_at(Some(&tree), &rope, 0, None));
        assert!(!is_comment_at(Some(&tree), &rope, 2, None));
        assert!(is_comment_at(None, &rope, 0, Some("/*")));
        assert!(!is_comment_at(None, &rope, 2, Some("/*")));
    }
}
