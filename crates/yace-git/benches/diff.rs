//! Line diff benchmark (spec §15.2, §23.7): the gutter change markers and
//! the diff view diff the whole file against its Git base after edits.

use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use yace_git::diff::line_hunks;

fn bench(c: &mut Criterion) {
    let base: String = (0..10_000).map(|i| format!("    let value_{i} = compute(alpha, beta, {i});\n")).collect();
    // Every 100th line changed, every 250th removed, a line added every 400.
    let current: String = base
        .lines()
        .enumerate()
        .filter(|(i, _)| i % 250 != 7)
        .flat_map(|(i, l)| {
            let line = if i % 100 == 3 { format!("{l} // edited") } else { l.to_string() };
            let extra = (i % 400 == 11).then(|| "    let added = 1;".to_string());
            std::iter::once(line).chain(extra)
        })
        .map(|l| l + "\n")
        .collect();
    c.bench_function("line_hunks_10k_lines", |b| b.iter(|| black_box(line_hunks(&base, &current))));
}

criterion_group!(benches, bench);
criterion_main!(benches);
