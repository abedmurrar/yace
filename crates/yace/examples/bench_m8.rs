//! M8 micro-benchmarks (`cargo run --release --example bench_m8`, after
//! `cargo build --release -p yace-plugin-hello` and
//! `scripts/build_grammars.py`): the cost of calling into a plugin, of a
//! plugin calling the editor, and of loading a packaged grammar.

use std::{
    path::Path,
    time::{Duration, Instant},
};

use yace_plugin::testing::MockHost;
use yace_plugin_host::Loaded;

fn per_call(n: u32, mut f: impl FnMut()) -> Duration {
    for _ in 0..1000 {
        f();
    }
    let t = Instant::now();
    for _ in 0..n {
        f();
    }
    t.elapsed() / n
}

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let lib = root.join("target/release/libyace_plugin_hello.so");
    let mock = MockHost::new();
    mock.with(|s| {
        s.settings.insert("greeting".into(), serde_json::json!("Hi"));
        s.settings.insert("exclaim".into(), serde_json::json!(true));
    });
    // The mock's table is copied into a box the plugin keeps.
    // SAFETY-free: the table is plain data (function pointers).
    let host = Box::new(unsafe_copy(mock.raw()));
    let (mut p, load) = Loaded::open(&lib, host, false).expect("build yace-plugin-hello first");
    println!("load + init: {:.2} ms", load.as_secs_f64() * 1e3);
    let n = 200_000;
    let d = per_call(n, || {
        p.command("unknown.command", "");
    });
    println!("editor → plugin call (unhandled command, timed): {:.0} ns", d.as_nanos());
    let d = per_call(n, || {
        p.event(r#"{"type":"selectionChanged","doc":1,"anchor":3,"head":3}"#);
    });
    println!("event delivery (JSON parsed by the SDK): {:.0} ns", d.as_nanos());
    let d = per_call(50_000, || {
        p.command("hello.greet", "");
    });
    println!("command making 3 host calls (settings ×2, roots; JSON both ways): {:.2} µs", d.as_secs_f64() * 1e6);
    p.shutdown();

    let pkg = root.join("dist/yace-grammars");
    let cpp = pkg.join("grammars/cpp.so");
    if cpp.exists() {
        let t = Instant::now();
        let sha = yace_plugin_host::store::sha256_file(&cpp).unwrap();
        let hash = t.elapsed();
        let t = Instant::now();
        let f = yace_plugin_host::open_grammar(&cpp, "tree_sitter_cpp").unwrap();
        let open = t.elapsed();
        let t = Instant::now();
        let lang = yace_app::plugins::grammar_language(f);
        let q = |f: &str| std::fs::read_to_string(pkg.join("queries/cpp").join(f)).unwrap();
        yace_syntax::register_grammar("cpp", lang, &(q("c-highlights.scm") + &q("cpp-highlights.scm")), "").unwrap();
        let reg = t.elapsed();
        println!(
            "C++ grammar ({:.1} MB): verify hash {:.1} ms, dlopen {:.2} ms, ABI check + compile queries {:.1} ms (sha256 {}…)",
            std::fs::metadata(&cpp).unwrap().len() as f64 / 1e6,
            hash.as_secs_f64() * 1e3,
            open.as_secs_f64() * 1e3,
            reg.as_secs_f64() * 1e3,
            &sha[..8]
        );
    }
}

fn unsafe_copy(t: *const yace_plugin::abi::YaceHost) -> yace_plugin::abi::YaceHost {
    // The host table holds only function pointers and a context pointer.
    let t = unsafe_ref(t);
    yace_plugin::abi::YaceHost { ..*t }
}

#[allow(unsafe_code)]
fn unsafe_ref<'a>(t: *const yace_plugin::abi::YaceHost) -> &'a yace_plugin::abi::YaceHost {
    // SAFETY: `MockHost::raw` points to a live table for the whole run.
    unsafe { &*t }
}
