// SPDX-License-Identifier: GPL-3.0-only
//! A ten band graphic equalizer: peaking filters (RBJ cookbook biquads) at the usual octave
//! centers, applied in the output callback so that a change is heard at once.

/// Center frequencies of the bands in Hz.
pub const BANDS: [f32; 10] = [
    31.0, 62.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0,
];
/// The range of a band's gain and of the preamp in dB.
pub const MAX_GAIN_DB: f32 = 12.0;
/// Bandwidth of each band: one octave.
const Q: f64 = std::f64::consts::SQRT_2;

/// Equalizer settings: a gain per band of [`BANDS`] and a preamp, all in dB.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Equalizer {
    pub preamp_db: f32,
    pub gains_db: [f32; 10],
}

impl Default for Equalizer {
    fn default() -> Self {
        Self {
            preamp_db: 0.0,
            gains_db: [0.0; 10],
        }
    }
}

impl Equalizer {
    /// From a list of gains (missing bands are flat); values are clamped to ±[`MAX_GAIN_DB`].
    pub fn new(preamp_db: f32, gains_db: &[f32]) -> Self {
        let clamp = |g: f32| {
            if g.is_finite() {
                g.clamp(-MAX_GAIN_DB, MAX_GAIN_DB)
            } else {
                0.0
            }
        };
        let mut gains = [0.0; 10];
        for (g, v) in gains.iter_mut().zip(gains_db) {
            *g = clamp(*v);
        }
        Self {
            preamp_db: clamp(preamp_db),
            gains_db: gains,
        }
    }

    /// The gain applied before the filters: the preamp, lowered by the largest boost so that
    /// a boosted band does not clip.
    pub fn headroom_db(&self) -> f32 {
        let boost = self.gains_db.iter().copied().fold(0.0f32, f32::max);
        self.preamp_db - boost
    }
}

#[derive(Debug, Clone, Copy)]
struct Coeffs {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl Coeffs {
    /// RBJ peaking filter; at 0 dB its response is exactly flat.
    fn peaking(rate: f64, freq: f64, gain_db: f64) -> Self {
        let a = 10f64.powf(gain_db / 40.0);
        let w0 = 2.0 * std::f64::consts::PI * freq / rate;
        let alpha = w0.sin() / (2.0 * Q);
        let cos = w0.cos();
        let a0 = 1.0 + alpha / a;
        Self {
            b0: ((1.0 + alpha * a) / a0) as f32,
            b1: (-2.0 * cos / a0) as f32,
            b2: ((1.0 - alpha * a) / a0) as f32,
            a1: (-2.0 * cos / a0) as f32,
            a2: ((1.0 - alpha / a) / a0) as f32,
        }
    }

    /// Magnitude response in dB at `freq`.
    fn response_db(rate: f64, center: f64, gain_db: f64, freq: f64) -> f64 {
        let c = Self::peaking(rate, center, gain_db);
        let w = 2.0 * std::f64::consts::PI * freq / rate;
        let (cos1, sin1, cos2, sin2) = (w.cos(), w.sin(), (2.0 * w).cos(), (2.0 * w).sin());
        let (b0, b1, b2) = (f64::from(c.b0), f64::from(c.b1), f64::from(c.b2));
        let (a1, a2) = (f64::from(c.a1), f64::from(c.a2));
        let num = (b0 + b1 * cos1 + b2 * cos2).hypot(b1 * sin1 + b2 * sin2);
        let den = (1.0 + a1 * cos1 + a2 * cos2).hypot(a1 * sin1 + a2 * sin2);
        20.0 * (num / den).log10()
    }
}

/// Neighboring bands overlap, so a filter's gain is not what its band ends up with. Solve for
/// the filter gains that give the wanted gain at every center frequency (the interaction
/// matrix of the bands, refined twice because the filters are not linear in dB).
fn filter_gains(rate: f64, target: &[f32; 10], bands: usize) -> [f64; 10] {
    const PROBE_DB: f64 = 12.0;
    let freqs = &BANDS[..bands];
    let total = |g: &[f64; 10], f: f64| -> f64 {
        freqs
            .iter()
            .zip(g)
            .map(|(c, g)| Coeffs::response_db(rate, f64::from(*c), *g, f))
            .sum()
    };
    let mut m = vec![vec![0.0; bands]; bands];
    for (i, fi) in freqs.iter().enumerate() {
        for (j, fj) in freqs.iter().enumerate() {
            m[i][j] =
                Coeffs::response_db(rate, f64::from(*fj), PROBE_DB, f64::from(*fi)) / PROBE_DB;
        }
    }
    let want: Vec<f64> = target[..bands].iter().map(|g| f64::from(*g)).collect();
    let mut g = [0.0; 10];
    let mut miss = want.clone();
    for _ in 0..3 {
        let step = solve(&m, &miss);
        for (g, d) in g.iter_mut().zip(step) {
            *g += d;
        }
        for (i, f) in freqs.iter().enumerate() {
            miss[i] = want[i] - total(&g, f64::from(*f));
        }
    }
    g
}

/// Gaussian elimination with partial pivoting for the small matrix of [`filter_gains`].
fn solve(m: &[Vec<f64>], y: &[f64]) -> Vec<f64> {
    let n = y.len();
    let mut a: Vec<Vec<f64>> = m
        .iter()
        .zip(y)
        .map(|(row, y)| row.iter().copied().chain([*y]).collect())
        .collect();
    for c in 0..n {
        let p = (c..n)
            .max_by(|x, z| a[*x][c].abs().total_cmp(&a[*z][c].abs()))
            .unwrap_or(c);
        a.swap(c, p);
        if a[c][c].abs() < 1e-12 {
            continue;
        }
        let pivot = a[c].clone();
        for (r, row) in a.iter_mut().enumerate() {
            if r != c {
                let k = row[c] / pivot[c];
                for (x, p) in row.iter_mut().zip(&pivot).skip(c) {
                    *x -= k * p;
                }
            }
        }
    }
    (0..n)
        .map(|i| {
            if a[i][i].abs() < 1e-12 {
                0.0
            } else {
                a[i][n] / a[i][i]
            }
        })
        .collect()
}

/// Filter state of one band for one channel (transposed direct form II).
#[derive(Debug, Clone, Copy, Default)]
struct State {
    z1: f32,
    z2: f32,
}

/// One set of filters for every band and channel.
#[derive(Debug, Clone)]
struct Bank {
    coeffs: Vec<Coeffs>,
    /// `channels * coeffs.len()` states.
    state: Vec<State>,
    gain: f32,
    /// Flat: the equalizer is off.
    off: bool,
}

impl Bank {
    fn new(eq: Option<&Equalizer>, rate: u32, channels: usize) -> Self {
        let rate = f64::from(rate.max(1));
        // Bands too close to the Nyquist frequency are left out (low sample rates).
        let bands = BANDS
            .iter()
            .filter(|f| f64::from(**f) < rate * 0.45)
            .count();
        let flat = Equalizer::default();
        let off = eq.is_none();
        let eq = eq.unwrap_or(&flat);
        let gains = if off {
            [0.0; 10]
        } else {
            filter_gains(rate, &eq.gains_db, bands)
        };
        let coeffs: Vec<Coeffs> = BANDS[..bands]
            .iter()
            .zip(gains)
            .map(|(f, g)| Coeffs::peaking(rate, f64::from(*f), g))
            .collect();
        Self {
            state: vec![State::default(); coeffs.len() * channels.max(1)],
            coeffs,
            gain: 10f32.powf(eq.headroom_db() / 20.0),
            off,
        }
    }

    fn run(&mut self, ch: usize, x: f32) -> f32 {
        let bands = self.coeffs.len();
        let mut x = x * self.gain;
        for (c, st) in self
            .coeffs
            .iter()
            .zip(&mut self.state[ch * bands..(ch + 1) * bands])
        {
            let y = c.b0 * x + st.z1;
            st.z1 = c.b1 * x - c.a1 * y + st.z2;
            st.z2 = c.b2 * x - c.a2 * y;
            x = y;
        }
        x
    }
}

/// How long a change of the settings is crossfaded, in seconds.
const FADE_SECS: f64 = 0.05;

/// The running equalizer of an output. A change of the settings crossfades from the old
/// filters to the new ones (which start from the old filters' state), so switching presets or
/// switching the equalizer on and off does not click. When it is off and the fade is over,
/// samples pass untouched.
#[derive(Debug)]
pub struct EqProcessor {
    rate: u32,
    channels: usize,
    current: Bank,
    /// Filters being faded out, and how far the fade is (0..fade_len).
    previous: Option<(Bank, usize)>,
    fade_len: usize,
    /// A change that came while a fade was running.
    queued: Option<Option<Equalizer>>,
}

impl EqProcessor {
    /// An equalizer for an output with this rate and channel count, starting with `eq`.
    pub fn new(eq: Option<&Equalizer>, rate: u32, channels: usize) -> Self {
        Self {
            rate,
            channels: channels.max(1),
            current: Bank::new(eq, rate, channels),
            previous: None,
            fade_len: ((f64::from(rate) * FADE_SECS) as usize).max(1),
            queued: None,
        }
    }

    /// Whether the output is `rate` / `channels` (else build a new processor).
    pub fn fits(&self, rate: u32, channels: usize) -> bool {
        self.rate == rate && self.channels == channels.max(1)
    }

    /// Fade to new settings (`None`: off).
    pub fn set(&mut self, eq: Option<&Equalizer>) {
        if self.previous.is_some() {
            self.queued = Some(eq.copied());
            return;
        }
        let mut next = Bank::new(eq, self.rate, self.channels);
        if next.state.len() == self.current.state.len() {
            next.state.copy_from_slice(&self.current.state);
        }
        let old = std::mem::replace(&mut self.current, next);
        self.previous = Some((old, 0));
    }

    /// Filter interleaved samples in place.
    pub fn process(&mut self, samples: &mut [f32]) {
        if self.current.off && self.previous.is_none() {
            return;
        }
        let channels = self.channels;
        for frame in samples.chunks_exact_mut(channels) {
            match &mut self.previous {
                None => {
                    for (ch, s) in frame.iter_mut().enumerate() {
                        *s = self.current.run(ch, *s);
                    }
                }
                Some((old, pos)) => {
                    let t = *pos as f32 / self.fade_len as f32;
                    for (ch, s) in frame.iter_mut().enumerate() {
                        let a = old.run(ch, *s);
                        let b = self.current.run(ch, *s);
                        *s = a + (b - a) * t;
                    }
                    *pos += 1;
                    if *pos >= self.fade_len {
                        self.previous = None;
                        if let Some(eq) = self.queued.take() {
                            self.set(eq.as_ref());
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Gain in dB for a sine at `freq`, measured (RMS) after the filters settled.
    fn level(eq: &Equalizer, freq: f32) -> f32 {
        let rate = 48_000;
        let mut p = EqProcessor::new(Some(eq), rate, 1);
        let input: Vec<f32> = (0..rate)
            .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / rate as f32).sin() * 0.25)
            .collect();
        let mut buf = input.clone();
        p.process(&mut buf);
        let rms = |s: &[f32]| (s.iter().map(|x| x * x).sum::<f32>() / s.len() as f32).sqrt();
        let half = rate as usize / 2;
        20.0 * (rms(&buf[half..]) / rms(&input[half..])).log10()
    }

    #[test]
    fn flat_equalizer_leaves_the_signal_alone() {
        for f in [50.0, 1000.0, 10_000.0] {
            assert!(level(&Equalizer::default(), f).abs() < 0.05);
        }
    }

    #[test]
    fn band_gain_is_heard_at_its_frequency() {
        let mut gains = [0.0; 10];
        gains[5] = -9.0; // 1 kHz
        let eq = Equalizer::new(0.0, &gains);
        assert!(
            (level(&eq, 1000.0) + 9.0).abs() < 0.5,
            "{}",
            level(&eq, 1000.0)
        );
        assert!(level(&eq, 62.0).abs() < 0.5);
        assert!(level(&eq, 16_000.0).abs() < 0.5);
    }

    #[test]
    fn boosts_do_not_clip() {
        let mut gains = [0.0; 10];
        gains[1] = 12.0;
        let eq = Equalizer::new(0.0, &gains);
        // The headroom takes the boost back: the boosted band ends up near 0 dB.
        assert!(level(&eq, 62.0).abs() < 0.7, "{}", level(&eq, 62.0));
        assert!((level(&eq, 4000.0) + 12.0).abs() < 0.7);
    }

    #[test]
    fn off_and_bad_values() {
        let mut p = EqProcessor::new(None, 44_100, 2);
        let mut s = [0.5f32, -0.5];
        p.process(&mut s);
        assert_eq!(s, [0.5, -0.5]);
        let eq = Equalizer::new(f32::NAN, &[40.0, f32::INFINITY]);
        assert_eq!(eq.preamp_db, 0.0);
        assert_eq!(eq.gains_db[0], MAX_GAIN_DB);
        assert_eq!(eq.gains_db[1], 0.0);
    }
}
