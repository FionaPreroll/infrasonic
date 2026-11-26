// SPDX-License-Identifier: GPL-3.0-only
//! Plays music without a user interface: songs from a Subsonic server through
//! [`infrasonic_core::Session`], or local files straight through the engine. It is the Android
//! prototype (run it with `adb shell`), and handy for checking the engine on any device.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context as _, bail};
use clap::Parser;
use infrasonic_core::config::{PlaybackConfig, ReplayGainSetting};
use infrasonic_core::{Profile, Session, SessionEvent, config};
use infrasonic_engine::output::OutputFactory;
use infrasonic_engine::{
    Command, Engine, EngineConfig, Event, MediaSource, PlayState, Snapshot, SourceProvider, Track,
};
use infrasonic_subsonic::Child;

/// Plays songs without a user interface and prints what happens.
#[derive(Debug, Clone, Parser)]
#[command(name = "infrasonic-headless", version, about, long_about = None)]
pub struct Args {
    /// Local audio files to play. Without files, songs come from the server.
    pub files: Vec<PathBuf>,

    /// Server, like `https://user:password@music.example.org`. The password can also come
    /// from `INFRASONIC_PASSWORD`.
    #[arg(short, long, env = "INFRASONIC_URL")]
    pub url: Option<String>,

    /// Play this many random songs from the server.
    #[arg(long, default_value_t = 10)]
    pub random: u32,

    /// Play the songs of this album (ID3 album id).
    #[arg(long, conflicts_with = "search")]
    pub album: Option<String>,

    /// Play the songs a search finds.
    #[arg(long)]
    pub search: Option<String>,

    /// Stop after this many seconds.
    #[arg(long, value_name = "SECONDS")]
    pub seconds: Option<f64>,

    /// Volume 0–100.
    #[arg(long, default_value_t = 80)]
    pub volume: u8,

    /// ReplayGain: off, track, album or auto.
    #[arg(long, default_value = "auto")]
    pub replay_gain: String,

    /// Bitrate limit in kbit/s for server songs (0 = original).
    #[arg(long, default_value_t = 0)]
    pub bitrate: u32,

    /// Transcode to this format, e.g. `opus` or `mp3`.
    #[arg(long)]
    pub format: Option<String>,

    /// Discard the audio instead of playing it (for tests and devices without output).
    #[arg(long)]
    pub null: bool,

    /// Print a status line this often, in seconds.
    #[arg(long, default_value_t = 1.0)]
    pub status_every: f64,
}

/// What happened during a run.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Report {
    /// Titles in the order they started.
    pub played: Vec<String>,
    pub errors: Vec<String>,
    pub underruns: u64,
    /// Whether the queue was played to the end (not stopped by `--seconds`).
    pub finished: bool,
}

/// Status line for the log.
pub fn status_line(s: &Snapshot) -> String {
    let state = match s.state {
        PlayState::Playing => "playing",
        PlayState::Paused => "paused",
        PlayState::Buffering => "buffering",
        PlayState::Stopped => "stopped",
    };
    let title = s
        .current
        .as_ref()
        .map(|c| c.track.title.as_str())
        .unwrap_or("-");
    let dur = s.duration.map(|d| format!("{d:.1}")).unwrap_or("?".into());
    let mut line = format!(
        "{state} {}/{} {title} {:.1}/{dur}s underruns={}",
        s.current_index.map_or(0, |i| i + 1),
        s.queue.len(),
        s.position,
        s.underruns
    );
    if let Some(db) = s.replay_gain_db {
        line.push_str(&format!(" rg={db:+.1}dB"));
    }
    line
}

fn replay_gain(arg: &str) -> anyhow::Result<ReplayGainSetting> {
    Ok(match arg {
        "off" => ReplayGainSetting::Off,
        "track" => ReplayGainSetting::Track,
        "album" => ReplayGainSetting::Album,
        "auto" => ReplayGainSetting::Auto,
        other => bail!("unknown ReplayGain mode {other}"),
    })
}

/// Play according to `args` on `output` and report.
pub async fn run(args: &Args, output: Box<dyn OutputFactory>) -> anyhow::Result<Report> {
    let mut playback = PlaybackConfig {
        volume: args.volume.min(100),
        replay_gain: replay_gain(&args.replay_gain)?,
        max_bit_rate: args.bitrate,
        format: args.format.clone(),
        scrobble: false,
        save_queue_on_exit: false,
        ..PlaybackConfig::default()
    };
    if !args.files.is_empty() {
        return play_files(args, &playback, output).await;
    }
    let Some(url) = &args.url else {
        bail!("give audio files or a server with --url");
    };
    let profile: Profile = config::profile_from_url(url)?;
    playback.cache_mb = 0;
    let (session, mut events) = Session::connect("headless", &profile, playback, output)
        .await
        .with_context(|| format!("connecting to {}", profile.url))?;
    let info = session.server_info();
    println!(
        "connected to {} {} (API {})",
        info.server_type.as_deref().unwrap_or("subsonic"),
        info.server_version.as_deref().unwrap_or(""),
        info.version
    );
    let songs = server_songs(&session, args).await?;
    if songs.is_empty() {
        bail!("the server returned no songs");
    }
    println!("playing {} songs", songs.len());
    session.play_songs(&songs, 0);
    let report = watch(
        args,
        || session.snapshot(),
        || {
            let mut v = Vec::new();
            while let Ok(e) = events.try_recv() {
                match e {
                    SessionEvent::Engine(e) => v.push(e),
                    SessionEvent::Error(e) => v.push(Event::Error(e)),
                    _ => {}
                }
            }
            v
        },
    )
    .await;
    session.send(Command::Stop);
    session.finish().await.ok();
    Ok(report)
}

async fn server_songs(session: &Session, args: &Args) -> anyhow::Result<Vec<Child>> {
    let c = session.client();
    if let Some(id) = &args.album {
        return Ok(c.get_album(id).await?.song);
    }
    if let Some(q) = &args.search {
        let opts = infrasonic_subsonic::SearchOptions {
            artist_count: 0,
            album_count: 0,
            song_count: 50,
            ..Default::default()
        };
        return Ok(c.search3(q, opts).await?.song);
    }
    Ok(c.get_random_songs(args.random, None).await?)
}

/// Local files go straight to the engine; no server and no async I/O involved.
async fn play_files(
    args: &Args,
    playback: &PlaybackConfig,
    output: Box<dyn OutputFactory>,
) -> anyhow::Result<Report> {
    let mut tracks = Vec::new();
    for (i, p) in args.files.iter().enumerate() {
        if !p.is_file() {
            bail!("{} is not a file", p.display());
        }
        let mut t = Track::new(p.to_string_lossy(), file_title(p));
        t.suffix = p.extension().map(|e| e.to_string_lossy().to_lowercase());
        t.album_id = Some("files".into());
        t.track_number = Some(i as u32 + 1);
        tracks.push(t);
    }
    let provider: Arc<dyn SourceProvider> = Arc::new(|t: &Track| {
        std::fs::File::open(&t.id)
            .map(|f| Box::new(f) as Box<dyn MediaSource>)
            .map_err(|e| infrasonic_engine::Error::Source(format!("{}: {e}", t.id)))
    });
    let engine = Engine::new(
        EngineConfig {
            volume: playback.volume,
            replay_gain: playback.replay_gain_config(),
            ..EngineConfig::default()
        },
        provider,
        output,
    );
    println!("playing {} files", tracks.len());
    engine.send(Command::SetQueue {
        tracks,
        start: 0,
        position: 0.0,
        play: true,
    });
    let report = watch(
        args,
        || engine.snapshot(),
        || engine.events().try_iter().collect(),
    )
    .await;
    engine.send(Command::Stop);
    Ok(report)
}

fn file_title(p: &std::path::Path) -> String {
    p.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Print events and status lines until the queue ends or time is up.
async fn watch(
    args: &Args,
    snapshot: impl Fn() -> Snapshot,
    mut events: impl FnMut() -> Vec<Event>,
) -> Report {
    let started = Instant::now();
    let mut report = Report::default();
    let mut last_status = Instant::now();
    let every = Duration::from_secs_f64(args.status_every.max(0.1));
    loop {
        for e in events() {
            match e {
                Event::TrackChanged(Some(entry)) => {
                    println!("track {}", entry.track.title);
                    report.played.push(entry.track.title.clone());
                }
                Event::Error(e) => {
                    println!("error {e}");
                    report.errors.push(e);
                }
                Event::QueueEnded => {
                    report.finished = true;
                }
                _ => {}
            }
        }
        let snap = snapshot();
        report.underruns = snap.underruns;
        if report.finished {
            println!("{}", status_line(&snap));
            println!("end of the queue");
            return report;
        }
        if last_status.elapsed() >= every {
            println!("{}", status_line(&snap));
            last_status = Instant::now();
        }
        if args
            .seconds
            .is_some_and(|s| started.elapsed().as_secs_f64() >= s)
        {
            println!("{}", status_line(&snap));
            println!("time is up");
            return report;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
