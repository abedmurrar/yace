//! Search benchmarks (spec §23.7): fuzzy Quick Open over 200k paths and
//! content search throughput.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use yace_search::{FileIndex, QueryContext};

fn synthetic_paths(n: usize) -> Vec<Box<str>> {
    let dirs = ["src", "lib", "app", "components", "services", "controllers", "models", "utils", "tests", "packages"];
    let names = [
        "index",
        "jobs.service",
        "auth.controller",
        "user.model",
        "button",
        "helpers",
        "config",
        "router",
        "store",
        "api",
    ];
    let exts = ["ts", "tsx", "rs", "js", "json", "md"];
    (0..n)
        .map(|i| {
            format!(
                "{}/{}/{}/{}{}.{}",
                dirs[i % 10],
                dirs[(i / 10) % 10],
                dirs[(i / 100) % 10],
                names[(i / 7) % 10],
                i,
                exts[i % 6]
            )
            .into_boxed_str()
        })
        .collect()
}

fn bench(c: &mut Criterion) {
    let mut ix = FileIndex::new();
    ix.ensure_root(1, "big", std::path::Path::new("/big"));
    ix.add_batch(1, synthetic_paths(200_000));
    ix.finish_walk(1, false);
    let ctx = QueryContext::default();
    for q in ["jobserv", "authctrl", "index.ts", "srccompbtn"] {
        c.bench_function(&format!("quick_open_200k_{q}"), |b| b.iter(|| black_box(ix.query(q, &ctx, 100))));
    }
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    c.bench_function("walk_yace_repo", |b| {
        b.iter(|| black_box(yace_search::walk::collect(&root, &yace_search::WalkOptions::default())))
    });
    let roots = vec![yace_search::SearchRoot { key: 1, name: "yace".into(), path: root.clone() }];
    let q = yace_search::ContentQuery { pattern: "fn ".into(), exclude: vec!["target".into()], ..Default::default() };
    c.bench_function("content_search_yace_repo_fn", |b| {
        b.iter(|| {
            black_box(
                yace_search::content::search(&roots, &q, &Default::default(), &Default::default(), &|_| {}).unwrap(),
            )
        })
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
