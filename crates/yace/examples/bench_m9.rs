//! M9 micro-benchmarks (`cargo run --release --example bench_m9`): parsing
//! a large `git status`, and the gutter diff of a large file.

use std::time::{Duration, Instant};

fn median(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v[v.len() / 2]
}

fn main() {
    // 50,000 status records (modified, untracked, renamed).
    let mut out = b"# branch.oid 0123456789abcdef\0# branch.head main\0# branch.ab +1 -2\0".to_vec();
    for i in 0..50_000 {
        match i % 3 {
            0 => {
                out.extend(format!("1 .M N... 100644 100644 100644 aaa aaa src/dir{}/file{i}.ts\0", i % 97).as_bytes())
            }
            1 => out.extend(format!("? untracked/u{i}.txt\0").as_bytes()),
            _ => out.extend(format!("2 R. N... 100644 100644 100644 b b R100 new/n{i}.rs\0old/o{i}.rs\0").as_bytes()),
        }
    }
    let times: Vec<Duration> = (0..20)
        .map(|_| {
            let t = Instant::now();
            std::hint::black_box(yace_git::status::parse(&out));
            t.elapsed()
        })
        .collect();
    println!(
        "parse 50,000 status records ({:.1} MB): {:.2} ms",
        out.len() as f64 / 1e6,
        median(times).as_secs_f64() * 1e3
    );

    // Decorations for them (what the app builds per status).
    let st = yace_git::status::parse(&out);
    let t = Instant::now();
    let mut decor = std::collections::HashMap::new();
    for f in &st.files {
        decor.insert(f.path.clone(), f.kind());
    }
    println!("decoration map for 50,000 files: {:.2} ms", t.elapsed().as_secs_f64() * 1e3);

    // Gutter diff: a 10,000-line file with edits spread through it.
    let base: String = (0..10_000).map(|i| format!("    let value_{i} = compute({i}, \"text\");\n")).collect();
    let mut cur: Vec<String> = base.lines().map(str::to_string).collect();
    for k in (0..10_000).step_by(500) {
        cur[k] = format!("{} // edited", cur[k]);
    }
    cur.insert(5000, "    inserted();".into());
    cur.remove(7000);
    let cur = cur.join("\n") + "\n";
    let times: Vec<Duration> = (0..20)
        .map(|_| {
            let t = Instant::now();
            std::hint::black_box(yace_git::line_hunks(&base, &cur));
            t.elapsed()
        })
        .collect();
    println!(
        "gutter diff, 10,000 lines, {} hunks: {:.2} ms (on a worker, after a 150 ms typing pause)",
        yace_git::line_hunks(&base, &cur).len(),
        median(times).as_secs_f64() * 1e3
    );
}
