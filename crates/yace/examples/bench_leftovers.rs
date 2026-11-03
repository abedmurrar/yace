//! Costs of the v0.8.0 additions: file-type icon lookup (per drawn row),
//! VS Code copy naming, and a background folder copy.
//! `cargo run --release --example bench_leftovers`

use std::{hint::black_box, time::Instant};

fn main() {
    let names = [
        "main.rs", "index.ts", "App.tsx", "package.json", "README.md", "Dockerfile", ".gitignore",
        "styles.module.scss", "notes.unknownext", "Makefile", "archive.tar.gz", "photo.JPEG",
    ];
    let n = 1_000_000;
    let t = Instant::now();
    for i in 0..n {
        black_box(yace_app::icons::file_type(black_box(names[i % names.len()])));
    }
    let per = t.elapsed().as_nanos() as f64 / n as f64;
    println!("icon lookup: {per:.1} ns ({:.1} µs for a 60-row Explorer)", per * 60.0 / 1000.0);

    let src = tempfile::tempdir().unwrap();
    let tree = src.path().join("big");
    for d in 0..100 {
        let dir = tree.join(format!("dir{d}"));
        std::fs::create_dir_all(&dir).unwrap();
        for f in 0..100 {
            std::fs::write(dir.join(format!("file{f}.ts")), "export const x = 1;\n").unwrap();
        }
    }
    let t = Instant::now();
    let to = yace_app::app::explorer_ops::unique_destination(src.path(), "big");
    let naming = t.elapsed();
    let t = Instant::now();
    let r = yace_app::app::explorer_ops::copy_paths(vec![(tree, to.clone())]);
    assert!(r.iter().all(|(_, r)| r.is_ok()));
    println!("copy name: {naming:?}; copying 10,000 files in 100 folders (on a worker thread): {:?}", t.elapsed());
}
