// SPDX-License-Identifier: GPL-3.0-only
//! The single-threaded pump mode used in the browser: driven by hand with a pull output, it
//! must play exactly what the threaded engine plays, and wait for sources that are not ready.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use infrasonic_engine::decode::TrackDecoder;
use infrasonic_engine::output::{OutputSpec, PullOutputFactory};
use infrasonic_engine::{
    Command, Engine, EngineConfig, Error, Event, MediaSource, PlayState, SourceProvider, Track,
};

const PART_FRAMES: usize = 88_200;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testdata/gapless")
}

/// Files that become ready only after `delay` readiness checks each.
struct SlowFiles {
    delay: usize,
    checks: AtomicUsize,
}

impl SourceProvider for SlowFiles {
    fn open(&self, t: &Track) -> Result<Box<dyn MediaSource>, Error> {
        let f = std::fs::File::open(&t.id).map_err(|e| Error::Source(e.to_string()))?;
        Ok(Box::new(f))
    }
    fn is_ready(&self, _t: &Track) -> bool {
        self.checks.fetch_add(1, Ordering::Relaxed) % (self.delay + 1) == self.delay
    }
}

fn tracks(ext: &str) -> Vec<Track> {
    (1..=3)
        .map(|n| {
            let p = dir().join(ext).join(format!("part{n}.{ext}"));
            let mut t = Track::new(p.to_string_lossy(), format!("part{n}"));
            t.suffix = Some(ext.into());
            t
        })
        .collect()
}

fn reference() -> Vec<f32> {
    let mut out = Vec::new();
    for n in 1..=3 {
        let p = dir().join("flac").join(format!("part{n}.flac"));
        let f = std::fs::File::open(&p).expect("open");
        let mut dec = TrackDecoder::open(Box::new(f), Some("flac")).expect("decoder");
        while let Some(c) = dec.next_chunk().expect("decode") {
            out.extend_from_slice(c.samples);
        }
    }
    out
}

/// Pull audio like an AudioWorklet would: 128-frame quanta, a few per turn.
fn run(ext: &str, rate: u32, delay: usize) -> (Vec<f32>, Vec<Event>) {
    let (factory, handle) = PullOutputFactory::new(OutputSpec {
        sample_rate: rate,
        channels: 2,
    });
    let provider = Arc::new(SlowFiles {
        delay,
        checks: AtomicUsize::new(0),
    });
    let (engine, mut pump) = Engine::new_pump(EngineConfig::default(), provider, Box::new(factory));
    engine.send(Command::SetQueue {
        tracks: tracks(ext),
        start: 0,
        position: 0.0,
        play: true,
    });
    let mut out = Vec::new();
    let mut events = Vec::new();
    let mut buf = vec![0.0f32; 128 * 2];
    for _ in 0..200_000 {
        assert!(pump.turn());
        for _ in 0..4 {
            let n = handle.pull(&mut buf);
            out.extend_from_slice(&buf[..n * 2]);
        }
        let mut ended = false;
        for e in engine.events().try_iter() {
            ended |= e == Event::QueueEnded;
            if let Event::Error(err) = &e {
                panic!("{ext}: {err}");
            }
            events.push(e);
        }
        if ended {
            return (out, events);
        }
    }
    panic!("{ext}: the queue did not end");
}

#[test]
fn pump_plays_gaplessly() {
    let reference = reference();
    let (out, events) = run("flac", 44_100, 0);
    assert_eq!(out.len(), reference.len(), "flac frames");
    let max = out
        .iter()
        .zip(&reference)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    assert!(max < 1e-6, "flac differs by {max}");
    let tracks = events
        .iter()
        .filter(|e| matches!(e, Event::TrackChanged(Some(_))))
        .count();
    assert_eq!(tracks, 3);
}

#[test]
fn pump_waits_for_sources_and_resamples() {
    // Sources that need several turns before they are ready still join without a gap, and
    // 48 kHz Opus is resampled for a 44.1 kHz device.
    let (out, _) = run("opus", 44_100, 25);
    let n = out.len() / 2;
    assert!(n.abs_diff(3 * PART_FRAMES) <= 64, "{n} frames");
}

#[test]
fn pump_pause_seek_and_flush() {
    let (factory, handle) = PullOutputFactory::new(OutputSpec {
        sample_rate: 44_100,
        channels: 2,
    });
    let provider = Arc::new(SlowFiles {
        delay: 0,
        checks: AtomicUsize::new(0),
    });
    let (engine, mut pump) = Engine::new_pump(EngineConfig::default(), provider, Box::new(factory));
    engine.send(Command::SetQueue {
        tracks: tracks("flac"),
        start: 1,
        position: 0.5,
        play: true,
    });
    let mut buf = vec![0.0f32; 4410 * 2];
    let mut turn = |pull: bool| {
        pump.turn();
        if pull { handle.pull(&mut buf) } else { 0 }
    };
    for _ in 0..8 {
        turn(true);
    }
    let s = engine.snapshot();
    assert_eq!(s.state, PlayState::Playing);
    assert_eq!(s.current_index, Some(1));
    assert!(s.position > 0.5, "{}", s.position);

    engine.send(Command::Pause);
    turn(false);
    assert_eq!(turn(true), 0, "nothing comes out while paused");
    assert!(handle.is_paused());

    engine.send(Command::Seek(1.0));
    engine.send(Command::Play);
    turn(false);
    assert!(handle.take_flushed(), "a seek flushes the host queue");
    for _ in 0..3 {
        turn(true);
    }
    let s = engine.snapshot();
    assert_eq!(s.current_index, Some(1));
    assert!((s.position - 1.3).abs() < 0.15, "{}", s.position);

    // Audio the host still holds is not counted as played.
    turn(false);
    let before = engine.snapshot().position;
    handle.set_queued(4410);
    turn(false);
    let after = engine.snapshot().position;
    assert!((before - after - 0.1).abs() < 0.01, "{before} -> {after}");
    assert!(!handle.take_flushed());
    drop(engine);
    assert!(!pump.turn(), "shut down with the engine");
}
