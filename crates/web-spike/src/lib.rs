// SPDX-License-Identifier: GPL-3.0-only
//! Web spike: decode and resample a file with the engine's code inside WebAssembly.
//!
//! The page in `www/` fetches the test files, calls [`decode`] and plays the result through
//! WebAudio. `scripts/web-spike.sh` builds it and checks it in headless Chromium.

use infrasonic_engine::convert::{RateConverter, map_channels};
use infrasonic_engine::decode::TrackDecoder;
use infrasonic_engine::source::StreamReader;
use wasm_bindgen::prelude::*;

/// Decoded audio, interleaved stereo at the requested rate.
#[wasm_bindgen]
#[derive(Debug)]
pub struct Decoded {
    source_rate: u32,
    source_channels: usize,
    rate: u32,
    samples: Vec<f32>,
}

#[wasm_bindgen]
impl Decoded {
    #[wasm_bindgen(getter)]
    pub fn source_rate(&self) -> u32 {
        self.source_rate
    }
    #[wasm_bindgen(getter)]
    pub fn source_channels(&self) -> usize {
        self.source_channels
    }
    #[wasm_bindgen(getter)]
    pub fn rate(&self) -> u32 {
        self.rate
    }
    /// Length in seconds.
    #[wasm_bindgen(getter)]
    pub fn seconds(&self) -> f64 {
        self.samples.len() as f64 / 2.0 / f64::from(self.rate)
    }
    /// Highest absolute sample value.
    #[wasm_bindgen(getter)]
    pub fn peak(&self) -> f32 {
        self.samples.iter().fold(0.0f32, |m, s| m.max(s.abs()))
    }
    /// One channel (0 or 1), for an `AudioBuffer`.
    pub fn channel(&self, ch: usize) -> Vec<f32> {
        self.samples
            .iter()
            .skip(ch.min(1))
            .step_by(2)
            .copied()
            .collect()
    }
}

/// Decode a whole file (`hint` is an extension or MIME type) to stereo at `rate` Hz.
#[wasm_bindgen]
pub fn decode(bytes: Vec<u8>, hint: &str, rate: u32) -> Result<Decoded, JsError> {
    let hint = (!hint.is_empty()).then_some(hint);
    let mut dec = TrackDecoder::open(Box::new(StreamReader::from_bytes(bytes)), hint)
        .map_err(|e| JsError::new(&e.to_string()))?;
    let (source_rate, source_channels) = (dec.sample_rate(), dec.channels());
    let mut conv = RateConverter::new(source_rate, rate, 2);
    let mut stereo = Vec::new();
    let mut samples = Vec::new();
    while let Some(chunk) = dec.next_chunk().map_err(|e| JsError::new(&e.to_string()))? {
        stereo.clear();
        map_channels(chunk.samples, chunk.channels, 2, &mut stereo);
        match &mut conv {
            Some(c) => c.process(&stereo, &mut samples),
            None => samples.extend_from_slice(&stereo),
        }
    }
    if let Some(c) = &mut conv {
        c.finish(&mut samples);
    }
    Ok(Decoded {
        source_rate,
        source_channels,
        rate,
        samples,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn garbage_is_an_error() {
        // JsError only works on wasm; check the decoder path natively.
        let r = infrasonic_engine::decode::TrackDecoder::open(
            Box::new(infrasonic_engine::source::StreamReader::from_bytes(vec![
                0;
                64
            ])),
            Some("mp3"),
        );
        assert!(r.is_err());
    }
}
