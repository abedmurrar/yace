//! Buffer benchmarks (spec §9.2, §23.7): realistic edits on a 10 MB file,
//! many-cursor typing and undo of large transactions.

use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use yace_buffer::{
    Change, ChangeSet, Document, EditKind, Range, Selection,
    ops::{self, EditOptions},
};

fn big_text() -> String {
    let line = "    let value = compute(alpha, beta, gamma); // a fairly typical line of code\n";
    line.repeat(10 * 1024 * 1024 / line.len())
}

fn bench(c: &mut Criterion) {
    let text = big_text();
    let opts = EditOptions::default();

    for (name, frac) in [("start", 0.0), ("middle", 0.5), ("end", 1.0)] {
        c.bench_function(&format!("insert_char_10mb_{name}"), |b| {
            let mut doc = Document::from_text(&text);
            let pos = ((doc.len_chars() as f64) * frac) as usize;
            let sel = Selection::point(pos);
            b.iter(|| {
                let e = ops::type_char(doc.rope(), &sel, 'x', &opts);
                let applied = doc.apply(e.changes, &sel, e.selection, EditKind::Insert);
                black_box(applied.selection);
                doc.undo();
            });
        });
    }

    c.bench_function("type_char_1000_cursors", |b| {
        let mut doc = Document::from_text(&text[..1_000_000]);
        let lines = doc.rope().len_lines().min(1000);
        let sel = Selection::new((0..lines).map(|l| Range::point(doc.rope().line_to_char(l))), 0);
        b.iter(|| {
            let e = ops::type_char(doc.rope(), &sel, 'x', &opts);
            doc.apply(e.changes, &sel, e.selection, EditKind::Other);
            doc.undo();
        });
    });

    c.bench_function("undo_redo_100k_changes", |b| {
        let mut doc = Document::from_text(&text);
        let len = doc.len_chars();
        let changes: Vec<Change> = (0..100_000).map(|i| Change::insert(i * 50, "y")).collect();
        let cs = ChangeSet::new(changes, len);
        let sel = Selection::point(0);
        doc.apply(cs, &sel, sel.clone(), EditKind::Other);
        b.iter(|| {
            doc.undo();
            doc.redo();
        });
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
