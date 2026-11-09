//! Costs behind the v0.9.0 search changes, on a real repository:
//! narrowing a workspace search, re-searching changed files, following
//! edits in Find, and watching every indexed folder.
//! `cargo run --release --example bench_batch2 -- <repo>`

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Mutex, atomic::AtomicBool},
    time::{Duration, Instant},
};

use notify::Watcher;
use yace_buffer::{Change, ChangeSet, Rope};
use yace_search::{ContentQuery, SearchRoot, WalkOptions, content, find};

fn median(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v[v.len() / 2]
}

fn time<T>(runs: usize, mut f: impl FnMut() -> T) -> Duration {
    median(
        (0..runs)
            .map(|_| {
                let t = Instant::now();
                std::hint::black_box(f());
                t.elapsed()
            })
            .collect(),
    )
}

fn main() {
    let repo = PathBuf::from(std::env::args().nth(1).expect("usage: bench_batch2 <repo>"));
    let roots = vec![SearchRoot { key: 1, name: "repo".into(), path: repo.clone() }];
    let none = AtomicBool::new(false);
    let overlays = HashMap::new();

    // --- Workspace search: typing "process.nextTic" → "process.nextTick".
    let short = ContentQuery { pattern: "process.nextTic".into(), max_results: 1_000_000, ..Default::default() };
    let long = ContentQuery { pattern: "process.nextTick".into(), ..short.clone() };
    let matched = Mutex::new(Vec::new());
    content::search(&roots, &short, &overlays, &none, &|f| matched.lock().unwrap().push(f.path)).unwrap();
    let matched = matched.into_inner().unwrap();
    let full = time(5, || content::search(&roots, &long, &overlays, &none, &|_| {}).unwrap());
    let narrow =
        time(5, || content::search_paths_with(&roots, &long, &matched, &overlays, true, &none, &|_, _| {}).unwrap());
    println!(
        "search narrowing: whole walk {:.1} ms → {} matched files only {:.2} ms",
        full.as_secs_f64() * 1e3,
        matched.len(),
        narrow.as_secs_f64() * 1e3
    );
    // --- Live update: re-search one saved file vs the whole search.
    let one = vec![matched.first().cloned().unwrap_or_else(|| repo.join("README.md"))];
    let update = time(20, || content::search_paths(&roots, &long, &one, &overlays, &none, &|_, _| {}).unwrap());
    println!(
        "live update of one changed file: {:.3} ms (a new search: {:.1} ms)",
        update.as_secs_f64() * 1e3,
        full.as_secs_f64() * 1e3
    );

    // --- Find in file: 100k lines, 10k matches, one keystroke.
    let text: String = (0..100_000)
        .map(
            |i| if i % 10 == 0 { format!("let value_{i} = needle({i});\n") } else { format!("let value_{i} = {i};\n") },
        )
        .collect();
    let mut rope = Rope::from_str(&text);
    let re = find::build_regex("needle", Default::default()).unwrap();
    let matches = find::find_in_text(&text, &re, None).matches;
    assert_eq!(matches.len(), 10_000);
    let before = time(20, || find::find_in_text(&rope.to_string(), &re, None));
    let at = rope.line_to_char(50_000);
    let cs = ChangeSet::new(vec![Change::replace(at, at, "x")], rope.len_chars());
    cs.apply(&mut rope);
    let after = time(200, || find::update_matches(&matches, &cs, &rope, &re, 2_000).unwrap());
    let rope_full = time(20, || find::find_in_rope(&rope, &re, None, true));
    println!(
        "find after a keystroke (100k lines, 10k matches): full copy + search {:.2} ms → edited lines {:.3} ms \
         (whole-file search without the copy: {:.2} ms)",
        before.as_secs_f64() * 1e3,
        after.as_secs_f64() * 1e3,
        rope_full.as_secs_f64() * 1e3
    );

    // --- Quick Open index: walk with folders, then watch every folder.
    let walk = WalkOptions { report_dirs: true, ..Default::default() };
    let t = Instant::now();
    let (all, _) = yace_search::walk::collect(&repo, &walk);
    let walk_time = t.elapsed();
    let dirs: Vec<&str> = all.iter().filter_map(|p| p.strip_suffix('/')).collect();
    println!(
        "index walk: {} files and {} folders in {:.0} ms",
        all.len() - dirs.len(),
        dirs.len(),
        walk_time.as_secs_f64() * 1e3
    );
    let (tx, rx) = std::sync::mpsc::channel();
    let mut w = notify::recommended_watcher(move |e| drop(tx.send(e))).unwrap();
    let t = Instant::now();
    w.watch(&repo, notify::RecursiveMode::NonRecursive).unwrap();
    for d in &dirs {
        w.watch(&repo.join(d), notify::RecursiveMode::NonRecursive).unwrap();
    }
    let watch_time = t.elapsed();
    let kernel = std::fs::read_dir("/proc/self/fdinfo")
        .map(|it| {
            it.flatten()
                .map(|e| std::fs::read_to_string(e.path()).unwrap_or_default().matches("inotify wd:").count())
                .sum::<usize>()
        })
        .unwrap_or(0);
    println!(
        "watching {} folders: {:.0} ms on the watch thread ({:.1} µs each); kernel watches: {kernel}",
        dirs.len() + 1,
        watch_time.as_secs_f64() * 1e3,
        watch_time.as_secs_f64() * 1e6 / (dirs.len() + 1) as f64
    );
    // Event to index latency: create a file and wait for its event.
    let probe = repo.join(dirs.first().copied().unwrap_or("")).join(".yace-bench-probe");
    while rx.try_recv().is_ok() {}
    let t = Instant::now();
    std::fs::write(&probe, "x").unwrap();
    let got = rx.recv_timeout(Duration::from_secs(2)).is_ok();
    let lat = t.elapsed();
    let _ = std::fs::remove_file(&probe);
    println!("file created → event: {:.2} ms (received: {got})", lat.as_secs_f64() * 1e3);
}
