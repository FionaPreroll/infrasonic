// SPDX-License-Identifier: GPL-3.0-only
//! `infrasonic`: terminal music player for Subsonic servers.

use std::io::stdout;
use std::path::PathBuf;
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
use infrasonic::keys::Keymap;
use infrasonic::media::Media;
use infrasonic::theme::{self, Theme};
use infrasonic::{app, i18n, logging, setup};
use infrasonic_core::cache::{AudioCache, server_key};
use infrasonic_core::config::BrowseMode;
use infrasonic_core::{Config, CoreError, Paths, Profile, Session, SessionEvent};
use infrasonic_engine::output::{CpalOutputFactory, output_device_names};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::sync::mpsc::UnboundedReceiver;

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
    i18n::set_language(i18n::Lang::from_setting(&config.ui.language));
    let themes_dir = path.parent().map(|d| d.join("themes"));
    let (theme, warnings) = Theme::load(&config.ui.theme, themes_dir.as_deref(), &config.ui.colors);
    theme::apply(theme);
    for w in warnings {
        tracing::warn!("{w}");
    }
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("infrasonic-rt")
        .build()?;
    let ctx = Context {
        config,
        paths,
        log,
        themes_dir,
        offline: args.offline,
    };
    rt.block_on(async {
        let (mut name, mut profile) = (name, profile);
        loop {
            match play(&ctx, &name, &profile).await? {
                Some(next) => {
                    let Some(p) = ctx.config.profiles.get(&next) else {
                        anyhow::bail!("unknown profile {next}");
                    };
                    profile = p.clone();
                    name = next;
                }
                None => return Ok(()),
            }
        }
    })
}

/// What stays the same across profile switches.
struct Context {
    config: Config,
    paths: Paths,
    log: logging::LogBuffer,
    themes_dir: Option<PathBuf>,
    offline: bool,
}

/// Connect, or open the pinned songs if the server cannot be reached.
async fn connect(
    ctx: &Context,
    name: &str,
    profile: &Profile,
) -> anyhow::Result<(Session, UnboundedReceiver<SessionEvent>)> {
    let output = || {
        Box::new(CpalOutputFactory {
            device: ctx.config.playback.device.clone(),
        })
    };
    let playback = ctx.config.playback.clone();
    if ctx.offline {
        return Ok(Session::offline(name, profile, playback, output())?);
    }
    eprintln!("Connecting to {}…", profile.url);
    match Session::connect(name, profile, playback.clone(), output()).await {
        Ok(s) => Ok(s),
        Err(CoreError::Subsonic(e)) if matches!(e, infrasonic_subsonic::Error::Http(_)) => {
            tracing::warn!("{} unreachable, starting offline: {e}", profile.url);
            eprintln!("{} is unreachable ({e}), starting offline.", profile.url);
            Ok(Session::offline(name, profile, playback, output())?)
        }
        Err(e) => Err(e).with_context(|| format!("connecting to {}", profile.url)),
    }
}

/// Run one session. Returns the profile to switch to, if the user picked one.
async fn play(ctx: &Context, name: &str, profile: &Profile) -> anyhow::Result<Option<String>> {
    let (session, events) = connect(ctx, name, profile).await?;
    let session = Arc::new(session);
    if ctx.config.playback.cache_mb > 0 {
        let cache = AudioCache::new(
            &ctx.paths.cache_dir,
            &ctx.paths.data_dir,
            &server_key(&profile.url, &profile.username),
            ctx.config.playback.cache_mb << 20,
        );
        session.set_cache(Some(Arc::new(cache)));
    }

    let mut terminal = init_terminal(ctx.config.ui.mouse)?;
    let cover = Cover::new(
        ctx.config.ui.cover_art,
        Some(ctx.paths.cache_dir.join("covers")),
    );
    let (mut app, msgs) = app::App::new(
        session.clone(),
        ctx.config.ui.clone(),
        ctx.config.playback.clone(),
        ctx.log.clone(),
        cover,
        None,
    );
    app.set_keymap(Keymap::with_overrides(&ctx.config.keys));
    app.profiles = ctx.config.profiles.keys().cloned().collect();
    app.themes_dir = ctx.themes_dir.clone();
    match Media::new(app.sender()) {
        Ok(m) => app.set_media(m),
        Err(e) => tracing::info!("media controls unavailable: {e}"),
    }
    let result = app::run(&mut terminal, &mut app, msgs, events).await;
    restore_terminal();
    let next = app.switch_to.take();
    drop(app);

    if ctx.config.playback.save_queue_on_exit && session.is_online() {
        eprintln!("Saving the queue on the server…");
    }
    match tokio::time::timeout(Duration::from_secs(5), session.finish()).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => eprintln!("infrasonic: saving the queue failed: {e}"),
        Err(_) => eprintln!("infrasonic: saving the queue timed out"),
    }
    result.map(|()| next)
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
