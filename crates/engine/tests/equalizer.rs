// SPDX-License-Identifier: GPL-3.0-only
//! The equalizer with generated test tones: its frequency response, and that switching it on,
//! off and between presets, at song changes and with a reopened output device, never clicks.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use infrasonic_engine::eq::BANDS;
use infrasonic_engine::output::{
    AudioOutput, NullOutputFactory, OutputFactory, OutputShared, OutputSpec, Producer,
};
use infrasonic_engine::source::StreamReader;
use infrasonic_engine::{
    Command, Engine, EngineConfig, Equalizer, Error, Event, MediaSource, PlayState, Track,
};
use parking_lot::Mutex;

const RATE: u32 = 48_000;
const AMP: f32 = 0.25;

/// A stereo 32-bit float WAV with a sine at `freq`, starting at phase `frames_before`.
fn sine_wav(dir: &Path, name: &str, freq: f32, secs: f32, frames_before: usize) -> PathBuf {
    let path = dir.join(name);
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: RATE,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut w = hound::WavWriter::create(&path, spec).expect("wav");
    let frames = (secs * RATE as f32) as usize;
    for i in frames_before..frames_before + frames {
        let s = AMP * (2.0 * std::f32::consts::PI * freq * i as f32 / RATE as f32).sin();
        w.write_sample(s).expect("sample");
        w.write_sample(s).expect("sample");
    }
    w.finalize().expect("finalize");
    path
}

fn track(path: &Path) -> Track {
    let mut t = Track::new(
        path.to_string_lossy(),
        path.file_name().expect("file name").to_string_lossy(),
    );
    t.suffix = Some("wav".into());
    t
}

fn provider() -> Arc<dyn infrasonic_engine::SourceProvider> {
    Arc::new(|t: &Track| -> Result<Box<dyn MediaSource>, Error> {
        let bytes = std::fs::read(&t.id).map_err(|e| Error::Source(e.to_string()))?;
        Ok(Box::new(StreamReader::from_bytes(bytes)))
    })
}

const SPEC: OutputSpec = OutputSpec {
    sample_rate: RATE,
    channels: 2,
};

/// An output that records what it plays, keeps the state of the stream it opened last (so a
/// test can make the device fail) and notes where in the recording each stream started.
struct Recorder {
    inner: NullOutputFactory,
    capture: Arc<Mutex<Vec<f32>>>,
    last: Arc<Mutex<Option<Arc<OutputShared>>>>,
    starts: Arc<Mutex<Vec<usize>>>,
}

impl OutputFactory for Recorder {
    fn open(
        &self,
        shared: Arc<OutputShared>,
        buffer: Duration,
    ) -> Result<(Box<dyn AudioOutput>, Producer<f32>), Error> {
        self.starts.lock().push(self.capture.lock().len() / 2);
        *self.last.lock() = Some(shared.clone());
        self.inner.open(shared, buffer)
    }
}

struct Rig {
    engine: Engine,
    capture: Arc<Mutex<Vec<f32>>>,
    last: Arc<Mutex<Option<Arc<OutputShared>>>>,
    starts: Arc<Mutex<Vec<usize>>>,
}

fn rig(speed: f64) -> Rig {
    let capture = Arc::new(Mutex::new(Vec::new()));
    let last = Arc::new(Mutex::new(None));
    let starts = Arc::new(Mutex::new(Vec::new()));
    let out = Recorder {
        inner: NullOutputFactory::new(SPEC, speed).with_capture(capture.clone()),
        capture: capture.clone(),
        last: last.clone(),
        starts: starts.clone(),
    };
    let engine = Engine::new(EngineConfig::default(), provider(), Box::new(out));
    Rig {
        engine,
        capture,
        last,
        starts,
    }
}

impl Rig {
    fn play(&self, tracks: Vec<Track>) {
        self.engine.send(Command::SetQueue {
            tracks,
            start: 0,
            position: 0.0,
            play: true,
        });
    }

    /// Wait for the end of the queue, calling `each` for every event meanwhile.
    fn until_end(&self, timeout: Duration, mut each: impl FnMut(&Event)) {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if let Ok(ev) = self.engine.events().recv_timeout(Duration::from_millis(5)) {
                if ev == Event::QueueEnded {
                    return;
                }
                each(&ev);
            } else {
                each(&Event::QueueChanged);
            }
        }
        panic!("queue did not end in {timeout:?}");
    }

    fn left(&self) -> Vec<f32> {
        self.capture.lock().iter().step_by(2).copied().collect()
    }
}

/// Level in dB of `s` relative to the test tone.
fn level_db(s: &[f32]) -> f32 {
    let rms = (s.iter().map(|x| x * x).sum::<f32>() / s.len() as f32).sqrt();
    20.0 * (rms / (AMP / std::f32::consts::SQRT_2)).log10()
}

/// Checks a recorded sine of `freq` for clicks: the second difference of a sine is bounded
/// by its amplitude times (2πf/rate)², a jump in the signal is far above that; and its
/// loudness (in 2 ms steps) changes at most `max_step_db` from one step to the next.
fn assert_smooth(s: &[f32], freq: f32, max_step_db: f32, what: &str) {
    assert_smooth_from(s, freq, max_step_db, 0, what);
}

/// [`assert_smooth`], with the loudness compared from frame `level_from` on (after a fade in).
fn assert_smooth_from(s: &[f32], freq: f32, max_step_db: f32, level_from: usize, what: &str) {
    let w = 2.0 * std::f32::consts::PI * freq / RATE as f32;
    let bound = AMP * 2.0 * w * w * 1.5;
    for (i, x) in s.windows(3).enumerate() {
        let d2 = (x[2] - 2.0 * x[1] + x[0]).abs();
        assert!(
            d2 <= bound,
            "{what}: click at frame {i} ({:.3} s): second difference {d2:.5} > {bound:.5}",
            i as f32 / RATE as f32
        );
    }
    let win = (RATE / 500) as usize;
    let levels: Vec<f32> = s[level_from..]
        .chunks_exact(win)
        .map(|c| {
            (c.iter().map(|x| x * x).sum::<f32>() / win as f32)
                .sqrt()
                .max(1e-6)
        })
        .collect();
    for (i, l) in levels.windows(2).enumerate() {
        let step = 20.0 * (l[1] / l[0]).log10();
        assert!(
            step.abs() <= max_step_db || (l[0] < AMP * 0.02 && l[1] < AMP * 0.02),
            "{what}: loudness jumps {step:.1} dB at {:.3} s",
            ((i + 1) * win + level_from) as f32 / RATE as f32
        );
    }
}

fn eq(gains: [f32; 10]) -> Option<Equalizer> {
    Some(Equalizer::new(0.0, &gains))
}

#[test]
fn frequency_response_matches_the_bands() {
    let dir = tempfile::tempdir().unwrap();
    let tracks: Vec<Track> = BANDS
        .iter()
        .map(|f| track(&sine_wav(dir.path(), &format!("{f}.wav"), *f, 0.6, 0)))
        .collect();
    let presets: [[f32; 10]; 4] = [
        [0.0; 10],
        [6.0, 0.0, -6.0, 0.0, 3.0, -3.0, 0.0, 6.0, -6.0, 0.0],
        [-12.0; 10],
        [12.0, 9.0, 6.0, 3.0, 0.0, 0.0, -3.0, -6.0, -9.0, -12.0],
    ];
    for gains in presets {
        let r = rig(40.0);
        r.engine.send(Command::SetEqualizer(eq(gains)));
        r.play(tracks.clone());
        r.until_end(Duration::from_secs(20), |_| {});
        let left = r.left();
        let seg = (0.6 * RATE as f32) as usize;
        let headroom = Equalizer::new(0.0, &gains).headroom_db();
        for (i, want) in gains.iter().enumerate() {
            // The middle of each tone, away from the filters settling.
            let part = &left[i * seg + seg / 4..i * seg + seg * 3 / 4];
            let got = level_db(part);
            let want = want + headroom;
            assert!(
                (got - want).abs() < 0.75,
                "preset {gains:?}: {} Hz at {got:.2} dB, wanted {want:.2} dB",
                BANDS[i]
            );
        }
    }
}

#[test]
fn switching_while_playing_does_not_click() {
    let dir = tempfile::tempdir().unwrap();
    let freq = 1000.0;
    let file = sine_wav(dir.path(), "tone.wav", freq, 6.0, 0);
    let cut = {
        let mut g = [0.0; 10];
        g[5] = -12.0;
        g
    };
    let boost = {
        let mut g = [0.0; 10];
        g[5] = 9.0;
        g[4] = 4.0;
        g
    };
    // Off, presets, flat, and changes faster than a crossfade lasts.
    let steps: Vec<(u64, Option<Equalizer>)> = vec![
        (150, eq(cut)),
        (150, None),
        (150, eq(boost)),
        (150, eq([0.0; 10])),
        (150, eq(cut)),
        (5, eq(boost)),
        (5, None),
        (5, eq(cut)),
        (150, None),
        (150, eq(boost)),
    ];
    let r = rig(4.0);
    r.play(vec![track(&file)]);
    let mut steps = steps.into_iter();
    let mut next = Instant::now() + Duration::from_millis(100);
    let mut pending = steps.next();
    r.until_end(Duration::from_secs(10), |_| {
        if Instant::now() >= next
            && let Some((wait, e)) = pending.take()
        {
            r.engine.send(Command::SetEqualizer(e));
            next = Instant::now() + Duration::from_millis(wait);
            pending = steps.next();
        }
    });
    assert!(pending.is_none(), "not every change was made");
    let left = r.left();
    assert!((left.len() as f32 / RATE as f32 - 6.0).abs() < 0.01);
    assert_smooth(&left, freq, 1.5, "switching presets");
}

#[test]
fn switching_at_song_changes_does_not_click() {
    let dir = tempfile::tempdir().unwrap();
    let freq = 500.0;
    // One continuous tone cut into four songs: played gaplessly it has no seams.
    let secs = 1.0;
    let n = (secs * RATE as f32) as usize;
    let tracks: Vec<Track> = (0..4)
        .map(|i| {
            track(&sine_wav(
                dir.path(),
                &format!("{i}.wav"),
                freq,
                secs,
                i * n,
            ))
        })
        .collect();
    let presets = {
        let mut a = [0.0; 10];
        a[4] = -9.0;
        let mut b = [0.0; 10];
        b[4] = 6.0;
        [eq(a), None, eq(b), eq([0.0; 10])]
    };
    let r = rig(2.0);
    r.play(tracks);
    let mut i = 0;
    r.until_end(Duration::from_secs(10), |ev| {
        if let Event::TrackChanged(Some(_)) = ev {
            // Right at the song change, as when a preset follows the song.
            r.engine
                .send(Command::SetEqualizer(presets[i % presets.len()]));
            i += 1;
        }
    });
    assert_eq!(i, 4);
    let left = r.left();
    assert!(
        (left.len() as f32 / RATE as f32 - 4.0).abs() < 0.01,
        "played {} s",
        left.len() as f32 / RATE as f32
    );
    // From 10 ms on: the first song starts on a full tone, which the filters take a moment to
    // settle on.
    assert_smooth_from(
        &left,
        freq,
        1.5,
        RATE as usize / 100,
        "presets at song changes",
    );
}

#[test]
fn equalizer_survives_a_new_output_device() {
    let dir = tempfile::tempdir().unwrap();
    let freq = 1000.0;
    let file = sine_wav(dir.path(), "tone.wav", freq, 3.0, 0);
    let mut cut = [0.0; 10];
    cut[5] = -12.0;
    let r = rig(2.0);
    r.engine.send(Command::SetEqualizer(eq(cut)));
    r.play(vec![track(&file)]);
    let mut failed = false;
    r.until_end(Duration::from_secs(10), |ev| {
        if !failed
            && let Event::Position { position, .. } = ev
            && *position > 1.0
        {
            // The device goes away; the engine opens another one.
            if let Some(s) = r.last.lock().as_ref() {
                s.mark_failed();
            }
            failed = true;
        }
    });
    assert!(failed);
    let starts = r.starts.lock().clone();
    assert_eq!(starts.len(), 2, "the output was not opened again");
    let left = r.left();
    // Nothing is played twice or skipped (the old device's unplayed buffer is not counted):
    // the new device goes on from where the old one got, not from the engine's last turn
    // (which repeated up to a turn of the loop, and more when the engine thread was slow).
    let secs = left.len() as f32 / RATE as f32;
    assert!((secs - 3.0).abs() < 0.005, "played {secs} s");
    // The new device starts quietly and plays on with the equalizer.
    let new = &left[starts[1]..];
    assert!(new[0].abs() < AMP * 0.05, "new device starts at {}", new[0]);
    assert_smooth_from(new, freq, 1.5, RATE as usize / 100, "after the new device");
    let level = level_db(&new[RATE as usize / 4..RATE as usize / 2]);
    assert!(
        (level + 12.0).abs() < 0.75,
        "equalizer after the switch: {level} dB"
    );
    assert_eq!(r.engine.snapshot().state, PlayState::Stopped);
}

#[test]
fn seek_and_pause_do_not_click() {
    let dir = tempfile::tempdir().unwrap();
    let freq = 440.0;
    let file = sine_wav(dir.path(), "tone.wav", freq, 4.0, 0);
    let r = rig(2.0);
    r.engine.send(Command::SetEqualizer(eq([3.0; 10])));
    r.play(vec![track(&file)]);
    let started = Instant::now();
    let mut step = 0;
    r.until_end(Duration::from_secs(10), |_| {
        let t = started.elapsed();
        if step == 0 && t > Duration::from_millis(300) {
            r.engine.send(Command::Pause);
            step += 1;
        } else if step == 1 && t > Duration::from_millis(500) {
            r.engine.send(Command::Play);
            step += 1;
        } else if step == 2 && t > Duration::from_millis(700) {
            r.engine.send(Command::Seek(2.5));
            step += 1;
        }
    });
    assert_eq!(step, 3);
    let left = r.left();
    // A seek jumps within the tone: the old part fades out and the new one fades in, so the
    // second difference stays bounded; the loudness changes by the fade.
    assert_smooth(&left, freq, 40.0, "seek and pause");
}
