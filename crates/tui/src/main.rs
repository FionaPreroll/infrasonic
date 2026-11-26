// SPDX-License-Identifier: GPL-3.0-only
//! `infrasonic`: terminal music player for Subsonic servers.

use std::io::stdout;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use clap::Parser;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use infrasonic::cli::Args;
use infrasonic::cover::Cover;
use infrasonic::media::Media;
use infrasonic::{app, logging, setup};
use infrasonic_core::config::BrowseMode;
use infrasonic_core::{Config, Paths, Profile, Session};
use infrasonic_engine::output::{CpalOutputFactory, output_device_names};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

fn main() -> ExitCode {
    let args = Args::parse();
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("infrasonic: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Args) -> anyhow::Result<()> {
    let path = setup::config_path(&args);
    if let Some(src) = &args.import_stmps {
        return setup::import_stmps(src.as_deref(), &path);
    }
    if args.list_devices {
        for d in output_device_names() {
            println!("{d}");
        }
        return Ok(());
    }
    let mut config = setup::load_config(&path)?;
    if args.list {
        setup::list_profiles(&config, &path);
        return Ok(());
    }
    if let Some(name) = &args.store_password {
        return setup::store_password(&config, name);
    }
    if args.add_profile {
        setup::add_profile(&mut config, &path)?;
        return Ok(());
    }
    let (name, profile) = setup::select_profile(&args, &mut config, &path)?;
    if args.tags {
        config.ui.browse_mode = BrowseMode::Tags;
    }
    let paths = Paths::platform();
    let log_file = args.log_file.clone().unwrap_or(paths.log_file.clone());
    let log = logging::init(Some(&log_file));
    tracing::info!("infrasonic {} starting", env!("CARGO_PKG_VERSION"));
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("infrasonic-rt")
        .build()?;
    rt.block_on(play(name, profile, config, paths, log))
}

async fn play(
    name: String,
    profile: Profile,
    config: Config,
    paths: Paths,
    log: logging::LogBuffer,
) -> anyhow::Result<()> {
    eprintln!("Connecting to {}…", profile.url);
    let output = Box::new(CpalOutputFactory {
        device: config.playback.device.clone(),
    });
    let (session, events) = Session::connect(&name, &profile, config.playback.clone(), output)
        .await
        .with_context(|| format!("connecting to {}", profile.url))?;
    let session = Arc::new(session);

    let mut terminal = init_terminal(config.ui.mouse)?;
    let cover = Cover::new(config.ui.cover_art, Some(paths.cache_dir.join("covers")));
    let (mut app, msgs) = app::App::new(
        session.clone(),
        config.ui.clone(),
        config.playback.clone(),
        log,
        cover,
        None,
    );
    match Media::new(app.sender()) {
        Ok(m) => app.set_media(m),
        Err(e) => tracing::info!("media controls unavailable: {e}"),
    }
    let result = app::run(&mut terminal, &mut app, msgs, events).await;
    restore_terminal();
    drop(app);

    if config.playback.save_queue_on_exit {
        eprintln!("Saving the queue on the server…");
    }
    match tokio::time::timeout(Duration::from_secs(5), session.finish()).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => eprintln!("infrasonic: saving the queue failed: {e}"),
        Err(_) => eprintln!("infrasonic: saving the queue timed out"),
    }
    result
}

fn init_terminal(mouse: bool) -> anyhow::Result<Terminal<CrosstermBackend<std::io::Stdout>>> {
    let hook = std::panic::take_hook();
    let main = std::thread::current().id();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!("panic: {info}");
        // Panics on other threads (tokio tasks, libraries) do not end the program.
        if std::thread::current().id() == main {
            restore_terminal();
            hook(info);
        }
    }));
    enable_raw_mode()?;
    execute!(stdout(), EnterAlternateScreen)?;
    if mouse {
        execute!(stdout(), EnableMouseCapture)?;
    }
    Ok(Terminal::new(CrosstermBackend::new(stdout()))?)
}

fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(
        stdout(),
        DisableMouseCapture,
        LeaveAlternateScreen,
        crossterm::cursor::Show
    );
}
