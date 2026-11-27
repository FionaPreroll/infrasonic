// SPDX-License-Identifier: GPL-3.0-only
//! Playback speed without a change in pitch: WSOLA (waveform similarity overlap-add).
//!
//! The output is built from overlapping windows of the input. Each window starts near where
//! the playback speed says, at the offset whose waveform continues the previous window best,
//! so that the overlaps add up without phase cancellation. At speed 1 the input passes
//! through untouched, and the switches between the two are seamless: starting continues the
//! last output sample, and stopping ends on a window that lines up with the input exactly.

/// Slowest and fastest speed.
pub const MIN_SPEED: f32 = 0.5;
pub const MAX_SPEED: f32 = 3.0;

/// Window length and search range in seconds.
const WINDOW_SECS: f64 = 0.024;
const SEEK_SECS: f64 = 0.008;

/// Clamp a speed to the supported range; 1 for values that are not finite.
pub fn clamp_speed(speed: f32) -> f32 {
    if speed.is_finite() {
        speed.clamp(MIN_SPEED, MAX_SPEED)
    } else {
        1.0
    }
}

fn hop_for(rate: u32) -> usize {
    ((f64::from(rate) * WINDOW_SECS / 2.0) as usize).max(16)
}

#[derive(Debug)]
pub struct Tempo {
    channels: usize,
    /// Window length and hop in frames (`hop` = half a window).
    window: usize,
    hop: usize,
    seek: usize,
    /// Periodic Hann window: two halves a hop apart add up to one.
    win: Vec<f32>,
    speed: f32,
    /// Stretching (or finishing a stretch); false = input passes through.
    active: bool,
    /// Leave stretching at the next hop, because the speed is 1 again.
    leaving: bool,
    /// Input frames not dropped yet, interleaved.
    input: Vec<f32>,
    /// Where the next window should start by the speed, in frames of `input`.
    pos: f64,
    /// Start of the window that would continue the last one seamlessly.
    natural: usize,
    /// Overlap-add buffer: the second half of the last window.
    ola: Vec<f32>,
    /// Output of the last hop not handed out yet.
    out: Vec<f32>,
    out_pos: usize,
    /// Input frames passed on after leaving, before the ring buffer is read again.
    drain: usize,
    /// Source frames heard, with the fraction carried over.
    consumed: f64,
    /// Input frames pulled and source frames reported as heard, in all.
    pulled: u64,
    reported: u64,
}

impl Tempo {
    pub fn new(rate: u32, channels: usize) -> Self {
        let channels = channels.max(1);
        let hop = hop_for(rate);
        let window = hop * 2;
        let seek = ((f64::from(rate) * SEEK_SECS) as usize).max(4);
        let win = (0..window)
            .map(|i| {
                let x = std::f32::consts::PI * i as f32 / window as f32;
                x.sin() * x.sin()
            })
            .collect();
        Self {
            channels,
            window,
            hop,
            seek,
            win,
            speed: 1.0,
            active: false,
            leaving: false,
            input: Vec::new(),
            pos: 0.0,
            natural: 0,
            ola: Vec::new(),
            out: Vec::new(),
            out_pos: 0,
            drain: 0,
            consumed: 0.0,
            pulled: 0,
            reported: 0,
        }
    }

    /// Whether this stretcher is for a stream with this format.
    pub fn fits(&self, rate: u32, channels: usize) -> bool {
        hop_for(rate) == self.hop && channels.max(1) == self.channels
    }

    pub fn speed(&self) -> f32 {
        self.speed
    }

    pub fn set_speed(&mut self, speed: f32) {
        self.speed = clamp_speed(speed);
        if self.speed != 1.0 {
            self.leaving = false;
        } else if self.active {
            self.leaving = true;
        }
    }

    /// Drop everything buffered (after a flush).
    pub fn reset(&mut self) {
        self.active = false;
        self.leaving = false;
        self.input.clear();
        self.out.clear();
        self.out_pos = 0;
        self.drain = 0;
        self.consumed = 0.0;
        self.pulled = 0;
        self.reported = 0;
    }

    /// The input ended for now (end of the queue): hand out what is buffered without waiting
    /// for more input, ending on the input itself as when the speed goes back to 1.
    pub fn finish(&mut self) {
        if !self.active {
            return;
        }
        let ch = self.channels;
        let need = self.natural + self.window;
        let have = self.input.len() / ch;
        let padded = need.saturating_sub(have);
        self.input.resize(need.max(have) * ch, 0.0);
        self.leaving = true;
        self.hop_once(&mut |_: &mut Vec<f32>, _: usize| 0);
        self.drain = self.drain.saturating_sub(padded);
        self.input.truncate(self.drain * ch);
    }

    /// Whether the input simply passes through: no stretching, nothing buffered.
    pub fn is_idle(&self) -> bool {
        self.speed == 1.0 && !self.active && self.drain == 0 && self.out_pos >= self.out.len()
    }

    /// Fill `out` (whole frames). `pull(buf, n)` appends up to `n` input frames to `buf` and
    /// returns how many it appended. Returns the output frames written and the source frames
    /// they stand for. Writes less than asked only when the input runs out.
    pub fn fill(
        &mut self,
        out: &mut [f32],
        mut pull: impl FnMut(&mut Vec<f32>, usize) -> usize,
    ) -> (usize, u64) {
        let ch = self.channels;
        let want = out.len() / ch;
        let mut done = 0;
        while done < want {
            if self.out_pos < self.out.len() {
                let n = ((self.out.len() - self.out_pos) / ch).min(want - done);
                out[done * ch..(done + n) * ch]
                    .copy_from_slice(&self.out[self.out_pos..self.out_pos + n * ch]);
                self.out_pos += n * ch;
                done += n;
                continue;
            }
            if self.drain > 0 {
                // Input buffered while stretching, passed on as it is.
                let n = self.drain.min(want - done);
                out[done * ch..(done + n) * ch].copy_from_slice(&self.input[..n * ch]);
                self.input.drain(..n * ch);
                self.drain -= n;
                self.consumed += n as f64;
                done += n;
                continue;
            }
            if !self.active {
                if self.speed == 1.0 {
                    let mut tmp = std::mem::take(&mut self.input);
                    tmp.clear();
                    let n = pull(&mut tmp, want - done);
                    self.pulled += n as u64;
                    out[done * ch..(done + n) * ch].copy_from_slice(&tmp[..n * ch]);
                    tmp.clear();
                    self.input = tmp;
                    self.consumed += n as f64;
                    done += n;
                    if n == 0 {
                        break;
                    }
                    continue;
                }
                if !self.start(&mut pull) {
                    break;
                }
            }
            if !self.hop_once(&mut pull) {
                break;
            }
        }
        let whole = if self.active {
            let w = (self.consumed.floor() as u64).min(self.pulled - self.reported);
            self.consumed -= self.consumed.floor();
            w
        } else {
            // Not stretching: everything pulled was heard but what still waits in `input`,
            // which also settles the rounding of the stretched part.
            self.consumed = 0.0;
            (self.pulled - self.drain as u64).saturating_sub(self.reported)
        };
        self.reported += whole;
        (done, whole)
    }

    /// Make sure `input` holds `frames` frames.
    fn need(
        &mut self,
        frames: usize,
        pull: &mut impl FnMut(&mut Vec<f32>, usize) -> usize,
    ) -> bool {
        let have = self.input.len() / self.channels;
        if have < frames {
            self.pulled += pull(&mut self.input, frames - have) as u64;
        }
        self.input.len() / self.channels >= frames
    }

    /// Begin stretching right after the last frame that passed through: the previous window
    /// is taken to have ended there, so its falling half covers the next hop of input.
    fn start(&mut self, pull: &mut impl FnMut(&mut Vec<f32>, usize) -> usize) -> bool {
        if !self.need(self.window + self.seek, pull) {
            return false;
        }
        let ch = self.channels;
        self.ola.clear();
        self.ola.resize(self.hop * ch, 0.0);
        for i in 0..self.hop {
            let w = self.win[self.hop + i];
            for c in 0..ch {
                self.ola[i * ch + c] = w * self.input[i * ch + c];
            }
        }
        self.natural = 0;
        self.pos = -(self.hop as f64);
        self.active = true;
        self.leaving = false;
        true
    }

    /// Produce one hop of output into `out`.
    fn hop_once(&mut self, pull: &mut impl FnMut(&mut Vec<f32>, usize) -> usize) -> bool {
        let ch = self.channels;
        let advance = f64::from(self.speed) * self.hop as f64;
        let nominal = (self.pos + advance).max(0.0);
        let leaving = self.leaving;
        let lo = (nominal as usize).saturating_sub(self.seek);
        let hi = nominal as usize + self.seek;
        let needed = if leaving {
            self.natural + self.window
        } else {
            (self.natural + self.window).max(hi + self.window)
        };
        if !self.need(needed, pull) {
            return false;
        }
        let chosen = if leaving {
            self.natural
        } else {
            self.best_match(lo, hi)
        };
        self.out.clear();
        for i in 0..self.hop {
            let w = self.win[i];
            for c in 0..ch {
                self.out
                    .push(self.ola[i * ch + c] + w * self.input[(chosen + i) * ch + c]);
            }
        }
        self.out_pos = 0;
        for i in 0..self.hop {
            let w = self.win[self.hop + i];
            for c in 0..ch {
                self.ola[i * ch + c] = w * self.input[(chosen + self.hop + i) * ch + c];
            }
        }
        self.natural = chosen + self.hop;
        self.pos = nominal;
        self.consumed += if leaving { self.hop as f64 } else { advance };
        if leaving {
            // This hop ended exactly on the input: go on with it unchanged.
            self.active = false;
            self.leaving = false;
            self.input.drain(..self.natural * ch);
            self.drain = self.input.len() / ch;
            return true;
        }
        // Drop input no later window can use.
        let keep = self.natural.min(lo);
        if keep > 0 {
            self.input.drain(..keep * ch);
            self.natural -= keep;
            self.pos -= keep as f64;
        }
        true
    }

    /// The window start in `lo..=hi` whose waveform best continues the natural continuation
    /// (normalised cross-correlation of the channel sum; coarse search, then refined).
    fn best_match(&self, lo: usize, hi: usize) -> usize {
        let ch = self.channels;
        let len = self.window;
        let mono = |f: usize| -> f32 { self.input[f * ch..f * ch + ch].iter().sum() };
        let template: Vec<f32> = (0..len)
            .step_by(2)
            .map(|i| mono(self.natural + i))
            .collect();
        let score = |start: usize| -> f32 {
            let mut dot = 0.0f32;
            let mut energy = 1e-9f32;
            for (k, t) in template.iter().enumerate() {
                let x = mono(start + 2 * k);
                dot += t * x;
                energy += x * x;
            }
            dot / energy.sqrt()
        };
        let mut best = lo;
        let mut best_score = f32::MIN;
        let mut s = lo;
        while s <= hi {
            let v = score(s);
            if v > best_score {
                best_score = v;
                best = s;
            }
            s += 4;
        }
        let from = best.saturating_sub(3).max(lo);
        for s in from..=(best + 3).min(hi) {
            let v = score(s);
            if v > best_score {
                best_score = v;
                best = s;
            }
        }
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    fn sine(freq: f32, frames: usize, ch: usize) -> Vec<f32> {
        (0..frames)
            .flat_map(|i| {
                let v = (2.0 * std::f32::consts::PI * freq * i as f32 / RATE as f32).sin() * 0.5;
                std::iter::repeat_n(v, ch)
            })
            .collect()
    }

    /// Run `input` through at `speed`, in output blocks of `block` frames.
    fn run(t: &mut Tempo, input: &[f32], block: usize) -> (Vec<f32>, u64) {
        let ch = t.channels;
        let mut at = 0;
        let mut out = Vec::new();
        let mut consumed = 0;
        let mut buf = vec![0.0; block * ch];
        let mut finished = false;
        loop {
            let (n, c) = t.fill(&mut buf, |dst, k| {
                let n = k.min((input.len() - at) / ch);
                dst.extend_from_slice(&input[at..at + n * ch]);
                at += n * ch;
                n
            });
            consumed += c;
            out.extend_from_slice(&buf[..n * ch]);
            if n < block {
                if at < input.len() || finished {
                    return (out, consumed);
                }
                t.finish();
                finished = true;
            }
        }
    }

    fn crossings(x: &[f32]) -> usize {
        x.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count()
    }

    #[test]
    fn speed_one_passes_through() {
        let mut t = Tempo::new(RATE, 2);
        let input = sine(440.0, 10_000, 2);
        let (out, consumed) = run(&mut t, &input, 333);
        assert_eq!(out, input);
        assert_eq!(consumed, 10_000);
    }

    #[test]
    fn faster_and_slower_keep_the_pitch() {
        for speed in [0.5f32, 0.8, 1.25, 1.5, 2.0, 3.0] {
            let mut t = Tempo::new(RATE, 1);
            t.set_speed(speed);
            let frames = RATE as usize * 2;
            let input = sine(440.0, frames, 1);
            let (out, consumed) = run(&mut t, &input, 512);
            let secs = out.len() as f32 / RATE as f32;
            let expect = 2.0 / speed;
            // The input still buffered at the end plays at 1x.
            let tol = 0.02 + (WINDOW_SECS + SEEK_SECS) as f32 * (1.0 / speed - 1.0).abs();
            assert!(
                (secs - expect).abs() < tol,
                "{speed}x: {secs} s, not {expect}"
            );
            let hz = crossings(&out) as f32 / secs;
            assert!((hz - 440.0).abs() < 4.0, "{speed}x: {hz} Hz");
            assert_eq!(
                consumed, frames as u64,
                "{speed}x: all input counts as heard"
            );
            // A steady tone stays steady: no dips where windows overlap (up to the end, where
            // the input stops).
            let step = RATE as usize / 100;
            for (i, w) in out[step..out.len() - 6 * step].chunks(step).enumerate() {
                let rms = (w.iter().map(|x| x * x).sum::<f32>() / w.len() as f32).sqrt();
                assert!((rms - 0.3536).abs() < 0.03, "{speed}x block {i}: rms {rms}");
            }
        }
    }

    #[test]
    fn switching_speed_is_seamless() {
        let mut t = Tempo::new(RATE, 2);
        let input = sine(300.0, RATE as usize * 3, 2);
        let mut at = 0;
        let mut out = Vec::new();
        let mut buf = vec![0.0; 256 * 2];
        let mut blocks = 0;
        loop {
            // 1x, then 1.5x, then 1x again, switching in the middle of hops.
            t.set_speed(match blocks {
                0..=60 => 1.0,
                61..=200 => 1.5,
                _ => 1.0,
            });
            let (n, _) = t.fill(&mut buf, |dst, k| {
                let n = k.min((input.len() - at) / 2);
                dst.extend_from_slice(&input[at..at + n * 2]);
                at += n * 2;
                n
            });
            out.extend_from_slice(&buf[..n * 2]);
            blocks += 1;
            if n < 256 {
                break;
            }
        }
        assert!(t.is_idle(), "back to passing through");
        let left: Vec<f32> = out.iter().step_by(2).copied().collect();
        // No sample-to-sample jump beyond what a 300 Hz tone does.
        let w = 2.0 * std::f32::consts::PI * 300.0 / RATE as f32;
        let limit = 0.5 * w * w * 1.5 + 1e-4;
        for (i, s) in left.windows(3).enumerate() {
            let d2 = (s[0] - 2.0 * s[1] + s[2]).abs();
            assert!(d2 < limit * 6.0, "jump at {i}: {d2}");
        }
        // The input was used up, and the part after the switch back is untouched input.
        let tail = &out[out.len() - 2000..];
        let src = &input[input.len() - 2000..];
        assert_eq!(tail, src);
    }

    #[test]
    fn bad_speeds_are_clamped() {
        assert_eq!(clamp_speed(f32::NAN), 1.0);
        assert_eq!(clamp_speed(10.0), MAX_SPEED);
        assert_eq!(clamp_speed(0.0), MIN_SPEED);
    }
}
