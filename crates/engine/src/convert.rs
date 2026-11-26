// SPDX-License-Identifier: GPL-3.0-only
//! Channel mapping and sample rate conversion to the output format.

use audioadapter_buffers::direct::InterleavedSlice;
use rubato::{
    Async, FixedAsync, Resampler, SincInterpolationParameters, SincInterpolationType,
    WindowFunction,
};

/// Map interleaved `input` with `in_ch` channels to `out_ch` channels, appending to `out`.
pub fn map_channels(input: &[f32], in_ch: usize, out_ch: usize, out: &mut Vec<f32>) {
    if in_ch == 0 || out_ch == 0 {
        return;
    }
    if in_ch == out_ch {
        out.extend_from_slice(input);
        return;
    }
    out.reserve(input.len() / in_ch * out_ch);
    for frame in input.chunks_exact(in_ch) {
        match (in_ch, out_ch) {
            (1, _) => out.extend(std::iter::repeat_n(frame[0], out_ch)),
            (_, 1) => out.push(frame.iter().sum::<f32>() / in_ch as f32),
            _ => {
                // Keep front left/right, drop the rest. Proper downmixing comes later.
                let keep = in_ch.min(out_ch);
                out.extend_from_slice(&frame[..keep]);
                out.extend(std::iter::repeat_n(0.0, out_ch - keep));
            }
        }
    }
}

const CHUNK: usize = 1024;

/// Streaming sample rate converter for interleaved audio.
///
/// Output frame `k` corresponds to input frame `k / ratio`: the filter delay is removed at
/// the start and flushed at the end, so consecutive tracks fed into one converter stay
/// sample aligned (gapless) and [`RateConverter::finish`] returns exactly
/// `round(input_frames * ratio)` frames in total.
pub struct RateConverter {
    inner: Async<f32>,
    channels: usize,
    pending: Vec<f32>,
    out: Vec<f32>,
    /// Output frames still to drop (filter delay).
    skip: usize,
    input_frames: u64,
    output_frames: u64,
}

impl std::fmt::Debug for RateConverter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RateConverter")
            .field("channels", &self.channels)
            .field("input_frames", &self.input_frames)
            .finish_non_exhaustive()
    }
}

impl RateConverter {
    /// Create a converter from `from` Hz to `to` Hz. Returns `None` if no conversion is needed.
    pub fn new(from: u32, to: u32, channels: usize) -> Option<Self> {
        if from == to || from == 0 || to == 0 || channels == 0 {
            return None;
        }
        let params = SincInterpolationParameters::new(128, WindowFunction::BlackmanHarris2)
            .oversampling_factor(128)
            .interpolation(SincInterpolationType::Linear);
        let inner = Async::<f32>::new_sinc(
            f64::from(to) / f64::from(from),
            1.0,
            &params,
            CHUNK,
            channels,
            FixedAsync::Input,
        )
        .ok()?;
        let out = vec![0.0; inner.output_frames_max() * channels];
        let skip = inner.output_delay();
        Some(Self {
            inner,
            channels,
            pending: Vec::with_capacity(CHUNK * channels * 2),
            out,
            skip,
            input_frames: 0,
            output_frames: 0,
        })
    }

    /// Output rate divided by input rate.
    pub fn ratio(&self) -> f64 {
        self.inner.resample_ratio()
    }

    /// Input frames fed so far.
    pub fn input_frames(&self) -> u64 {
        self.input_frames
    }

    /// Output frames that `input` input frames turn into.
    pub fn output_for(&self, input: u64) -> u64 {
        (input as f64 * self.ratio()).round() as u64
    }

    /// Feed interleaved input; converted samples are appended to `dst`.
    pub fn process(&mut self, input: &[f32], dst: &mut Vec<f32>) {
        self.input_frames += (input.len() / self.channels) as u64;
        self.pending.extend_from_slice(input);
        self.drain(dst);
    }

    /// Flush the remaining input (end of stream), padding with silence, and reset.
    pub fn finish(&mut self, dst: &mut Vec<f32>) {
        let ch = self.channels;
        let total = self.output_for(self.input_frames);
        let mut guard = 0;
        while self.output_frames < total && guard < 64 {
            let need = self.inner.input_frames_next();
            let have = self.pending.len() / ch;
            self.pending.resize(need.max(have) * ch, 0.0);
            self.drain(dst);
            guard += 1;
        }
        let extra = (self.output_frames.saturating_sub(total) as usize).min(dst.len() / ch);
        dst.truncate(dst.len() - extra * ch);
        self.pending.clear();
        self.inner.reset();
        self.skip = self.inner.output_delay();
        self.input_frames = 0;
        self.output_frames = 0;
    }

    fn drain(&mut self, dst: &mut Vec<f32>) {
        let ch = self.channels;
        loop {
            let need = self.inner.input_frames_next();
            if self.pending.len() < need * ch {
                return;
            }
            let out_frames = self.out.len() / ch;
            let (Ok(input), Ok(mut output)) = (
                InterleavedSlice::new(&self.pending[..need * ch], ch, need),
                InterleavedSlice::new_mut(&mut self.out[..], ch, out_frames),
            ) else {
                self.pending.clear();
                return;
            };
            match self.inner.process_into_buffer(&input, &mut output, None) {
                Ok((used, written)) => {
                    let drop = self.skip.min(written);
                    self.skip -= drop;
                    dst.extend_from_slice(&self.out[drop * ch..written * ch]);
                    self.output_frames += (written - drop) as u64;
                    self.pending.drain(..used * ch);
                }
                Err(e) => {
                    tracing::warn!("resampler error: {e}");
                    self.pending.clear();
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mono_to_stereo_duplicates() {
        let mut out = Vec::new();
        map_channels(&[0.1, 0.2], 1, 2, &mut out);
        assert_eq!(out, vec![0.1, 0.1, 0.2, 0.2]);
    }

    #[test]
    fn stereo_to_mono_averages() {
        let mut out = Vec::new();
        map_channels(&[0.2, 0.4, -1.0, 1.0], 2, 1, &mut out);
        assert_eq!(out, vec![0.3, 0.0]);
    }

    #[test]
    fn surround_to_stereo_keeps_front() {
        let mut out = Vec::new();
        map_channels(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], 6, 2, &mut out);
        assert_eq!(out, vec![1.0, 2.0]);
    }

    #[test]
    fn same_rate_needs_no_converter() {
        assert!(RateConverter::new(48_000, 48_000, 2).is_none());
    }

    #[test]
    fn resampling_preserves_length_and_frequency() {
        let from = 44_100u32;
        let to = 48_000u32;
        let secs = 1.0;
        let n = (f64::from(from) * secs) as usize;
        let input: Vec<f32> = (0..n)
            .map(|i| (i as f32 * 440.0 * std::f32::consts::TAU / from as f32).sin() * 0.5)
            .collect();
        let mut rc = RateConverter::new(from, to, 1).expect("converter");
        let mut out = Vec::new();
        for c in input.chunks(777) {
            rc.process(c, &mut out);
        }
        rc.finish(&mut out);
        let expected = (f64::from(to) * secs) as isize;
        assert!(
            (out.len() as isize - expected).abs() <= 1,
            "got {} frames, expected about {expected}",
            out.len()
        );
        // Count zero crossings in the middle: 440 Hz -> about 880 per second.
        let mid = &out[4800..out.len() - 4800];
        let crossings = mid.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        let seconds = mid.len() as f64 / f64::from(to);
        let freq = crossings as f64 / seconds;
        assert!((freq - 440.0).abs() < 5.0, "frequency {freq}");
    }

    #[test]
    fn resampling_is_time_aligned() {
        // A sine resampled must match the same sine generated at the target rate, from the
        // very first frames on (no filter delay left in the output).
        let (from, to) = (44_100u32, 48_000u32);
        let f = 440.0f64;
        let sine = |rate: u32, n: usize| -> Vec<f32> {
            (0..n)
                .map(|i| (i as f64 * f * std::f64::consts::TAU / f64::from(rate)).sin() as f32)
                .collect()
        };
        let input = sine(from, 44_100);
        let mut rc = RateConverter::new(from, to, 1).expect("converter");
        let mut out = Vec::new();
        for c in input.chunks(1000) {
            rc.process(c, &mut out);
        }
        rc.finish(&mut out);
        assert_eq!(out.len(), 48_000);
        let want = sine(to, 48_000);
        let err = out[2000..46_000]
            .iter()
            .zip(&want[2000..46_000])
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        // The filter delay is a fraction of a frame off at most (0.02 here); a whole frame
        // of misalignment would be above 0.035.
        assert!(err < 0.025, "max error {err}");
    }
}
