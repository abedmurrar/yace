//! `yace` — entry point.

use std::{io::IsTerminal, process::ExitCode, time::Instant};

use clap::Parser;
use yace_app::{
    app::{App, AppOptions},
    cli::{self, Args},
    initial_workspace,
    run::Runtime,
    terminal,
};
use yace_config::Paths;

fn init_logging(paths: &Paths) {
    // Logging is opt-in (YACE_LOG=debug); file contents are never logged.
    let Ok(filter) = std::env::var("YACE_LOG") else { return };
    let _ = std::fs::create_dir_all(&paths.state_dir);
    let Ok(file) = std::fs::OpenOptions::new().create(true).append(true).open(paths.log_file()) else { return };
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(filter))
        .with_writer(std::sync::Mutex::new(file))
        .with_ansi(false)
        .try_init();
}

fn main() -> ExitCode {
    let process_start = Instant::now();
    yace_app::mem::limit_arenas();
    let args = Args::parse();
    if args.install_plugin.is_some() || args.list_plugins || args.package_plugin.is_some() {
        return yace_app::plugin_cli::run(&args, &Paths::discover());
    }
    let launch = match cli::plan(&args) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("yace: {e}");
            return ExitCode::from(2);
        }
    };
    if !std::io::stdout().is_terminal() || !std::io::stdin().is_terminal() {
        eprintln!("yace: standard input and output must be a terminal");
        return ExitCode::from(2);
    }
    let paths = Paths::discover();
    init_logging(&paths);

    let mut app = App::new(AppOptions {
        paths,
        system_clipboard: true,
        keyboard_enhanced: false,
        process_start,
        safe_mode: args.safe_mode,
        terminal_depth: yace_app::theme::terminal_depth(),
    });

    terminal::install_panic_hook();
    let want_enhanced = app.settings.keyboard.enhanced_protocol;
    let (mut term, enhanced) = match terminal::enter(want_enhanced) {
        Ok(t) => t,
        Err(e) => {
            terminal::restore();
            eprintln!("yace: could not initialise the terminal: {e}");
            return ExitCode::FAILURE;
        }
    };
    let _guard = terminal::Guard;
    app.keyboard_enhanced = enhanced;

    let mut runtime = Runtime::new(&mut app);
    initial_workspace(&mut app, &launch);
    runtime.spawn_input_thread();
    runtime.spawn_signal_thread();

    let result = runtime.run(&mut app, &mut term, args.startup_time);
    app.shutdown_plugins();
    terminal::restore();
    // Benchmarks: YACE_METRICS_FILE=<path> writes latency samples at exit.
    if let Some(path) = std::env::var_os("YACE_METRICS_FILE") {
        let m = &app.metrics;
        let ms = |d: std::time::Duration| d.as_secs_f64() * 1000.0;
        let text = format!(
            "input_events={}\ninput_p50_ms={:.3}\ninput_p95_ms={:.3}\ninput_max_ms={:.3}\nframes={}\nrender_p50_ms={:.3}\nrender_p95_ms={:.3}\nrender_max_ms={:.3}\n",
            m.input.count,
            ms(m.input.percentile(0.5)),
            ms(m.input.percentile(0.95)),
            ms(m.input.max),
            m.frames,
            ms(m.render.percentile(0.5)),
            ms(m.render.percentile(0.95)),
            ms(m.render.max),
        );
        let _ = std::fs::write(path, text);
    }
    if args.startup_time {
        if let Some(d) = app.metrics.first_frame {
            println!("first_frame_ms={:.3}", d.as_secs_f64() * 1000.0);
        }
        if let Some(kib) = yace_app::metrics::rss_kib() {
            println!("rss_kib={kib}");
        }
    }
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("yace: {e}");
            ExitCode::FAILURE
        }
    }
}
