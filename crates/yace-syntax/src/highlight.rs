//! Viewport-bounded highlighting (spec §12.3, §22.3, §22.4).
//!
//! Only the requested byte range is queried. Captures are painted into a
//! per-byte scope buffer (outer nodes first, inner nodes override; for the
//! same node, the *later* query pattern wins, so specific patterns override
//! generic ones like `(identifier) @variable`, as in Neovim), then
//! compressed into spans. Injected languages (Markdown code fences,
//! `<script>`/`<style>`, template literals, Markdown inline) are parsed
//! lazily for the visible part only and cached per document version.

use std::{collections::HashMap, ops::Range, sync::OnceLock};

use ropey::Rope;
use tree_sitter::{Parser, Point, Query, QueryCursor, StreamingIterator, Tree};

use crate::{
    language::{LanguageConfig, by_injection_name},
    syntax::RopeText,
};

/// Colour classes. Themes give each a colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Scope {
    None = 0,
    Comment,
    String,
    StringSpecial,
    Escape,
    Character,
    Number,
    Boolean,
    Constant,
    ConstantBuiltin,
    Keyword,
    KeywordControl,
    Operator,
    Punctuation,
    PunctuationBracket,
    Function,
    FunctionBuiltin,
    FunctionMethod,
    Macro,
    Type,
    TypeBuiltin,
    Constructor,
    Variable,
    VariableBuiltin,
    VariableParameter,
    Property,
    Attribute,
    Tag,
    Namespace,
    Label,
    Heading,
    Bold,
    Italic,
    Link,
    Raw,
    ListMarker,
    Embedded,
}

impl Scope {
    pub const COUNT: usize = Scope::Embedded as usize + 1;

    fn from_u8(v: u8) -> Scope {
        ALL.get(usize::from(v)).copied().unwrap_or(Scope::None)
    }

    /// Map a capture name (`function.method.call`, `keyword.control`, …)
    /// by its longest known prefix.
    pub fn from_capture(name: &str) -> Scope {
        let mut n = name;
        loop {
            if let Some(s) = exact(n) {
                return s;
            }
            match n.rfind('.') {
                Some(i) => n = &n[..i],
                None => return Scope::None,
            }
        }
    }
}

const ALL: [Scope; Scope::COUNT] = [
    Scope::None,
    Scope::Comment,
    Scope::String,
    Scope::StringSpecial,
    Scope::Escape,
    Scope::Character,
    Scope::Number,
    Scope::Boolean,
    Scope::Constant,
    Scope::ConstantBuiltin,
    Scope::Keyword,
    Scope::KeywordControl,
    Scope::Operator,
    Scope::Punctuation,
    Scope::PunctuationBracket,
    Scope::Function,
    Scope::FunctionBuiltin,
    Scope::FunctionMethod,
    Scope::Macro,
    Scope::Type,
    Scope::TypeBuiltin,
    Scope::Constructor,
    Scope::Variable,
    Scope::VariableBuiltin,
    Scope::VariableParameter,
    Scope::Property,
    Scope::Attribute,
    Scope::Tag,
    Scope::Namespace,
    Scope::Label,
    Scope::Heading,
    Scope::Bold,
    Scope::Italic,
    Scope::Link,
    Scope::Raw,
    Scope::ListMarker,
    Scope::Embedded,
];

fn exact(n: &str) -> Option<Scope> {
    Some(match n {
        "comment" | "comment.documentation" => Scope::Comment,
        "string" | "string.special.key" => Scope::String,
        "string.special" | "string.regex" | "string.regexp" | "regex" | "string.special.symbol" | "symbol" => {
            Scope::StringSpecial
        }
        "string.escape" | "escape" => Scope::Escape,
        "character" => Scope::Character,
        "number" | "float" | "constant.numeric" | "number.float" => Scope::Number,
        "boolean" | "constant.builtin.boolean" => Scope::Boolean,
        "constant" => Scope::Constant,
        "constant.builtin" => Scope::ConstantBuiltin,
        "keyword" | "keyword.function" | "keyword.operator" | "keyword.storage" | "keyword.type" | "include"
        | "storageclass" => Scope::Keyword,
        "keyword.control"
        | "keyword.return"
        | "keyword.conditional"
        | "keyword.repeat"
        | "keyword.exception"
        | "conditional"
        | "repeat"
        | "exception"
        | "keyword.import"
        | "keyword.directive" => Scope::KeywordControl,
        "operator" => Scope::Operator,
        "punctuation" | "punctuation.delimiter" | "punctuation.special" => Scope::Punctuation,
        "punctuation.bracket" => Scope::PunctuationBracket,
        "function" | "function.call" => Scope::Function,
        "function.builtin" => Scope::FunctionBuiltin,
        "function.method" | "method" | "function.method.call" => Scope::FunctionMethod,
        "function.macro" | "macro" | "preproc" | "keyword.directive.define" => Scope::Macro,
        "type" | "type.definition" | "type.qualifier" => Scope::Type,
        "type.builtin" => Scope::TypeBuiltin,
        "constructor" => Scope::Constructor,
        "variable" | "variable.member" => Scope::Variable,
        "variable.builtin" => Scope::VariableBuiltin,
        "variable.parameter" | "parameter" => Scope::VariableParameter,
        "property" | "field" | "variable.other.member" | "property.definition" => Scope::Property,
        "attribute" | "tag.attribute" | "decorator" => Scope::Attribute,
        "tag" | "tag.delimiter" | "tag.builtin" => Scope::Tag,
        "namespace" | "module" => Scope::Namespace,
        "label" => Scope::Label,
        "text.title" | "markup.heading" | "heading" | "title" => Scope::Heading,
        "text.strong" | "markup.bold" | "markup.strong" => Scope::Bold,
        "text.emphasis" | "markup.italic" => Scope::Italic,
        "text.uri" | "text.reference" | "markup.link" | "markup.link.url" | "markup.link.label" | "link_uri"
        | "link_text" => Scope::Link,
        "text.literal" | "markup.raw" | "markup.raw.block" | "markup.raw.inline" => Scope::Raw,
        "punctuation.list_marker" | "markup.list" => Scope::ListMarker,
        "embedded" | "punctuation.special.embedded" => Scope::Embedded,
        _ => return None,
    })
}

/// A highlighted byte span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub scope: Scope,
}

fn scopes_for(query: &Query) -> Vec<Scope> {
    query.capture_names().iter().map(|n| Scope::from_capture(n)).collect()
}

/// Per-query capture→scope tables, built once.
fn scope_table(lang: &'static LanguageConfig) -> &'static [Scope] {
    static TABLES: OnceLock<std::sync::Mutex<HashMap<&'static str, &'static [Scope]>>> = OnceLock::new();
    let m = TABLES.get_or_init(Default::default);
    let mut guard = m.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(t) = guard.get(lang.id) {
        return t;
    }
    let table: &'static [Scope] = match lang.highlight_query() {
        Some(q) => Box::leak(scopes_for(q).into_boxed_slice()),
        None => &[],
    };
    guard.insert(lang.id, table);
    table
}

/// Parsed injection trees, valid for one document version.
#[derive(Default)]
pub struct InjectionCache {
    version: u64,
    trees: HashMap<(&'static str, usize, usize), Tree>,
}

impl InjectionCache {
    fn for_version(&mut self, version: u64) {
        if self.version != version {
            self.trees.clear();
            self.version = version;
        }
        if self.trees.len() > 256 {
            self.trees.clear();
        }
    }
}

const MAX_DEPTH: usize = 2;

/// Highlight `range` (bytes) of the document.
pub fn highlight(
    lang: &'static LanguageConfig,
    tree: &Tree,
    rope: &Rope,
    range: Range<usize>,
    version: u64,
    cache: &mut InjectionCache,
) -> Vec<Span> {
    let range = range.start.min(rope.len_bytes())..range.end.min(rope.len_bytes());
    if range.is_empty() {
        return Vec::new();
    }
    cache.for_version(version);
    let mut paint = vec![0u8; range.len()];
    paint_tree(lang, tree, rope, &range, &mut paint, cache, 0);
    // Compress into spans.
    let mut spans = Vec::new();
    let mut i = 0;
    while i < paint.len() {
        let v = paint[i];
        let mut j = i + 1;
        while j < paint.len() && paint[j] == v {
            j += 1;
        }
        if v != 0 {
            spans.push(Span { start: range.start + i, end: range.start + j, scope: Scope::from_u8(v) });
        }
        i = j;
    }
    spans
}

fn paint_tree(
    lang: &'static LanguageConfig,
    tree: &Tree,
    rope: &Rope,
    range: &Range<usize>,
    paint: &mut [u8],
    cache: &mut InjectionCache,
    depth: usize,
) {
    if let Some(q) = lang.highlight_query() {
        let table = scope_table(lang);
        let mut cursor = QueryCursor::new();
        cursor.set_byte_range(range.clone());
        let mut caps: Vec<(usize, usize, usize, u8)> = Vec::new();
        let mut it = cursor.captures(q, tree.root_node(), RopeText(rope));
        while let Some((m, idx)) = it.next() {
            let c = m.captures()[*idx];
            let scope = table.get(c.index as usize).copied().unwrap_or(Scope::None);
            if scope == Scope::None {
                continue;
            }
            let r = c.node.byte_range();
            caps.push((r.start, r.end, m.pattern_index, scope as u8));
        }
        // Outer nodes first; for the same node, later patterns are painted
        // last and win.
        caps.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)).then(a.2.cmp(&b.2)));
        for (s, e, _, v) in caps {
            let s = s.max(range.start) - range.start;
            let e = e.min(range.end).saturating_sub(range.start);
            if s < e {
                paint[s..e].fill(v);
            }
        }
    }
    if depth >= MAX_DEPTH {
        return;
    }
    let Some(iq) = lang.injection_query() else { return };
    let content_idx = iq.capture_index_for_name("injection.content");
    let lang_idx = iq.capture_index_for_name("injection.language");
    let mut cursor = QueryCursor::new();
    cursor.set_byte_range(range.clone());
    let mut injections: Vec<(&'static LanguageConfig, usize, usize, Point, Point)> = Vec::new();
    let mut it = cursor.matches(iq, tree.root_node(), RopeText(rope));
    while let Some(m) = it.next() {
        let mut name: Option<String> = None;
        for p in iq.property_settings(m.pattern_index) {
            if &*p.key == "injection.language"
                && let Some(v) = &p.value
            {
                name = Some(v.to_string());
            }
        }
        let mut content = None;
        for c in m.captures() {
            if Some(c.index) == lang_idx && name.is_none() {
                let r = c.node.byte_range();
                name = Some(rope.byte_slice(r.start..r.end.min(rope.len_bytes())).to_string());
            } else if Some(c.index) == content_idx {
                content = Some(c.node);
            }
        }
        let (Some(name), Some(node)) = (name, content) else { continue };
        let Some(inner) = by_injection_name(&name) else { continue };
        let r = node.byte_range();
        injections.push((inner, r.start, r.end, node.start_position(), node.end_position()));
    }
    for (inner, start, end, sp, ep) in injections {
        if end <= range.start || start >= range.end || inner.id == lang.id && depth > 0 {
            continue;
        }
        let key = (inner.id, start, end);
        let tree = match cache.trees.get(&key) {
            Some(t) => t.clone(),
            None => {
                let Some(t) = parse_injection(inner, rope, start, end, sp, ep) else { continue };
                cache.trees.insert(key, t.clone());
                t
            }
        };
        // The inner grammar paints over the parent only where it has
        // captures (so e.g. Markdown heading colours survive).
        paint_tree(inner, &tree, rope, range, paint, cache, depth + 1);
    }
}

/// Parse one injected region (Tree clones are reference-counted).
fn parse_injection(
    inner: &'static LanguageConfig,
    rope: &Rope,
    start: usize,
    end: usize,
    sp: Point,
    ep: Point,
) -> Option<Tree> {
    let mut parser = Parser::new();
    parser.set_language(inner.language()?).ok()?;
    let r = tree_sitter::Range { start_byte: start, end_byte: end, start_point: sp, end_point: ep };
    parser.set_included_ranges(&[r]).ok()?;
    parser.parse_with_options(
        &mut |byte, _| {
            if byte >= rope.len_bytes() {
                return &[][..];
            }
            let (chunk, cb, _, _) = rope.chunk_at_byte(byte);
            &chunk.as_bytes()[byte - cb..]
        },
        None,
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{language::by_id, syntax::Syntax};

    fn spans(lang: &str, src: &str) -> (Rope, Vec<Span>) {
        let rope = Rope::from_str(src);
        let l = by_id(lang).unwrap();
        let mut s = Syntax::new(l).unwrap();
        s.parse(&rope, 1, None);
        let sp = highlight(l, s.tree().unwrap(), &rope, 0..rope.len_bytes(), 1, &mut InjectionCache::default());
        (rope, sp)
    }

    fn scope_of(rope: &Rope, spans: &[Span], needle: &str) -> Scope {
        let text = rope.to_string();
        let at = text.find(needle).unwrap_or_else(|| panic!("{needle} not in source"));
        spans.iter().find(|s| s.start <= at && at < s.end).map(|s| s.scope).unwrap_or(Scope::None)
    }

    #[test]
    fn rust_scopes() {
        let (r, s) = spans("rust", "// hi\nfn main() { let s = \"x\"; println!(\"{}\", 42); }\n");
        assert_eq!(scope_of(&r, &s, "// hi"), Scope::Comment);
        assert_eq!(scope_of(&r, &s, "fn"), Scope::Keyword);
        assert_eq!(scope_of(&r, &s, "main"), Scope::Function);
        assert_eq!(scope_of(&r, &s, "\"x\""), Scope::String);
        assert!(matches!(scope_of(&r, &s, "42"), Scope::Number | Scope::ConstantBuiltin));
        assert_eq!(scope_of(&r, &s, "println"), Scope::Macro);
    }

    #[test]
    fn typescript_and_tsx_scopes() {
        let (r, s) =
            spans("typescript", "import { a } from './a';\nexport function f(x: number): string { return `${x}`; }\n");
        assert_eq!(scope_of(&r, &s, "import"), Scope::Keyword);
        assert_eq!(scope_of(&r, &s, "f(x"), Scope::Function);
        assert_eq!(scope_of(&r, &s, "number"), Scope::TypeBuiltin);
        assert_eq!(scope_of(&r, &s, "'./a'"), Scope::String);
        let (r, s) = spans("typescriptreact", "const el = <Button kind=\"x\">hi</Button>;\n");
        assert_eq!(scope_of(&r, &s, "const"), Scope::Keyword);
        assert!(
            matches!(scope_of(&r, &s, "Button"), Scope::Tag | Scope::Constructor | Scope::Type),
            "{:?}",
            scope_of(&r, &s, "Button")
        );
        assert_eq!(scope_of(&r, &s, "kind"), Scope::Attribute);
    }

    #[test]
    fn python_and_markdown_with_injection() {
        let (r, s) = spans("python", "def f(a):\n    return None  # c\n");
        assert_eq!(scope_of(&r, &s, "def"), Scope::Keyword);
        assert_eq!(scope_of(&r, &s, "# c"), Scope::Comment);
        let md = "# Title\n\nSome *em* text.\n\n```rust\nfn x() {}\n```\n";
        let (r, s) = spans("markdown", md);
        assert_eq!(scope_of(&r, &s, "Title"), Scope::Heading);
        assert_eq!(scope_of(&r, &s, "fn x"), Scope::Keyword, "code fence highlighted as Rust");
    }

    #[test]
    fn viewport_range_only() {
        let src = "fn a() {}\n".repeat(1000);
        let rope = Rope::from_str(&src);
        let l = by_id("rust").unwrap();
        let mut s = Syntax::new(l).unwrap();
        s.parse(&rope, 1, None);
        let start = rope.line_to_byte(500);
        let end = rope.line_to_byte(510);
        let sp = highlight(l, s.tree().unwrap(), &rope, start..end, 1, &mut InjectionCache::default());
        assert!(!sp.is_empty());
        assert!(sp.iter().all(|x| x.start >= start && x.end <= end));
    }

    #[test]
    fn capture_name_prefixes() {
        assert_eq!(Scope::from_capture("function.method.call"), Scope::FunctionMethod);
        assert_eq!(Scope::from_capture("keyword.control.flow"), Scope::KeywordControl);
        assert_eq!(Scope::from_capture("variable"), Scope::Variable);
        assert_eq!(Scope::from_capture("nonsense"), Scope::None);
    }
}
