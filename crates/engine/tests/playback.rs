// SPDX-License-Identifier: GPL-3.0-only
//! Decoding and playback tests with the synthetic files from `testdata/music`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use infrasonic_engine::decode::TrackDecoder;
use infrasonic_engine::output::{
    AudioOutput, NullOutputFactory, OutputFactory, OutputShared, OutputSpec, Producer,
};
use infrasonic_engine::source::StreamReader;
use infrasonic_engine::{
    Command, Engine, EngineConfig, Error, Event, MediaSource, PlayState, RepeatMode, Track,
};
use parking_lot::Mutex;

fn music() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/music")
}

fn file(rel: &str) -> PathBuf {
    music().join(rel)
}

const SINE: &str = "Infrasonic Test Ensemble/Sine Studies (2024)";
const FORMATS: &str = "The Codec Quartet/Formats Live (2025)";

fn open(path: &Path) -> TrackDecoder {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let ext = path.extension().and_then(|e| e.to_str());
    TrackDecoder::open(Box::new(StreamReader::from_bytes(bytes)), ext)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

struct Decoded {
    left: Vec<f32>,
    rate: u32,
    channels: usize,
}

fn decode_all(dec: &mut TrackDecoder) -> Decoded {
    let mut left = Vec::new();
    let mut rate = 0;
    let mut channels = 0;
    while let Some(c) = dec.next_chunk().expect("decode") {
        rate = c.rate;
        channels = c.channels;
        left.extend(c.samples.iter().step_by(c.channels));
    }
    Decoded {
        left,
        rate,
        channels,
    }
}

/// Estimate the dominant frequency from zero crossings, skipping the edges.
fn frequency(samples: &[f32], rate: u32) -> f64 {
    let skip = (rate / 10) as usize;
    let mid = &samples[skip.min(samples.len())..samples.len().saturating_sub(skip)];
    let crossings = mid.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
    crossings as f64 / (mid.len() as f64 / f64::from(rate))
}

#[test]
fn decodes_all_formats() {
    let cases = [
        (format!("{SINE}/01 - Low A.flac"), 220.0, 4.0, 44_100),
        (format!("{FORMATS}/01 - MP3 Song.mp3"), 330.0, 5.0, 44_100),
        (
            format!("{FORMATS}/02 - Vorbis Song.ogg"),
            350.0,
            5.0,
            44_100,
        ),
        (format!("{FORMATS}/03 - Opus Song.opus"), 370.0, 5.0, 48_000),
        (format!("{FORMATS}/04 - AAC Song.m4a"), 390.0, 5.0, 44_100),
    ];
    for (path, freq, secs, rate) in cases {
        let mut dec = open(&file(&path));
        let d = decode_all(&mut dec);
        assert_eq!(d.rate, rate, "{path}");
        assert_eq!(d.channels, 2, "{path}");
        let got_secs = d.left.len() as f64 / f64::from(d.rate);
        // MP3/AAC carry encoder delay that not every container signals.
        assert!(
            (got_secs - secs).abs() < 0.08,
            "{path}: {got_secs:.3}s instead of {secs}s"
        );
        let f = frequency(&d.left, d.rate);
        assert!(
            (f - freq).abs() < 3.0,
            "{path}: {f:.1} Hz instead of {freq} Hz"
        );
        let peak = d.left.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak > 0.03 && peak <= 1.0, "{path}: peak {peak}");
    }
}

#[test]
fn seek_is_sample_accurate_for_flac() {
    let mut dec = open(&file(&format!("{SINE}/02 - Middle A.flac")));
    let pos = dec.seek(1.5).expect("seek");
    assert!((pos - 1.5).abs() < 1e-9);
    let d = decode_all(&mut dec);
    let remaining = d.left.len() as f64 / f64::from(d.rate);
    assert!((remaining - 2.5).abs() < 0.01, "remaining {remaining}");
}

#[test]
fn seek_works_for_lossy_formats() {
    for name in [
        "01 - MP3 Song.mp3",
        "02 - Vorbis Song.ogg",
        "03 - Opus Song.opus",
    ] {
        let mut dec = open(&file(&format!("{FORMATS}/{name}")));
        dec.seek(3.0).expect("seek");
        let d = decode_all(&mut dec);
        let remaining = d.left.len() as f64 / f64::from(d.rate);
        assert!(
            (remaining - 2.0).abs() < 0.1,
            "{name}: remaining {remaining}"
        );
    }
}

#[test]
fn garbage_is_rejected() {
    let r = TrackDecoder::open(
        Box::new(StreamReader::from_bytes(vec![0x42; 4096])),
        Some("mp3"),
    );
    assert!(r.is_err());
}

// ---------------------------------------------------------------------------------------------
// Engine tests

fn track(rel: &str) -> Track {
    let p = file(rel);
    let mut t = Track::new(p.to_string_lossy(), rel);
    t.suffix = p.extension().map(|e| e.to_string_lossy().into_owned());
    t
}

fn file_provider() -> Arc<dyn infrasonic_engine::SourceProvider> {
    Arc::new(|t: &Track| -> Result<Box<dyn MediaSource>, Error> {
        if t.id == "broken" {
            return Ok(Box::new(StreamReader::from_bytes(vec![7; 10_000])));
        }
        let bytes = std::fs::read(&t.id).map_err(|e| Error::Source(e.to_string()))?;
        Ok(Box::new(StreamReader::from_bytes(bytes)))
    })
}

const SPEC: OutputSpec = OutputSpec {
    sample_rate: 44_100,
    channels: 2,
};

fn engine(speed: f64, capture: Option<Arc<Mutex<Vec<f32>>>>) -> Engine {
    let mut out = NullOutputFactory::new(SPEC, speed);
    if let Some(c) = capture {
        out = out.with_capture(c);
    }
    Engine::new(EngineConfig::default(), file_provider(), Box::new(out))
}

fn wait_for(
    engine: &Engine,
    timeout: Duration,
    mut pred: impl FnMut(&Event) -> bool,
) -> Vec<Event> {
    let deadline = Instant::now() + timeout;
    let mut seen = Vec::new();
    while Instant::now() < deadline {
        if let Ok(ev) = engine.events().recv_timeout(Duration::from_millis(20)) {
            let done = pred(&ev);
            seen.push(ev);
            if done {
                return seen;
            }
        }
    }
    panic!("timeout; events: {seen:#?}");
}

fn sine_album() -> Vec<Track> {
    vec![
        track(&format!("{SINE}/01 - Low A.flac")),
        track(&format!("{SINE}/02 - Middle A.flac")),
        track(&format!("{SINE}/03 - High A.flac")),
    ]
}

#[test]
fn plays_album_gaplessly() {
    let capture = Arc::new(Mutex::new(Vec::new()));
    let e = engine(20.0, Some(capture.clone()));
    e.send(Command::SetQueue {
        tracks: sine_album(),
        start: 0,
        position: 0.0,
        play: true,
    });
    let events = wait_for(&e, Duration::from_secs(10), |ev| *ev == Event::QueueEnded);
    let changes: Vec<String> = events
        .iter()
        .filter_map(|ev| match ev {
            Event::TrackChanged(Some(t)) => Some(t.track.title.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        changes,
        sine_album()
            .iter()
            .map(|t| t.title.clone())
            .collect::<Vec<_>>()
    );
    assert!(events.contains(&Event::StateChanged(PlayState::Playing)));
    assert_eq!(e.snapshot().state, PlayState::Stopped);

    let samples = capture.lock();
    let left: Vec<f32> = samples.iter().step_by(2).copied().collect();
    let secs = left.len() as f64 / 44_100.0;
    assert!((secs - 12.0).abs() < 0.05, "played {secs}s instead of 12s");
    // A gap would show up as a run of silence between the tracks.
    let mut run = 0usize;
    let mut longest = 0usize;
    for s in &left[..left.len() - 100] {
        if s.abs() < 1e-4 {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    assert!(longest < 8, "found {longest} silent samples in a row");
}

#[test]
fn transport_commands() {
    let e = engine(1.0, None);
    e.send(Command::SetVolume(50));
    e.send(Command::SetQueue {
        tracks: sine_album(),
        start: 1,
        position: 0.0,
        play: true,
    });
    wait_for(&e, Duration::from_secs(5), |ev| {
        *ev == Event::StateChanged(PlayState::Playing)
    });
    // Slow CI runners need a moment before the first audio is played; poll instead of sleeping.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let mut s = e.snapshot();
    while s.position <= 0.1 && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
        s = e.snapshot();
    }
    assert_eq!(s.current_index, Some(1));
    assert!(
        s.position > 0.1 && s.position < 1.5,
        "position {}",
        s.position
    );
    assert_eq!(s.volume, 50);

    e.send(Command::Pause);
    wait_for(&e, Duration::from_secs(2), |ev| {
        *ev == Event::StateChanged(PlayState::Paused)
    });
    // The snapshot may still trail the output by one loop turn (Windows timers are coarse).
    std::thread::sleep(Duration::from_millis(100));
    let p1 = e.snapshot().position;
    std::thread::sleep(Duration::from_millis(200));
    assert!(
        (e.snapshot().position - p1).abs() < 0.02,
        "position moved while paused"
    );

    e.send(Command::Seek(3.0));
    let seen = wait_for(
        &e,
        Duration::from_secs(2),
        |ev| matches!(ev, Event::Position { position, .. } if (*position - 3.0).abs() < 0.05),
    );
    // Seek keeps the pause state, also while the decoder seeks.
    assert!(
        !seen.iter().any(|ev| matches!(ev, Event::StateChanged(_))),
        "{seen:?}"
    );
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(e.snapshot().state, PlayState::Paused);
    e.send(Command::TogglePause);
    wait_for(&e, Duration::from_secs(2), |ev| {
        *ev == Event::StateChanged(PlayState::Playing)
    });

    e.send(Command::Next);
    wait_for(
        &e,
        Duration::from_secs(3),
        |ev| matches!(ev, Event::TrackChanged(Some(t)) if t.track.title.contains("High A")),
    );
    e.send(Command::Previous);
    wait_for(
        &e,
        Duration::from_secs(3),
        |ev| matches!(ev, Event::TrackChanged(Some(t)) if t.track.title.contains("Middle A")),
    );
    e.send(Command::Stop);
    wait_for(&e, Duration::from_secs(2), |ev| {
        *ev == Event::StateChanged(PlayState::Stopped)
    });
    // The snapshot is up to date when the event arrives.
    assert_eq!(e.snapshot().state, PlayState::Stopped);
    assert_eq!(e.snapshot().position, 0.0);
}

/// Frontends read the snapshot when an event arrives (the Android app does); it must not
/// trail the event, or a state such as "buffering" would stay their view.
#[test]
fn snapshot_is_current_when_the_event_arrives() {
    for _ in 0..20 {
        let e = engine(1.0, None);
        e.send(Command::SetQueue {
            tracks: sine_album(),
            start: 0,
            position: 1.0,
            play: true,
        });
        wait_for(&e, Duration::from_secs(5), |ev| {
            *ev == Event::StateChanged(PlayState::Playing)
        });
        assert_eq!(e.snapshot().state, PlayState::Playing);
        e.send(Command::Pause);
        wait_for(&e, Duration::from_secs(2), |ev| {
            *ev == Event::StateChanged(PlayState::Paused)
        });
        assert_eq!(e.snapshot().state, PlayState::Paused);
    }
}

/// Files that take a while to open, like a song that starts streaming.
fn slow_provider(
    delay: Duration,
    opened: Arc<AtomicUsize>,
) -> Arc<dyn infrasonic_engine::SourceProvider> {
    let files = file_provider();
    Arc::new(move |t: &Track| -> Result<Box<dyn MediaSource>, Error> {
        opened.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(delay);
        files.open(t)
    })
}

#[test]
fn pause_while_opening_shows_at_once() {
    let opened = Arc::new(AtomicUsize::new(0));
    let e = Engine::new(
        EngineConfig::default(),
        slow_provider(Duration::from_millis(800), opened),
        Box::new(NullOutputFactory::new(SPEC, 1.0)),
    );
    e.send(Command::SetQueue {
        tracks: sine_album(),
        start: 0,
        position: 0.0,
        play: true,
    });
    wait_for(&e, Duration::from_secs(2), |ev| {
        *ev == Event::StateChanged(PlayState::Buffering)
    });
    let asked = Instant::now();
    e.send(Command::Pause);
    wait_for(&e, Duration::from_secs(2), |ev| {
        *ev == Event::StateChanged(PlayState::Paused)
    });
    assert!(
        asked.elapsed() < Duration::from_millis(500),
        "the pause waited for the song"
    );
    // Paused once open, without playing a moment.
    std::thread::sleep(Duration::from_millis(1200));
    let s = e.snapshot();
    assert_eq!(s.state, PlayState::Paused);
    assert_eq!(s.position, 0.0);
    // Play goes on.
    e.send(Command::TogglePause);
    wait_for(
        &e,
        Duration::from_secs(5),
        |ev| matches!(ev, Event::Position { position, .. } if *position > 0.3),
    );
    assert_eq!(e.snapshot().state, PlayState::Playing);
}

/// The Bluetooth speaker is switched off: the app pauses, then the device goes away. The
/// engine opens another output, stays paused where it was, without a "buffering" state and
/// without fetching the song again, and plays on when asked.
#[test]
fn lost_device_while_paused_stays_paused() {
    let last = Arc::new(Mutex::new(None::<Arc<OutputShared>>));
    let opened = Arc::new(AtomicUsize::new(0));
    let out = FlakyOutput {
        inner: NullOutputFactory::new(SPEC, 1.0),
        last: last.clone(),
        opened: opened.clone(),
    };
    let songs = Arc::new(AtomicUsize::new(0));
    let config = EngineConfig {
        preload_next: false,
        ..EngineConfig::default()
    };
    let e = Engine::new(
        config,
        slow_provider(Duration::from_millis(300), songs.clone()),
        Box::new(out),
    );
    e.send(Command::SetQueue {
        tracks: sine_album(),
        start: 1,
        position: 0.0,
        play: true,
    });
    wait_for(
        &e,
        Duration::from_secs(5),
        |ev| matches!(ev, Event::Position { position, .. } if *position > 1.0),
    );
    e.send(Command::Pause);
    wait_for(&e, Duration::from_secs(2), |ev| {
        *ev == Event::StateChanged(PlayState::Paused)
    });
    std::thread::sleep(Duration::from_millis(100));
    let at = e.snapshot().position;
    if let Some(s) = last.lock().as_ref() {
        s.mark_failed();
    }
    // Long enough for the song to be opened again, if it were.
    let until = Instant::now() + Duration::from_secs(2);
    let mut seen = Vec::new();
    while Instant::now() < until {
        if let Ok(ev) = e.events().recv_timeout(Duration::from_millis(20)) {
            seen.push(ev);
        }
    }
    assert_eq!(
        opened.load(Ordering::SeqCst),
        2,
        "the output was not opened again"
    );
    assert!(
        !seen
            .iter()
            .any(|ev| matches!(ev, Event::StateChanged(_) | Event::Error(_))),
        "{seen:?}"
    );
    let s = e.snapshot();
    assert_eq!(s.state, PlayState::Paused);
    assert_eq!(s.current_index, Some(1));
    assert!(
        (s.position - at).abs() < 0.05,
        "position {} instead of {at}",
        s.position
    );
    assert_eq!(songs.load(Ordering::SeqCst), 1, "the song was opened again");

    e.send(Command::Play);
    wait_for(&e, Duration::from_secs(2), |ev| {
        *ev == Event::StateChanged(PlayState::Playing)
    });
    wait_for(
        &e,
        Duration::from_secs(5),
        |ev| matches!(ev, Event::Position { position, .. } if *position > at + 0.5),
    );
}

#[test]
fn broken_track_is_skipped() {
    let e = engine(20.0, None);
    let mut broken = Track::new("broken", "Broken");
    broken.suffix = Some("mp3".into());
    e.send(Command::SetQueue {
        tracks: vec![broken, track(&format!("{SINE}/01 - Low A.flac"))],
        start: 0,
        position: 0.0,
        play: true,
    });
    let events = wait_for(&e, Duration::from_secs(10), |ev| *ev == Event::QueueEnded);
    assert!(events.iter().any(|ev| matches!(ev, Event::Error(_))));
    assert!(
        events.iter().any(
            |ev| matches!(ev, Event::TrackChanged(Some(t)) if t.track.title.contains("Low A"))
        )
    );
}

#[test]
fn repeat_one_and_queue_edits() {
    let e = engine(20.0, None);
    e.send(Command::SetRepeat(RepeatMode::One));
    e.send(Command::SetQueue {
        tracks: sine_album(),
        start: 2,
        position: 3.0,
        play: true,
    });
    // High A ends and starts again.
    let ev = wait_for(
        &e,
        Duration::from_secs(5),
        |ev| matches!(ev, Event::TrackChanged(Some(t)) if t.track.title.contains("High A")),
    );
    assert!(!ev.is_empty());
    wait_for(
        &e,
        Duration::from_secs(5),
        |ev| matches!(ev, Event::TrackChanged(Some(t)) if t.track.title.contains("High A")),
    );
    e.send(Command::SetRepeat(RepeatMode::Off));
    e.send(Command::Move { from: 2, to: 0 });
    e.send(Command::Remove(1));
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    let mut s = e.snapshot();
    while s.queue.len() != 2 && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
        s = e.snapshot();
    }
    assert_eq!(s.queue.len(), 2, "{s:?}");
    assert_eq!(s.current_index, Some(0), "{s:?}");
    assert!(
        s.current
            .as_ref()
            .is_some_and(|c| c.track.title.contains("High A")),
        "{s:?}"
    );
    e.send(Command::Clear);
    wait_for(&e, Duration::from_secs(2), |ev| {
        *ev == Event::StateChanged(PlayState::Stopped)
    });
    assert!(e.snapshot().queue.is_empty());
}

#[test]
fn appending_after_queue_end_continues() {
    let e = engine(20.0, None);
    e.send(Command::SetQueue {
        tracks: vec![track(&format!("{SINE}/01 - Low A.flac"))],
        start: 0,
        position: 3.5,
        play: true,
    });
    wait_for(&e, Duration::from_secs(5), |ev| {
        *ev == Event::StateChanged(PlayState::Playing)
    });
    e.send(Command::Append(vec![track(&format!(
        "{SINE}/02 - Middle A.flac"
    ))]));
    wait_for(
        &e,
        Duration::from_secs(5),
        |ev| matches!(ev, Event::TrackChanged(Some(t)) if t.track.title.contains("Middle A")),
    );
}

/// A [`NullOutputFactory`] that keeps the state of the stream it opened last, so a test can
/// make the "device" fail, and counts how often it was opened.
struct FlakyOutput {
    inner: NullOutputFactory,
    last: Arc<Mutex<Option<Arc<OutputShared>>>>,
    opened: Arc<AtomicUsize>,
}

impl OutputFactory for FlakyOutput {
    fn open(
        &self,
        shared: Arc<OutputShared>,
        buffer: Duration,
    ) -> Result<(Box<dyn AudioOutput>, Producer<f32>), Error> {
        self.opened.fetch_add(1, Ordering::SeqCst);
        *self.last.lock() = Some(shared.clone());
        self.inner.open(shared, buffer)
    }
}

#[test]
fn lost_device_is_opened_again() {
    let last = Arc::new(Mutex::new(None::<Arc<OutputShared>>));
    let opened = Arc::new(AtomicUsize::new(0));
    let out = FlakyOutput {
        inner: NullOutputFactory::new(SPEC, 1.0),
        last: last.clone(),
        opened: opened.clone(),
    };
    let e = Engine::new(EngineConfig::default(), file_provider(), Box::new(out));
    e.send(Command::SetQueue {
        tracks: sine_album(),
        start: 1,
        position: 0.0,
        play: true,
    });
    wait_for(
        &e,
        Duration::from_secs(5),
        |ev| matches!(ev, Event::Position { position, .. } if *position > 1.0),
    );
    let fail = |last: &Mutex<Option<Arc<OutputShared>>>| {
        if let Some(s) = last.lock().as_ref() {
            s.mark_failed();
        }
    };
    fail(&last);
    // Playback goes on in the same track, from about where it was.
    wait_for(
        &e,
        Duration::from_secs(5),
        |ev| matches!(ev, Event::Position { position, .. } if *position > 2.0),
    );
    let snap = e.snapshot();
    assert_eq!(snap.state, PlayState::Playing);
    assert_eq!(snap.current_index, Some(1));
    assert_eq!(opened.load(Ordering::SeqCst), 2);

    // A device that keeps failing ends playback with an error.
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut lost = false;
    while Instant::now() < deadline && !lost {
        fail(&last);
        if let Ok(ev) = e.events().recv_timeout(Duration::from_millis(50)) {
            lost = matches!(&ev, Event::Error(m) if m.contains("audio device lost"));
        }
    }
    assert!(lost, "no error after repeated failures");
    assert_eq!(opened.load(Ordering::SeqCst), 4);
}

#[test]
fn equalizer_changes_what_is_heard() {
    let rms = |eq: Option<infrasonic_engine::Equalizer>| {
        let capture = Arc::new(Mutex::new(Vec::new()));
        let e = engine(20.0, Some(capture.clone()));
        e.send(Command::SetEqualizer(eq));
        e.send(Command::SetQueue {
            tracks: sine_album()[..1].to_vec(),
            start: 0,
            position: 0.0,
            play: true,
        });
        wait_for(&e, Duration::from_secs(10), |ev| *ev == Event::QueueEnded);
        let s = capture.lock();
        let s = &s[s.len() / 4..s.len() * 3 / 4];
        (s.iter().map(|x| x * x).sum::<f32>() / s.len() as f32).sqrt()
    };
    let plain = rms(None);
    let cut = rms(Some(infrasonic_engine::Equalizer::new(0.0, &[-12.0; 10])));
    let db = 20.0 * (cut / plain).log10();
    assert!(
        (db + 12.0).abs() < 1.5,
        "equalizer changed the level by {db} dB"
    );
}
