// SPDX-License-Identifier: GPL-3.0-only
//! Soak test: random commands against a playing session for a long time.
//!
//! Runs only when `INFRASONIC_SOAK_SECS` is set (and servers are available):
//!
//! ```sh
//! INFRASONIC_SOAK_SECS=1800 scripts/e2e-servers.sh cargo test --release -p infrasonic-e2e --test soak -- --nocapture
//! ```
//!
//! `INFRASONIC_SOAK_SEED` makes a run reproducible. Checks: no errors, consistent snapshots,
//! the engine keeps answering, positions stay in range, playback makes progress, and memory and
//! thread counts stay flat.

use std::time::{Duration, Instant};

use infrasonic_core::config::PlaybackConfig;
use infrasonic_core::{Session, SessionEvent};
use infrasonic_e2e::{Server, null_output, servers};
use infrasonic_engine::{Command, Event, PlayState, RepeatMode};
use infrasonic_subsonic::Child;

/// Small deterministic PRNG (xorshift64*), so that a seed replays a run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Resident memory in KiB and thread count of this process (Linux only).
fn process_stats() -> Option<(u64, u64)> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    let field = |name: &str| {
        s.lines()
            .find_map(|l| l.strip_prefix(name))
            .and_then(|v| v.split_whitespace().next())
            .and_then(|v| v.parse().ok())
    };
    Some((field("VmRSS:")?, field("Threads:")?))
}

#[derive(Debug, Default)]
struct Stats {
    commands: u64,
    tracks_started: u64,
    queue_ends: u64,
    max_snapshot_ms: u128,
}

fn random_command(rng: &mut Rng, songs: &[Child], queue_len: usize) -> Option<Command> {
    let track = |c: &Child| infrasonic_core::track_from_child(c);
    let pick = |rng: &mut Rng, max: usize| -> Vec<_> {
        let n = 1 + rng.below(max);
        (0..n)
            .map(|_| track(&songs[rng.below(songs.len())]))
            .collect()
    };
    Some(match rng.below(100) {
        0..=14 => Command::Next,
        15..=21 => Command::Previous,
        22..=31 => Command::SeekBy(rng.unit() * 6.0 - 3.0),
        32..=36 => Command::Seek(rng.unit() * 6.0),
        37..=44 => Command::TogglePause,
        45..=47 => Command::Pause,
        48..=55 => Command::Play,
        56..=61 => Command::Append(pick(rng, 4)),
        62..=65 => Command::InsertNext(pick(rng, 3)),
        66..=70 if queue_len > 0 => Command::Remove(rng.below(queue_len)),
        71..=75 if queue_len > 1 => Command::Move {
            from: rng.below(queue_len),
            to: rng.below(queue_len),
        },
        76..=78 => Command::Shuffle,
        79..=81 => Command::SetRepeat(match rng.below(3) {
            0 => RepeatMode::Off,
            1 => RepeatMode::All,
            _ => RepeatMode::One,
        }),
        82..=86 => Command::SetQueue {
            tracks: pick(rng, 12),
            start: rng.below(4),
            position: rng.unit() * 2.0,
            play: rng.below(4) != 0,
        },
        87..=91 if queue_len > 0 => Command::PlayIndex(rng.below(queue_len)),
        92..=94 => Command::SetVolume(rng.below(101) as u8),
        95 => Command::Stop,
        96 => Command::Clear,
        _ => return None,
    })
}

async fn soak(server: &Server, secs: u64, seed: u64) {
    let playback = PlaybackConfig {
        save_queue_on_exit: false,
        scrobble: false,
        ..PlaybackConfig::default()
    };
    // Faster than real time so that many track changes, preloads and gapless transitions happen.
    let (session, mut rx) =
        Session::connect(&server.name, &server.profile(), playback, null_output(8.0))
            .await
            .expect("connect");
    let songs = session
        .client()
        .get_random_songs(100, None)
        .await
        .expect("songs");
    assert!(!songs.is_empty());
    session.play_songs(&songs, 0);

    let mut rng = Rng(seed | 1);
    let mut stats = Stats::default();
    let start = Instant::now();
    let deadline = start + Duration::from_secs(secs);
    let warmup = Duration::from_secs((secs / 10).clamp(5, 120));
    let mut baseline: Option<(u64, u64)> = None;
    let mut peak_threads = 0;
    let mut last_progress = Instant::now();
    let mut last_position = (0u64, 0.0f64);
    let mut errors = Vec::new();

    while Instant::now() < deadline {
        let wait = Duration::from_millis(20 + rng.below(400) as u64);
        let until = Instant::now() + wait;
        while let Ok(Some(ev)) = tokio::time::timeout_at(until.into(), rx.recv()).await {
            match ev {
                SessionEvent::Engine(Event::Error(e)) | SessionEvent::Error(e) => errors.push(e),
                SessionEvent::Engine(Event::TrackChanged(Some(_))) => stats.tracks_started += 1,
                SessionEvent::Engine(Event::QueueEnded) => stats.queue_ends += 1,
                SessionEvent::Engine(Event::Position { position, duration }) => {
                    assert!(position >= 0.0, "{}: negative position", server.name);
                    if let Some(d) = duration {
                        assert!(position <= d + 0.5, "{}: {position} > {d}", server.name);
                    }
                }
                _ => {}
            }
        }

        let t = Instant::now();
        let snap = session.snapshot();
        stats.max_snapshot_ms = stats.max_snapshot_ms.max(t.elapsed().as_millis());
        if let Some(i) = snap.current_index {
            assert!(
                i < snap.queue.len(),
                "{}: index {i} of {}",
                server.name,
                snap.queue.len()
            );
            assert_eq!(
                snap.current.as_ref().map(|c| c.uid),
                Some(snap.queue[i].uid)
            );
        }
        // Playback must move forward whenever it claims to play.
        let uid = snap.current.as_ref().map(|c| c.uid).unwrap_or(0);
        if snap.state != PlayState::Playing || (uid, snap.position) != last_position {
            last_progress = Instant::now();
            last_position = (uid, snap.position);
        }
        assert!(
            last_progress.elapsed() < Duration::from_secs(15),
            "{}: playing but stuck at {:?} (seed {seed})",
            server.name,
            last_position
        );

        if let Some(cmd) = random_command(&mut rng, &songs, snap.queue.len()) {
            stats.commands += 1;
            session.send(cmd);
        }
        if snap.queue.is_empty() || snap.state == PlayState::Stopped && rng.below(10) == 0 {
            session.play_songs(&songs, rng.below(songs.len()));
        }

        if let Some((rss, threads)) = process_stats() {
            peak_threads = peak_threads.max(threads);
            if baseline.is_none() && start.elapsed() > warmup {
                baseline = Some((rss, threads));
            }
        }
    }

    session.send(Command::Stop);
    tokio::time::sleep(Duration::from_millis(500)).await;
    let end = process_stats();
    eprintln!(
        "{}: {secs}s seed {seed}: {stats:?}, baseline {baseline:?}, end {end:?}, peak threads {peak_threads}",
        server.name
    );
    assert!(
        errors.is_empty(),
        "{}: errors (seed {seed}): {errors:#?}",
        server.name
    );
    assert!(
        stats.tracks_started > 10,
        "{}: too little playback: {stats:?}",
        server.name
    );
    assert!(
        stats.max_snapshot_ms < 500,
        "{}: engine slow to answer: {stats:?}",
        server.name
    );
    if let (Some((rss0, threads0)), Some((rss1, threads1))) = (baseline, end) {
        // Loader threads come and go; a steady growth means a leak.
        assert!(
            threads1 <= threads0 + 8,
            "{}: threads {threads0} -> {threads1}",
            server.name
        );
        assert!(
            rss1 <= rss0 + 64 * 1024,
            "{}: RSS {rss0} KiB -> {rss1} KiB",
            server.name
        );
    }
    session.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "multi_thread")]
async fn soak_random_commands() {
    let Some(secs) = std::env::var("INFRASONIC_SOAK_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
    else {
        eprintln!("INFRASONIC_SOAK_SECS not set; skipping soak test");
        return;
    };
    let seed = std::env::var("INFRASONIC_SOAK_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(1)
        });
    for s in servers() {
        soak(&s, secs, seed).await;
    }
}
