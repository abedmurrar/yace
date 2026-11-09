//! Regenerate the documentation site's screenshots and recordings.
//!
//! Drives the real release binary in a pseudo-terminal (120 × 34) with
//! scripted keys, inside a demo workspace under a temporary home, and
//! writes SVG screenshots, animated SVG recordings and their asciicast v2
//! sources to `site/src/media/`. See `site/README.md`.
//!
//! ```sh
//! cargo build --release -p yace -p yace-plugin-tasks
//! cargo run --release --manifest-path site/capture/Cargo.toml            # everything
//! cargo run --release --manifest-path site/capture/Cargo.toml -- quick-open source-control
//! cargo run --release --manifest-path site/capture/Cargo.toml -- --explore 'ctrl+p' 'type:cart' dump
//! ```

mod demo;
mod render;
mod scenes;
mod session;

use std::{
    fs,
    path::{Path, PathBuf},
};

use render::Defaults;
use session::Session;

pub const ROWS: usize = 34;
pub const COLS: usize = 120;

pub const DARK: Defaults = Defaults { fg: (0xcc, 0xcc, 0xcc), bg: (0x1e, 0x1e, 0x1e) };
pub const LIGHT: Defaults = Defaults { fg: (0x3b, 0x3b, 0x3b), bg: (0xff, 0xff, 0xff) };

/// One capture: how to start YACE and what to do.
pub struct Scene {
    pub name: &'static str,
    /// Extra lines for the `[workbench]` section of `config.toml`.
    pub workbench: &'static str,
    /// Extra lines for the `[terminal]` section.
    pub terminal: &'static str,
    pub light: bool,
    /// What to open, relative to the demo's `projects` folder (the
    /// workspace file when empty), plus `--goto` targets.
    pub open: &'static [&'static str],
    pub steps: &'static [&'static str],
}

struct Paths {
    root: PathBuf,
    yace: PathBuf,
    out: PathBuf,
}

/// The bytes a legacy terminal sends for a key, e.g. `ctrl+p`, `alt+shift+f`,
/// `f1`, `enter`.
fn key_bytes(k: &str) -> Vec<u8> {
    let named = |n: &str| -> Option<&'static [u8]> {
        Some(match n {
            "enter" => b"\r",
            "esc" => b"\x1b",
            "tab" => b"\t",
            "shift+tab" => b"\x1b[Z",
            "backspace" => b"\x7f",
            "space" => b" ",
            "up" => b"\x1b[A",
            "down" => b"\x1b[B",
            "right" => b"\x1b[C",
            "left" => b"\x1b[D",
            "home" => b"\x1b[H",
            "end" => b"\x1b[F",
            "pgup" => b"\x1b[5~",
            "pgdn" => b"\x1b[6~",
            "delete" => b"\x1b[3~",
            "f1" => b"\x1bOP",
            "f2" => b"\x1bOQ",
            "f3" => b"\x1bOR",
            "f4" => b"\x1bOS",
            "f5" => b"\x1b[15~",
            "f6" => b"\x1b[17~",
            "f8" => b"\x1b[19~",
            "f12" => b"\x1b[24~",
            "shift+f3" => b"\x1b[1;2R",
            "alt+up" => b"\x1b[1;3A",
            "alt+down" => b"\x1b[1;3B",
            "alt+right" => b"\x1b[1;3C",
            "alt+left" => b"\x1b[1;3D",
            "ctrl+up" => b"\x1b[1;5A",
            "ctrl+down" => b"\x1b[1;5B",
            "ctrl+shift+up" => b"\x1b[1;6A",
            "ctrl+shift+down" => b"\x1b[1;6B",
            "ctrl+`" | "ctrl+space" => b"\x00",
            "ctrl+\\" => b"\x1c",
            "ctrl+/" => b"\x1f",
            "ctrl+]" => b"\x1d",
            _ => return None,
        })
    };
    if let Some(b) = named(k) {
        return b.to_vec();
    }
    if let Some(rest) = k.strip_prefix("ctrl+")
        && rest.len() == 1
    {
        return vec![rest.as_bytes()[0].to_ascii_lowercase() & 0x1f];
    }
    if let Some(rest) = k.strip_prefix("alt+shift+")
        && rest.len() == 1
    {
        return vec![0x1b, rest.as_bytes()[0].to_ascii_uppercase()];
    }
    if let Some(rest) = k.strip_prefix("alt+")
        && rest.chars().count() == 1
    {
        let mut v = vec![0x1b];
        v.extend_from_slice(rest.as_bytes());
        return v;
    }
    panic!("unknown key {k:?}");
}

fn run_scene(p: &Paths, scene: &Scene, explore: Option<&[String]>) {
    let demo = demo::Demo::create();
    demo.write_config(scene.workbench, scene.terminal);
    demo.install_tasks_plugin(
        &p.yace,
        &p.root.join("plugins/tasks/yace-plugin.toml"),
        &p.root.join("target/release/libyace_plugin_tasks.so"),
    );
    let projects = demo.workspace.parent().unwrap().to_path_buf();
    let mut args: Vec<String> = Vec::new();
    if scene.open.is_empty() {
        args.push(demo.workspace.display().to_string());
    }
    for o in scene.open {
        if let Some(goto) = o.strip_prefix("goto:") {
            args.push("--goto".into());
            args.push(projects.join(goto).display().to_string());
        } else {
            args.push(projects.join(o).display().to_string());
        }
    }
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let defaults = if scene.light { LIGHT } else { DARK };
    let mut s = Session::spawn(&p.yace, &arg_refs, &demo.env(), &projects, ROWS, COLS, defaults);
    s.wait("EXPLORER");
    let owned: Vec<String>;
    let steps: Vec<&str> = match explore {
        Some(e) => {
            owned = e.to_vec();
            owned.iter().map(String::as_str).collect()
        }
        None => scene.steps.to_vec(),
    };
    for step in steps {
        let (op, arg) = step.split_once(':').unwrap_or((step, ""));
        match op {
            "type" => s.typ(arg),
            "wait" => s.wait(arg),
            "gone" => s.wait_gone(arg),
            "sleep" => s.pause(arg.parse().unwrap()),
            "settle" => s.settle(),
            "dump" => println!("{}", s.text()),
            "rec" => s.record_start(),
            "shot" => {
                let (name, title) = arg.split_once(':').unwrap_or((arg, arg));
                s.settle();
                let svg = render::svg_still(&s.frame(), defaults, title);
                write_media(&p.out, &format!("{name}.svg"), &svg);
            }
            "stop" => {
                let (name, title) = arg.split_once(':').unwrap_or((arg, arg));
                let rec = s.record_stop(title);
                write_media(&p.out, &format!("{name}.cast"), &rec.cast);
                let svg = render::svg_animated(&rec.frames, defaults, title);
                write_media(&p.out, &format!("{name}.anim.svg"), &svg);
                println!("  {name}: {} frames", rec.frames.len());
            }
            "shell" => {
                // A command typed into the integrated terminal.
                s.typ(arg);
                s.key(b"\r");
            }
            _ => s.key(&key_bytes(step)),
        }
    }
    s.quit();
}

fn write_media(dir: &Path, name: &str, text: &str) {
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join(name), text).unwrap();
    println!("  wrote {name} ({:.1} KiB)", text.len() as f64 / 1024.0);
}

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let mut p = Paths { yace: root.join("target/release/yace"), out: root.join("site/src/media"), root };
    let mut only: Vec<String> = Vec::new();
    let mut explore: Option<Vec<String>> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--yace" => p.yace = PathBuf::from(args.next().expect("--yace PATH")),
            "--out" => p.out = PathBuf::from(args.next().expect("--out DIR")),
            "--explore" => explore = Some(args.by_ref().collect()),
            _ => only.push(a),
        }
    }
    assert!(p.yace.exists(), "{} not found: cargo build --release -p yace -p yace-plugin-tasks", p.yace.display());
    if let Some(steps) = explore {
        let scene = Scene { name: "explore", workbench: "", terminal: "", light: false, open: &[], steps: &[] };
        run_scene(&p, &scene, Some(&steps));
        return;
    }
    for scene in scenes::SCENES {
        if only.is_empty() || only.iter().any(|o| o == scene.name) {
            println!("{}", scene.name);
            run_scene(&p, scene, None);
        }
    }
}
