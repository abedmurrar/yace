//! M6 micro-benchmarks (run with `cargo run --release --example bench_m6`):
//! theme load/switch cost per built-in theme, and keybinding-editor
//! filtering per keystroke over the full default keymap.

use std::time::{Duration, Instant};

use yace_app::{
    keybindings_editor::{KbRow, KeybindingEditor},
    theme::{BUILTIN, Catalog, Overrides, load_theme},
};
use yace_keymap::{KeyCode, KeyPress, Mods};

fn median(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v[v.len() / 2]
}

fn main() {
    let cat = Catalog::builtin();
    println!("theme load + colour-depth conversion (median of 300):");
    for (id, name, _, _) in BUILTIN {
        let times: Vec<Duration> = (0..300)
            .map(|_| {
                let t = Instant::now();
                let (theme, _) = load_theme(&cat, id, &Overrides::default()).unwrap();
                let theme = theme.for_depth(yace_app::theme::Depth::Ansi256);
                std::hint::black_box(theme);
                t.elapsed()
            })
            .collect();
        println!("  {name:<18} {:>7.1} µs", median(times).as_secs_f64() * 1e6);
    }

    let km = yace_keymap::default_keymap();
    let rows: Vec<KbRow> = km
        .bindings()
        .iter()
        .map(|b| KbRow {
            command: b.command.clone(),
            title: b.command.replace('.', " "),
            keys: Some(b.keys.clone()),
            when: b.when.to_string(),
            source: Some(b.source),
            conflict: false,
        })
        .collect();
    let n = rows.len();
    let mut worst = Duration::ZERO;
    let mut all = Vec::new();
    for _ in 0..50 {
        let mut e = KeybindingEditor::new(rows.clone());
        for c in "toggle side bar".chars() {
            let kp = if c == ' ' {
                KeyPress::new(KeyCode::Space, Mods::NONE)
            } else {
                KeyPress::new(KeyCode::Char(c), Mods::NONE)
            };
            let t = Instant::now();
            e.handle_key(kp, Some(c));
            let d = t.elapsed();
            worst = worst.max(d);
            all.push(d);
        }
    }
    println!(
        "keybinding editor filter over {n} rows: median {:.1} µs, worst {:.1} µs per keystroke",
        median(all).as_secs_f64() * 1e6,
        worst.as_secs_f64() * 1e6
    );
}
