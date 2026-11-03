//! Syntax benchmarks (spec §12.1, §23.7): full parse, incremental reparse
//! after a keystroke, and viewport highlighting, on a 10k-line TypeScript file.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use ropey::Rope;
use yace_syntax::{InjectionCache, Syntax, by_id, highlight, input_edits};

fn ts_source(lines: usize) -> String {
    let block = r#"import { Injectable } from '@nestjs/common';
/** Service for jobs. */
@Injectable()
export class JobsService<T extends { id: string }> {
  private readonly items: Map<string, T> = new Map();
  constructor(private readonly repo: Repository<T>) {}
  async findAll(limit = 10): Promise<T[]> {
    const rows = await this.repo.find({ take: limit, order: { createdAt: 'DESC' } });
    return rows.filter((r) => r.id !== `skip-${limit}`).map((r) => ({ ...r }));
  }
}
"#;
    let n = block.lines().count();
    block.repeat(lines / n + 1)
}

fn bench(c: &mut Criterion) {
    let lang = by_id("typescript").unwrap();
    let src = ts_source(10_000);
    let rope = Rope::from_str(&src);
    c.bench_function("ts_full_parse_10k_lines", |b| {
        b.iter(|| {
            let mut s = Syntax::new(lang).unwrap();
            black_box(s.parse(&rope, 1, None));
        })
    });
    let mut s = Syntax::new(lang).unwrap();
    s.parse(&rope, 1, None);
    let at = rope.line_to_char(5_000) + 10;
    c.bench_function("ts_incremental_reparse_keystroke", |b| {
        b.iter_batched(
            || {
                let mut r = rope.clone();
                let before = r.clone();
                r.insert(at, "x");
                let mut syn = Syntax::new(lang).unwrap();
                syn.set_tree(s.tree().unwrap().clone(), 1);
                syn.edit(&input_edits(&before, &[(at, at, "x")]));
                (syn, r)
            },
            |(mut syn, r)| black_box(syn.parse(&r, 2, None)),
            criterion::BatchSize::SmallInput,
        )
    });
    let start = rope.line_to_byte(5_000);
    let end = rope.line_to_byte(5_060);
    c.bench_function("ts_highlight_60_line_viewport", |b| {
        let mut cache = InjectionCache::default();
        b.iter(|| black_box(highlight(lang, s.tree().unwrap(), &rope, start..end, 1, &mut cache)))
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
