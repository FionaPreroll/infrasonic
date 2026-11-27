// SPDX-License-Identifier: GPL-3.0-only
//! Crossfade, playback speed, pausing after the current song and resuming long tracks, with
//! generated test tones played through the null output.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use infrasonic_engine::output::{NullOutputFactory, OutputSpec};
use infrasonic_engine::source::StreamReader;
use infrasonic_engine::{
    Command, Engine, EngineConfig, Error, Event, MediaSource, PlayState, Track,
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

fn track(path: &Path, album: &str, number: u32, secs: f64) -> Track {
    let mut t = Track::new(
        path.to_string_lossy(),
        path.file_name().expect("file name").to_string_lossy(),
    );
    t.suffix = Some("wav".into());
    t.album = album.into();
    t.album_id = Some(album.into());
    t.track_number = Some(number);
    t.duration = Some(secs);
    t
}

fn provider() -> Arc<dyn infrasonic_engine::SourceProvider> {
    Arc::new(|t: &Track| -> Result<Box<dyn MediaSource>, Error> {
        let bytes = std::fs::read(&t.id).map_err(|e| Error::Source(e.to_string()))?;
        Ok(Box::new(StreamReader::from_bytes(bytes)))
    })
}

struct Rig {
    engine: Engine,
    capture: Arc<Mutex<Vec<f32>>>,
}

fn rig(speed: f64, config: EngineConfig) -> Rig {
    let capture = Arc::new(Mutex::new(Vec::new()));
    let out = NullOutputFactory::new(
        OutputSpec {
            sample_rate: RATE,
            channels: 2,
        },
        speed,
    )
    .with_capture(capture.clone());
    Rig {
        engine: Engine::new(config, provider(), Box::new(out)),
        capture,
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

    /// Wait until `done` says so, calling it for every event (and every 5 ms without one).
    fn until(&self, timeout: Duration, what: &str, mut done: impl FnMut(&Event) -> bool) {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            let ev = self
                .engine
                .events()
                .recv_timeout(Duration::from_millis(5))
                .unwrap_or(Event::QueueChanged);
            if done(&ev) {
                return;
            }
        }
        panic!("timed out waiting for {what}");
    }

    fn heard_secs(&self) -> f32 {
        self.capture.lock().len() as f32 / 2.0 / RATE as f32
    }

    fn left(&self) -> Vec<f32> {
        self.capture.lock().iter().step_by(2).copied().collect()
    }
}

/// Strength of `freq` in `s` (Goertzel), relative to a full test tone.
fn tone(s: &[f32], freq: f32) -> f32 {
    let w = 2.0 * std::f32::consts::PI * freq / RATE as f32;
    let (mut s1, mut s2) = (0.0f32, 0.0f32);
    for x in s {
        let s0 = x + 2.0 * w.cos() * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    let power = s1 * s1 + s2 * s2 - 2.0 * w.cos() * s1 * s2;
    power.sqrt() * 2.0 / s.len() as f32 / AMP
}

fn rms(s: &[f32]) -> f32 {
    (s.iter().map(|x| x * x).sum::<f32>() / s.len() as f32).sqrt()
}

/// No sample jumps beyond what a tone of `max_freq` does, and no loudness steps above
/// `max_step_db` between 20 ms blocks (after `from` frames).
fn assert_smooth(s: &[f32], max_freq: f32, slack: f32, max_step_db: f32, from: usize, what: &str) {
    let w = 2.0 * std::f32::consts::PI * max_freq / RATE as f32;
    let bound = AMP * 2.0 * w * w * slack;
    for (i, x) in s[from..].windows(3).enumerate() {
        let d2 = (x[2] - 2.0 * x[1] + x[0]).abs();
        assert!(
            d2 <= bound,
            "{what}: click at {:.4} s: {d2:.5} > {bound:.5}",
            (i + from) as f32 / RATE as f32
        );
    }
    let win = RATE as usize / 50;
    let levels: Vec<f32> = s[from..]
        .chunks_exact(win)
        .map(|c| rms(c).max(1e-6))
        .collect();
    for (i, l) in levels.windows(2).enumerate() {
        let step = 20.0 * (l[1] / l[0]).log10();
        assert!(
            step.abs() <= max_step_db,
            "{what}: loudness jumps {step:.2} dB at {:.3} s",
            ((i + 1) * win + from) as f32 / RATE as f32
        );
    }
}

#[test]
fn crossfade_mixes_songs_and_keeps_the_timing() {
    let dir = tempfile::tempdir().unwrap();
    let a = track(&sine_wav(dir.path(), "a.wav", 440.0, 2.0, 0), "A", 1, 2.0);
    let b = track(&sine_wav(dir.path(), "b.wav", 660.0, 2.0, 0), "B", 1, 2.0);
    let r = rig(
        3.0,
        EngineConfig {
            crossfade: 1.0,
            ..EngineConfig::default()
        },
    );
    r.play(vec![a, b.clone()]);
    let mut changed_at = None;
    r.until(Duration::from_secs(10), "the end", |ev| {
        if let Event::TrackChanged(Some(e)) = ev
            && e.track.id == b.id
        {
            changed_at = Some(r.heard_secs());
        }
        *ev == Event::QueueEnded
    });
    let heard = r.heard_secs();
    assert!((heard - 3.0).abs() < 0.02, "heard {heard} s, not 2 + 2 - 1");
    let changed = changed_at.expect("second song");
    assert!(
        (changed - 1.0).abs() < 0.1,
        "the second song counts from the start of the fade, not {changed} s"
    );
    let left = r.left();
    let at = |t: f32| &left[(t * RATE as f32) as usize..((t + 0.1) * RATE as f32) as usize];
    // Before, in the middle of and after the fade.
    assert!(tone(at(0.5), 440.0) > 0.95 && tone(at(0.5), 660.0) < 0.05);
    let (x, y) = (tone(at(1.45), 440.0), tone(at(1.45), 660.0));
    assert!((x - y).abs() < 0.1 && x > 0.6, "mid fade: {x} / {y}");
    assert!(tone(at(2.5), 660.0) > 0.95 && tone(at(2.5), 440.0) < 0.05);
    // Equal power: two different tones add up to an even loudness, without clicks.
    // (Two tones beat a little, so the loudness wobbles by a fraction of a dB.)
    assert_smooth(&left, 660.0, 2.0, 1.0, 0, "crossfade");
    let steady = rms(at(0.5));
    let mid = rms(at(1.45));
    assert!(
        (20.0 * (mid / steady).log10()).abs() < 1.0,
        "loudness in the fade {mid} vs {steady}"
    );
}

#[test]
fn crossfade_leaves_albums_gapless() {
    let dir = tempfile::tempdir().unwrap();
    let n = RATE as usize;
    // One tone cut into three songs of an album: a crossfade would be heard as a dip.
    let tracks: Vec<Track> = (0..3)
        .map(|i| {
            let p = sine_wav(dir.path(), &format!("{i}.wav"), 500.0, 1.0, i * n);
            track(&p, "Live", i as u32 + 1, 1.0)
        })
        .collect();
    let r = rig(
        3.0,
        EngineConfig {
            crossfade: 0.5,
            ..EngineConfig::default()
        },
    );
    r.play(tracks);
    r.until(Duration::from_secs(10), "the end", |ev| {
        *ev == Event::QueueEnded
    });
    let heard = r.heard_secs();
    assert!((heard - 3.0).abs() < 0.01, "heard {heard} s");
    assert_smooth(&r.left(), 500.0, 1.5, 0.3, 0, "album");
}

#[test]
fn crossfade_can_be_turned_on_and_off_while_playing() {
    let dir = tempfile::tempdir().unwrap();
    let tracks: Vec<Track> = (0..4)
        .map(|i| {
            let f = [300.0, 400.0, 300.0, 400.0][i];
            track(
                &sine_wav(dir.path(), &format!("{i}.wav"), f, 3.0, 0),
                &format!("album {i}"),
                1,
                3.0,
            )
        })
        .collect();
    let r = rig(3.0, EngineConfig::default());
    r.play(tracks.clone());
    // A change counts from the song change on, which comes long before the decoder reaches
    // the end of the song (it is only the output buffer ahead).
    let mut songs = 0;
    r.until(Duration::from_secs(10), "the end", |ev| {
        if let Event::TrackChanged(Some(_)) = ev {
            songs += 1;
            // Gapless into the second song, faded into the third, gapless into the fourth.
            let secs = if songs == 2 { 0.4 } else { 0.0 };
            r.engine.send(Command::SetCrossfade(secs));
        }
        *ev == Event::QueueEnded
    });
    assert_eq!(songs, 4);
    let heard = r.heard_secs();
    assert!(
        (heard - 11.6).abs() < 0.03,
        "heard {heard} s, one fade of 0.4 s"
    );
}

#[test]
fn speed_changes_tempo_but_not_pitch() {
    let dir = tempfile::tempdir().unwrap();
    let t = track(&sine_wav(dir.path(), "t.wav", 440.0, 3.0, 0), "A", 1, 3.0);
    let r = rig(3.0, EngineConfig::default());
    r.engine.send(Command::SetSpeed(1.5));
    r.play(vec![t]);
    let mut last_position = 0.0;
    r.until(Duration::from_secs(10), "the end", |ev| {
        last_position = f64::max(last_position, r.engine.snapshot().position);
        *ev == Event::QueueEnded
    });
    let heard = r.heard_secs();
    assert!((heard - 2.0).abs() < 0.06, "3 s at 1.5x took {heard} s");
    assert!(
        last_position > 2.5,
        "position follows the song: {last_position}"
    );
    let left = r.left();
    let mid = &left[RATE as usize / 2..RATE as usize * 3 / 2];
    let up = mid.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
    assert!((up as f32 - 440.0).abs() < 5.0, "{up} Hz");
    assert_smooth(&left, 440.0, 6.0, 1.0, 0, "1.5x");
}

#[test]
fn speed_can_change_while_playing() {
    let dir = tempfile::tempdir().unwrap();
    let t = track(&sine_wav(dir.path(), "t.wav", 330.0, 4.0, 0), "A", 1, 4.0);
    let r = rig(3.0, EngineConfig::default());
    r.play(vec![t]);
    let mut step = 0;
    r.until(Duration::from_secs(10), "the end", |ev| {
        {
            let position = r.engine.snapshot().position;
            let want = match position {
                p if p > 3.0 => 1.0,
                p if p > 2.0 => 0.75,
                p if p > 1.0 => 2.0,
                _ => 1.0,
            };
            if want != [1.0, 2.0, 0.75, 1.0][step] {
                step += 1;
                r.engine.send(Command::SetSpeed(want));
            }
        }
        *ev == Event::QueueEnded
    });
    assert_eq!(step, 3);
    // 1 s at 1x, 1 s at 2x, 1 s at 0.75x, 1 s at 1x (the switches come a little late).
    let heard = r.heard_secs();
    let expect = 1.0 + 0.5 + 1.333 + 1.0;
    assert!(
        (heard - expect).abs() < 0.25,
        "heard {heard} s, about {expect}"
    );
    assert_smooth(&r.left(), 330.0, 6.0, 1.0, 0, "speed changes");
}

#[test]
fn pause_after_the_current_song() {
    let dir = tempfile::tempdir().unwrap();
    let tracks: Vec<Track> = (0..3)
        .map(|i| {
            track(
                &sine_wav(dir.path(), &format!("{i}.wav"), 440.0, 1.0, 0),
                &format!("album {i}"),
                1,
                1.0,
            )
        })
        .collect();
    let r = rig(
        3.0,
        EngineConfig {
            crossfade: 0.5,
            ..EngineConfig::default()
        },
    );
    r.play(tracks.clone());
    r.engine.send(Command::PauseAfterCurrent(true));
    r.until(Duration::from_secs(10), "the pause", |ev| {
        *ev == Event::StateChanged(PlayState::Paused)
    });
    let s = r.engine.snapshot();
    assert_eq!(s.current_index, Some(1), "waits at the next song");
    assert!(s.position < 0.01, "at its start: {}", s.position);
    let heard = r.heard_secs();
    assert!(
        (heard - 1.0).abs() < 0.02,
        "the first song to its end, no fade: {heard}"
    );
    // Playing on continues with the next song, and the sleep mode is over.
    r.engine.send(Command::Play);
    r.until(Duration::from_secs(10), "the end", |ev| {
        *ev == Event::QueueEnded
    });
    let heard = r.heard_secs();
    assert!((heard - 2.5).abs() < 0.03, "then with crossfade: {heard}");
}

#[test]
fn long_tracks_start_where_they_were_left() {
    let dir = tempfile::tempdir().unwrap();
    let long = track(
        &sine_wav(dir.path(), "long.wav", 200.0, 40.0, 0),
        "P",
        1,
        40.0,
    );
    let short = track(
        &sine_wav(dir.path(), "short.wav", 200.0, 5.0, 0),
        "S",
        1,
        5.0,
    );
    let r = rig(25.0, EngineConfig::default());
    r.engine.send(Command::SetResume {
        min_duration: Some(30.0),
        points: vec![(short.id.clone(), 2.0)],
    });
    // Short tracks ignore resume points.
    r.play(vec![short.clone()]);
    r.until(Duration::from_secs(10), "playing", |_| {
        r.engine.snapshot().position > 0.5
    });
    assert!(r.engine.snapshot().position < 2.0);

    r.play(vec![long.clone()]);
    r.until(Duration::from_secs(10), "20 s", |_| {
        r.engine.snapshot().position > 20.0
    });
    r.engine.send(Command::Pause);
    let mut kept = None;
    r.until(Duration::from_secs(10), "a resume point", |ev| {
        if let Event::ResumePoint { id, position } = ev
            && *id == long.id
        {
            kept = *position;
            return true;
        }
        false
    });
    let kept = kept.expect("a position");
    assert!((20.0..22.0).contains(&kept), "kept {kept}");

    // Played again from the start of the queue: it goes on where it was.
    r.play(vec![short.clone(), long.clone()]);
    r.engine.send(Command::PlayIndex(1));
    let mut on_long = false;
    let mut p = -1.0;
    r.until(Duration::from_secs(10), "playing again", |ev| {
        match ev {
            Event::TrackChanged(Some(e)) => on_long = e.track.id == long.id,
            // The first position of the opened track is where it starts.
            Event::Position { position, .. } if on_long => {
                p = *position;
                return true;
            }
            _ => {}
        }
        false
    });
    assert!((p - kept).abs() < 0.5, "starts at {p}, not {kept}");

    // Played to the end, it starts from the beginning next time.
    let mut forgot = false;
    r.until(Duration::from_secs(20), "the end", |ev| {
        if let Event::ResumePoint { id, position: None } = ev
            && *id == long.id
        {
            forgot = true;
        }
        *ev == Event::QueueEnded
    });
    assert!(forgot, "finished tracks are forgotten");
}
