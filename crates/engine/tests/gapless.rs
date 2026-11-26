// SPDX-License-Identifier: GPL-3.0-only
//! Gapless golden tests: `testdata/gapless/<format>/part{1,2,3}` are one continuous signal cut
//! into three parts of exactly 88200 frames. Decoded back to back they must reproduce the
//! lossless FLAC parts: same length, no gap, no offset at the joins.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use infrasonic_engine::decode::TrackDecoder;
use infrasonic_engine::output::{NullOutputFactory, OutputSpec};
use infrasonic_engine::{Command, Engine, EngineConfig, Error, Event, MediaSource, Track};
use parking_lot::Mutex;

const PART_FRAMES: usize = 88_200;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testdata/gapless")
}

/// Decode one file to interleaved stereo at its own rate.
fn decode(path: &std::path::Path) -> (Vec<f32>, u32) {
    let f = std::fs::File::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let ext = path.extension().and_then(|e| e.to_str());
    let mut dec = TrackDecoder::open(Box::new(f), ext).expect("open");
    let mut out = Vec::new();
    while let Some(c) = dec.next_chunk().expect("decode") {
        assert_eq!(c.channels, 2);
        out.extend_from_slice(c.samples);
    }
    (out, dec.sample_rate())
}

fn parts(ext: &str) -> Vec<(Vec<f32>, u32)> {
    (1..=3)
        .map(|n| decode(&dir().join(ext).join(format!("part{n}.{ext}"))))
        .collect()
}

fn reference() -> Vec<f32> {
    parts("flac").into_iter().flat_map(|(s, _)| s).collect()
}

/// RMS difference between `a` and `b` over `range` (frames), left channel.
fn rms(a: &[f32], b: &[f32], range: std::ops::Range<usize>) -> f64 {
    let mut sum = 0.0;
    let mut n = 0;
    for i in range {
        let (Some(x), Some(y)) = (a.get(i * 2), b.get(i * 2)) else {
            break;
        };
        sum += f64::from(x - y).powi(2);
        n += 1;
    }
    (sum / n.max(1) as f64).sqrt()
}

#[test]
fn flac_parts_are_exact() {
    for (s, rate) in parts("flac") {
        assert_eq!(rate, 44_100);
        assert_eq!(s.len() / 2, PART_FRAMES);
    }
}

#[test]
fn lossy_parts_join_without_gaps() {
    let reference = reference();
    for ext in ["mp3", "ogg", "m4a"] {
        let p = parts(ext);
        let lens: Vec<usize> = p.iter().map(|(s, _)| s.len() / 2).collect();
        let joined: Vec<f32> = p.into_iter().flat_map(|(s, _)| s).collect();
        // Error around each join compared with the error in the middle of the parts.
        let mid = rms(&joined, &reference, 40_000..48_000);
        let joins: Vec<f64> = [PART_FRAMES, 2 * PART_FRAMES]
            .iter()
            .map(|&j| rms(&joined, &reference, j - 1500..j + 1500))
            .collect();
        eprintln!("{ext}: lengths {lens:?}, rms mid {mid:.4}, joins {joins:?}");
        for l in &lens {
            assert!(
                l.abs_diff(PART_FRAMES) <= 1,
                "{ext}: part length {l} instead of {PART_FRAMES}"
            );
        }
        for j in joins {
            assert!(j < 0.02, "{ext}: join error {j} (middle {mid})");
        }
    }
}

/// Largest step between neighbouring samples (left channel) in `range` frames. A gap or a
/// misaligned join shows up as a step far above what a 660 Hz sine at -6 dBFS can make.
fn max_step(s: &[f32], range: std::ops::Range<usize>) -> f32 {
    let end = range.end.min(s.len() / 2);
    (range.start.max(1)..end)
        .map(|i| (s[i * 2] - s[(i - 1) * 2]).abs())
        .fold(0.0, f32::max)
}

#[test]
fn opus_parts_join_without_gaps() {
    let p = parts("opus");
    // ffmpeg resamples to 48 kHz before encoding: 2 s are 96000 frames.
    let lens: Vec<usize> = p.iter().map(|(s, _)| s.len() / 2).collect();
    assert!(p.iter().all(|(_, r)| *r == 48_000));
    for l in &lens {
        assert!(l.abs_diff(96_000) <= 1, "opus part length {l}: {lens:?}");
    }
    let joined: Vec<f32> = p.into_iter().flat_map(|(s, _)| s).collect();
    let normal = max_step(&joined, 10_000..90_000);
    for j in [96_000, 192_000] {
        let step = max_step(&joined, j - 2000..j + 2000);
        assert!(
            step < normal * 1.5,
            "opus join at {j}: step {step}, normal {normal}"
        );
    }
}

#[test]
fn aac_seek_respects_encoder_delay() {
    let reference = reference();
    let path = dir().join("m4a/part2.m4a");
    let f = std::fs::File::open(&path).expect("open");
    let mut dec = TrackDecoder::open(Box::new(f), Some("m4a")).expect("decoder");
    assert!((dec.duration().expect("duration") - 2.0).abs() < 1e-6);
    dec.seek(1.0).expect("seek");
    let mut out = Vec::new();
    while let Some(c) = dec.next_chunk().expect("decode") {
        out.extend_from_slice(c.samples);
    }
    assert_eq!(out.len() / 2, 44_100, "frames after seeking to 1 s");
    let want = &reference[(PART_FRAMES + 44_100) * 2..];
    let err = rms(&out, want, 2000..40_000);
    assert!(err < 0.01, "seek lands off by the encoder delay: rms {err}");
}

fn provider() -> Arc<dyn infrasonic_engine::SourceProvider> {
    Arc::new(|t: &Track| -> Result<Box<dyn MediaSource>, Error> {
        let f = std::fs::File::open(&t.id).map_err(|e| Error::Source(e.to_string()))?;
        Ok(Box::new(f))
    })
}

/// Play the three parts through the engine and return what reached the output.
fn play(ext: &str, rate: u32) -> Vec<f32> {
    play_with(ext, rate, EngineConfig::default(), None)
}

fn play_with(ext: &str, rate: u32, config: EngineConfig, album: Option<&str>) -> Vec<f32> {
    let capture = Arc::new(Mutex::new(Vec::new()));
    let spec = OutputSpec {
        sample_rate: rate,
        channels: 2,
    };
    let out = NullOutputFactory::new(spec, 50.0).with_capture(capture.clone());
    let e = Engine::new(config, provider(), Box::new(out));
    let tracks = (1..=3)
        .map(|n| {
            let p = dir().join(ext).join(format!("part{n}.{ext}"));
            let mut t = Track::new(p.to_string_lossy(), format!("part{n}"));
            t.suffix = Some(ext.into());
            t.album = album.unwrap_or_default().to_string();
            t
        })
        .collect();
    e.send(Command::SetQueue {
        tracks,
        start: 0,
        position: 0.0,
        play: true,
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        assert!(Instant::now() < deadline, "{ext}: queue did not end");
        match e.events().recv_timeout(Duration::from_millis(50)) {
            Ok(Event::QueueEnded) => break,
            Ok(Event::Error(err)) => panic!("{ext}: {err}"),
            _ => {}
        }
    }
    drop(e);
    capture.lock().clone()
}

#[test]
fn engine_plays_every_format_gaplessly() {
    let reference = reference();
    for ext in ["flac", "mp3", "ogg", "m4a"] {
        let out = play(ext, 44_100);
        assert!(
            (out.len() / 2).abs_diff(3 * PART_FRAMES) <= 3,
            "{ext}: {} frames",
            out.len() / 2
        );
        let tolerance = if ext == "flac" { 1e-6 } else { 0.02 };
        for j in [PART_FRAMES, 2 * PART_FRAMES] {
            let err = rms(&out, &reference, j - 1500..j + 1500);
            assert!(err < tolerance, "{ext}: join at {j}: rms {err}");
        }
    }
}

#[test]
fn engine_resamples_gaplessly() {
    // 44.1 kHz files on a 48 kHz device, and 48 kHz Opus on a 44.1 kHz device.
    for (ext, rate, frames) in [("flac", 48_000, 288_000), ("opus", 44_100, 264_600)] {
        let out = play(ext, rate);
        let n = out.len() / 2;
        assert!(n.abs_diff(frames) <= 64, "{ext} at {rate} Hz: {n} frames");
        let normal = max_step(&out, 10_000..60_000);
        for j in [frames / 3, 2 * frames / 3] {
            let step = max_step(&out, j - 2000..j + 2000);
            assert!(
                step < normal * 1.5,
                "{ext} at {rate} Hz: join at {j}: step {step}, normal {normal}"
            );
        }
    }
}

#[test]
fn replay_gain_tags_are_read() {
    // ffmpeg cannot write iTunes-style freeform atoms, so there is no M4A case here.
    for ext in ["flac", "mp3", "ogg", "opus"] {
        let path = dir().join(ext).join(format!("part1.{ext}"));
        let f = std::fs::File::open(&path).expect("open");
        let dec = TrackDecoder::open(Box::new(f), Some(ext)).expect("decoder");
        let rg = dec.replay_gain();
        assert_eq!(
            rg,
            Some(infrasonic_engine::ReplayGain {
                track_gain: Some(-3.0),
                track_peak: Some(0.5),
                album_gain: Some(-4.5),
                album_peak: Some(0.5),
            }),
            "{ext}"
        );
    }
}

#[test]
fn replay_gain_scales_the_output() {
    use infrasonic_engine::{ReplayGainConfig, ReplayGainMode};
    let reference = reference();
    let run = |mode, album| {
        let config = EngineConfig {
            replay_gain: ReplayGainConfig {
                mode,
                ..ReplayGainConfig::default()
            },
            ..EngineConfig::default()
        };
        let out = play_with("flac", 44_100, config, album);
        // Ratio of output to reference energy, as dB.
        let (mut a, mut b) = (0.0f64, 0.0f64);
        for (x, y) in out.iter().zip(&reference) {
            a += f64::from(*x).powi(2);
            b += f64::from(*y).powi(2);
        }
        10.0 * (a / b).log10()
    };
    let track = run(ReplayGainMode::Track, Some("A"));
    assert!((track + 3.0).abs() < 0.01, "track gain {track} dB");
    let album = run(ReplayGainMode::Auto, Some("A"));
    assert!(
        (album + 4.5).abs() < 0.01,
        "auto with one album: {album} dB"
    );
    let single = run(ReplayGainMode::Auto, None);
    assert!(
        (single + 3.0).abs() < 0.01,
        "auto without album: {single} dB"
    );
    let off = run(ReplayGainMode::Off, Some("A"));
    assert!(off.abs() < 0.001, "off: {off} dB");
}
