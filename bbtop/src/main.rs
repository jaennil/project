mod metrics;
mod procfs;
mod server;
mod ui;

use std::{
    env,
    path::PathBuf,
    process::ExitCode,
    sync::{Arc, RwLock},
    thread,
    time::{Duration, Instant},
};

use metrics::render_prometheus;
use procfs::{Collector, Snapshot};

#[derive(Clone, Debug)]
struct Config {
    listen: String,
    interval: Duration,
    process_limit: usize,
    proc_root: PathBuf,
    filesystem_root: PathBuf,
    runtime_root: PathBuf,
    tui: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen: "127.0.0.1:9099".into(),
            interval: Duration::from_secs(2),
            process_limit: 50,
            proc_root: env::var_os("BBTOP_PROC_ROOT")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/proc")),
            filesystem_root: env::var_os("BBTOP_FILESYSTEM_ROOT")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/")),
            // Where the privileged helpers publish the readings the exporter
            // cannot take itself: NVMe SMART and per-process network bytes.
            runtime_root: env::var_os("BBTOP_RUNTIME_ROOT")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/run/bbtop")),
            tui: true,
        }
    }
}

fn usage() -> &'static str {
    "bbtop [--no-tui] [--listen ADDRESS] [--interval SECONDS] [--top COUNT]\n\
\nOptions:\n\
  --no-tui           run only the Prometheus exporter\n\
  --listen ADDRESS   exporter address (default 127.0.0.1:9099)\n\
  --interval SEC     collection interval (default 2)\n\
  --top COUNT        process series exported (default 50)\n\
  -h, --help         show this help"
}

fn parse_args() -> Result<Option<Config>, String> {
    let mut config = Config::default();
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(None),
            "--no-tui" => config.tui = false,
            "--listen" => config.listen = args.next().ok_or("--listen needs a value")?,
            "--interval" => {
                let seconds: f64 = args
                    .next()
                    .ok_or("--interval needs a value")?
                    .parse()
                    .map_err(|_| "--interval must be a number")?;
                if !seconds.is_finite() || seconds < 0.2 {
                    return Err("--interval must be at least 0.2 seconds".into());
                }
                config.interval = Duration::from_secs_f64(seconds);
            }
            "--top" => {
                config.process_limit = args
                    .next()
                    .ok_or("--top needs a value")?
                    .parse()
                    .map_err(|_| "--top must be a positive integer")?;
                if config.process_limit == 0 {
                    return Err("--top must be a positive integer".into());
                }
            }
            _ => return Err(format!("unknown argument: {arg}")),
        }
    }
    Ok(Some(config))
}

fn main() -> ExitCode {
    let config = match parse_args() {
        Ok(Some(config)) => config,
        Ok(None) => {
            println!("{}", usage());
            return ExitCode::SUCCESS;
        }
        Err(error) => {
            eprintln!("bbtop: {error}\n\n{}", usage());
            return ExitCode::from(2);
        }
    };

    let initial = Snapshot::empty();
    let state = Arc::new(RwLock::new(initial));
    let server_state = Arc::clone(&state);
    let listen = config.listen.clone();
    let process_limit = config.process_limit;
    thread::spawn(move || {
        if let Err(error) = server::serve(&listen, server_state, process_limit) {
            eprintln!("bbtop exporter: {error}");
        }
    });

    let collector_state = Arc::clone(&state);
    let proc_root = config.proc_root.clone();
    let filesystem_root = config.filesystem_root.clone();
    let runtime_root = config.runtime_root.clone();
    let interval = config.interval;
    thread::spawn(move || {
        let mut collector = Collector::new(proc_root, filesystem_root, runtime_root);
        loop {
            let started = Instant::now();
            match collector.collect() {
                Ok(snapshot) => *collector_state.write().unwrap() = snapshot,
                Err(error) => eprintln!("bbtop collector: {error}"),
            }
            thread::sleep(interval.saturating_sub(started.elapsed()));
        }
    });

    if config.tui {
        if let Err(error) = ui::run(state, &config.listen, config.interval) {
            eprintln!("bbtop ui: {error}");
            return ExitCode::FAILURE;
        }
    } else {
        println!(
            "bbtop exporter listening on http://{}/metrics",
            config.listen
        );
        loop {
            thread::park();
        }
    }
    ExitCode::SUCCESS
}

#[allow(dead_code)]
fn _ensure_metrics_is_used(snapshot: &Snapshot) -> String {
    render_prometheus(snapshot, 50)
}
