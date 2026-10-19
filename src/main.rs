#![cfg_attr(windows, windows_subsystem = "windows")]

mod autostart;
mod hotkey;
mod platform;
mod ui;

use clap::Parser;
use spotlight::{
    ai,
    cli::{Args, Command},
    config::Config,
    ipc, launch,
    search::{self, ResultKind},
};
use std::{io::Write, process::ExitCode, sync::Arc};
use tokio_util::sync::CancellationToken;

fn main() -> ExitCode {
    // The Windows binary is a GUI app, so borrow the terminal's console (if
    // any) before anything prints, including clap's --help and --version.
    #[cfg(windows)]
    platform::windows::attach_parent_console();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    let args = Args::parse();
    let result = match args.command.clone() {
        Some(Command::Search {
            query,
            no_files,
            open,
        }) => run_search(&query.join(" "), no_files, open),
        Some(Command::Ask { prompt }) => run_ask(prompt.join(" ")),
        None => launch(&args),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("spotlight: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Forward the request to a running instance, or become the instance and run
/// the UI. `--daemon` with a running instance is a quiet no-op.
fn launch(args: &Args) -> Result<(), String> {
    let initial = args.ipc_command();
    if let Some(command) = &initial {
        #[cfg(windows)]
        platform::windows::allow_foreground_handoff();
        if ipc::send_to_running(command).map_err(|e| e.to_string())? {
            return Ok(());
        }
    }
    let commands = match ipc::serve() {
        Ok(commands) => commands,
        Err(error) if error.kind() == std::io::ErrorKind::AddrInUse && initial.is_none() => {
            return Ok(());
        }
        Err(error) => return Err(error.to_string()),
    };
    ui::run(initial, commands)
}

fn load_config() -> Config {
    Config::load().unwrap_or_else(|error| {
        eprintln!("spotlight: {error}; using defaults");
        Config::default()
    })
}

fn run_search(query: &str, no_files: bool, open: bool) -> Result<(), String> {
    let config = load_config();
    let (apps, files) = std::thread::scope(|scope| {
        let apps = scope.spawn(search::discover_apps);
        let files =
            (!no_files).then(|| scope.spawn(|| search::build_file_index(&config.search_roots)));
        (
            apps.join().unwrap_or_default(),
            files
                .map(|f| f.join().unwrap_or_default())
                .unwrap_or_default(),
        )
    });
    let results = search::query(query, &apps, &files, &config.web_engine);
    let mut out = std::io::stdout().lock();
    for result in &results {
        let tag = match result.kind {
            ResultKind::App { .. } => "app ",
            ResultKind::File { .. } => "file",
            ResultKind::Web { .. } => "web ",
        };
        let _ = writeln!(out, "[{tag}] {}  —  {}", result.title, result.subtitle);
    }
    drop(out);
    if open && let Some(first) = results.first() {
        launch::activate(first)?;
    }
    Ok(())
}

fn run_ask(prompt: String) -> Result<(), String> {
    let config = load_config();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    runtime.block_on(async move {
        let files = if config.local_tools {
            let roots = config.search_roots.clone();
            tokio::task::spawn_blocking(move || search::build_file_index(&roots))
                .await
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let cancel = CancellationToken::new();
        let task = tokio::spawn(ai::run(prompt, config, Arc::new(files), tx, cancel.clone()));
        let mut failure = None;
        loop {
            let event = tokio::select! {
                _ = tokio::signal::ctrl_c() => { cancel.cancel(); break; }
                event = rx.recv() => event,
            };
            let Some(event) = event else { break };
            match event {
                ai::Event::Status(status) => eprintln!("· {status}"),
                ai::Event::Delta(part) => {
                    let mut out = std::io::stdout().lock();
                    let _ = out.write_all(part.as_bytes());
                    let _ = out.flush();
                }
                ai::Event::Done => {
                    println!();
                    break;
                }
                ai::Event::Error(error) => {
                    failure = Some(error);
                    break;
                }
            }
        }
        let _ = task.await;
        failure.map_or(Ok(()), Err)
    })
}
