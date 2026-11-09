//! Completion filtering over a TypeScript-sized list (13,425 items): the
//! first keystroke scans every item; later ones only what still matched.
//! `cargo run --release --example bench_completion` (`OLD=1`: the ranking
//! before v0.11.0, for comparison under the same load).

use std::time::{Duration, Instant};

use yace_lsp::CompletionItem;

const PARTS: &[&str] = &[
    "abort",
    "add",
    "event",
    "listener",
    "array",
    "buffer",
    "request",
    "response",
    "element",
    "html",
    "node",
    "child",
    "list",
    "map",
    "set",
    "promise",
    "async",
    "iterator",
    "readable",
    "stream",
    "text",
    "decoder",
    "encoder",
    "url",
    "search",
    "params",
    "worker",
    "message",
    "channel",
    "port",
    "storage",
    "local",
    "session",
    "window",
    "document",
    "fragment",
    "range",
    "selection",
    "style",
    "sheet",
    "rule",
    "media",
    "query",
    "canvas",
    "context",
    "image",
    "data",
    "blob",
    "file",
    "reader",
    "form",
    "input",
    "keyboard",
    "mouse",
    "pointer",
    "touch",
    "animation",
    "frame",
    "timeout",
    "interval",
    "performance",
    "observer",
    "mutation",
    "resize",
    "intersection",
    "crypto",
    "subtle",
    "key",
    "error",
    "type",
    "syntax",
    "reference",
    "date",
    "math",
    "number",
    "string",
    "symbol",
    "object",
    "function",
    "proxy",
    "reflect",
    "weak",
    "shared",
    "atomics",
    "json",
    "intl",
    "collator",
    "format",
];

fn items() -> Vec<CompletionItem> {
    let cap = |s: &str| s[..1].to_uppercase() + &s[1..];
    let mut out = Vec::with_capacity(13_425);
    let mut i = 0usize;
    while out.len() < 13_425 {
        let a = PARTS[i % PARTS.len()];
        let b = PARTS[(i / PARTS.len()) % PARTS.len()];
        let c = PARTS[(i * 7 / 3) % PARTS.len()];
        let label = match i % 3 {
            0 => format!("{}{}", cap(a), cap(b)),
            1 => format!("{a}{}{}", cap(b), cap(c)),
            _ => format!("{}{}{}", cap(a), cap(b), cap(c)),
        };
        let item: CompletionItem = serde_json::from_value(serde_json::json!({
            "label": label,
            "kind": 6,
            "sortText": format!("{:05}", i % 997),
        }))
        .unwrap();
        out.push(item);
        i += 1;
    }
    out
}

/// "min / median / p95" in ms (min and median hold up when the machine
/// is busy).
fn p95(mut v: Vec<Duration>) -> String {
    v.sort();
    let ms = |d: Duration| d.as_secs_f64() * 1e3;
    format!("{:.2} / {:.2} / {:.2}", ms(v[0]), ms(v[v.len() / 2]), ms(v[v.len() * 95 / 100]))
}

/// The ranking before this change (string tie-break sort, full scorer for
/// one letter), for a side-by-side comparison under the same load.
fn old_rank(items: &[CompletionItem], candidates: &[usize], typed: &str) -> Vec<usize> {
    let q = yace_search::fuzzy::Query::new(typed);
    let first = typed.chars().next().map(|ch| ch.to_ascii_lowercase());
    let mut scored: Vec<(i32, usize)> = candidates
        .iter()
        .filter_map(|&i| {
            let key = &items[i].label;
            if let Some(f) = first
                && !key.chars().any(|k| k.to_ascii_lowercase() == f)
            {
                return None;
            }
            // Two tokens force the general scorer (no one-letter shortcut).
            let m = yace_search::fuzzy::match_label(&q, key)?;
            Some((m.score, i))
        })
        .collect();
    let sort_key = |i: usize| -> &str { items[i].sort_text.as_deref().unwrap_or(&items[i].label) };
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| sort_key(a.1).cmp(sort_key(b.1))));
    scored.into_iter().map(|(_, i)| i).collect()
}

fn main() {
    let mut items = items();
    // Lists arrive sorted by sortText (the reader thread sorts them).
    items.sort_by(|a, b| a.sort_text.cmp(&b.sort_text));
    if std::env::var_os("OLD").is_some() {
        let all: Vec<usize> = (0..items.len()).collect();
        let (mut first, mut next) = (Vec::new(), Vec::new());
        for round in 0..200 {
            let letter = ["r", "e", "s", "p", "a", "d"][round % 6];
            let t = Instant::now();
            let shown = old_rank(&items, &all, letter);
            first.push(t.elapsed());
            let t = Instant::now();
            std::hint::black_box(old_rank(&items, &shown, &format!("{letter}e")));
            next.push(t.elapsed());
        }
        println!("before, ms min / median / p95: first keystroke {}, next keystroke {}", p95(first), p95(next));
        return;
    }
    let all: Vec<usize> = (0..items.len()).collect();
    let mut first = Vec::new();
    let mut next = Vec::new();
    let mut back = Vec::new();
    for round in 0..200 {
        let letter = ["r", "e", "s", "p", "a", "d"][round % 6];
        let t = Instant::now();
        let shown = yace_app::app::rank_completions(&items, &all, letter);
        first.push(t.elapsed());
        let two = format!("{letter}e");
        let t = Instant::now();
        let narrowed = yace_app::app::rank_completions(&items, &shown, &two);
        next.push(t.elapsed());
        std::hint::black_box(narrowed);
        // Backspace to one letter again: a full scan.
        let t = Instant::now();
        std::hint::black_box(yace_app::app::rank_completions(&items, &all, letter));
        back.push(t.elapsed());
    }
    println!(
        "{} items, ms min / median / p95: first keystroke {}, next keystroke {}, backspace {}",
        items.len(),
        p95(first),
        p95(next),
        p95(back)
    );
}
