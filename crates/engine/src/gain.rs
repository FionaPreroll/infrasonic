// SPDX-License-Identifier: GPL-3.0-only
//! ReplayGain: loudness normalisation from track and album gain values.

/// ReplayGain values of a track. Gains in dB, peaks as linear sample values (1.0 = full scale).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ReplayGain {
    pub track_gain: Option<f32>,
    pub track_peak: Option<f32>,
    pub album_gain: Option<f32>,
    pub album_peak: Option<f32>,
}

impl ReplayGain {
    /// Whether any gain value is known.
    pub fn is_empty(&self) -> bool {
        self.track_gain.is_none() && self.album_gain.is_none()
    }

    /// Fill values missing in `self` from `other`.
    pub fn or(self, other: Option<ReplayGain>) -> ReplayGain {
        let Some(o) = other else {
            return self;
        };
        ReplayGain {
            track_gain: self.track_gain.or(o.track_gain),
            track_peak: self.track_peak.or(o.track_peak),
            album_gain: self.album_gain.or(o.album_gain),
            album_peak: self.album_peak.or(o.album_peak),
        }
    }

    /// Read ReplayGain from file tags (`REPLAYGAIN_*` in Vorbis comments, APE and ID3 `TXXX`,
    /// `R128_*` in Opus). Keys are matched case-insensitively.
    pub fn from_tags<'a>(tags: impl IntoIterator<Item = (&'a str, &'a str)>) -> Option<Self> {
        let mut rg = ReplayGain::default();
        let mut r128 = (None, None);
        for (key, value) in tags {
            let key = key.trim().to_ascii_lowercase();
            let key = key.strip_prefix("txxx:").unwrap_or(&key);
            match key {
                "replaygain_track_gain" => rg.track_gain = parse_db(value),
                "replaygain_album_gain" => rg.album_gain = parse_db(value),
                "replaygain_track_peak" => rg.track_peak = parse_peak(value),
                "replaygain_album_peak" => rg.album_peak = parse_peak(value),
                "r128_track_gain" => r128.0 = parse_q78(value),
                "r128_album_gain" => r128.1 = parse_q78(value),
                _ => {}
            }
        }
        // R128 gains are relative to -23 LUFS, ReplayGain to about -18 LUFS.
        rg.track_gain = rg.track_gain.or(r128.0.map(|g| g + 5.0));
        rg.album_gain = rg.album_gain.or(r128.1.map(|g| g + 5.0));
        (!rg.is_empty()).then_some(rg)
    }
}

fn parse_db(s: &str) -> Option<f32> {
    let s = s.trim();
    let s = s
        .strip_suffix("dB")
        .or_else(|| s.strip_suffix("db"))
        .or_else(|| s.strip_suffix("DB"))
        .unwrap_or(s);
    let v: f32 = s.trim().parse().ok()?;
    (v.is_finite() && v.abs() < 60.0).then_some(v)
}

fn parse_peak(s: &str) -> Option<f32> {
    let v: f32 = s.trim().parse().ok()?;
    (v.is_finite() && v > 0.0 && v < 100.0).then_some(v)
}

/// Opus R128 gains: signed Q7.8 fixed point in dB.
fn parse_q78(s: &str) -> Option<f32> {
    let v: i32 = s.trim().parse().ok()?;
    i16::try_from(v).ok().map(|v| f32::from(v) / 256.0)
}

/// Which gain to use.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ReplayGainMode {
    /// No normalisation.
    #[default]
    Off,
    Track,
    Album,
    /// Album gain while songs of one album play in a row, track gain otherwise.
    Auto,
}

/// ReplayGain settings.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ReplayGainConfig {
    pub mode: ReplayGainMode,
    /// Added to every gain, in dB.
    pub preamp_db: f32,
    /// Gain for tracks without ReplayGain values, in dB.
    pub fallback_db: f32,
    /// Lower the gain so that the peak does not clip.
    pub prevent_clipping: bool,
}

impl Default for ReplayGainConfig {
    fn default() -> Self {
        Self {
            mode: ReplayGainMode::Off,
            preamp_db: 0.0,
            fallback_db: 0.0,
            prevent_clipping: true,
        }
    }
}

impl ReplayGainConfig {
    /// Gain in dB for a track. `album` says whether album gain applies (in `Auto` mode).
    /// `None` when normalisation is off.
    pub fn gain_db(&self, rg: Option<&ReplayGain>, album_context: bool) -> Option<f32> {
        let use_album = match self.mode {
            ReplayGainMode::Off => return None,
            ReplayGainMode::Track => false,
            ReplayGainMode::Album => true,
            ReplayGainMode::Auto => album_context,
        };
        let (gain, peak) = match rg {
            Some(r) if use_album && r.album_gain.is_some() => (r.album_gain, r.album_peak),
            Some(r) if r.track_gain.is_some() => (r.track_gain, r.track_peak),
            Some(r) => (r.album_gain, r.album_peak),
            None => (None, None),
        };
        let mut db = gain.unwrap_or(self.fallback_db) + self.preamp_db;
        if self.prevent_clipping
            && let Some(p) = peak.filter(|p| *p > 0.0)
        {
            // Highest gain that keeps the peak at or below full scale.
            db = db.min(-20.0 * p.log10());
        }
        Some(db.clamp(-60.0, 24.0))
    }
}

/// dB to a linear factor.
pub fn db_to_linear(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tags() {
        let rg = ReplayGain::from_tags([
            ("REPLAYGAIN_TRACK_GAIN", "-6.54 dB"),
            ("replaygain_track_peak", "0.988"),
            ("TXXX:REPLAYGAIN_ALBUM_GAIN", "+1.2 dB"),
            ("comment", "x"),
        ])
        .expect("gain");
        assert_eq!(rg.track_gain, Some(-6.54));
        assert_eq!(rg.track_peak, Some(0.988));
        assert_eq!(rg.album_gain, Some(1.2));
        assert_eq!(rg.album_peak, None);
        assert_eq!(ReplayGain::from_tags([("title", "x")]), None);
        assert_eq!(
            ReplayGain::from_tags([("replaygain_track_gain", "loud")]),
            None
        );
    }

    #[test]
    fn parses_opus_r128() {
        // -1280 / 256 = -5 dB relative to -23 LUFS = 0 dB ReplayGain.
        let rg = ReplayGain::from_tags([("R128_TRACK_GAIN", "-1280")]).expect("gain");
        assert_eq!(rg.track_gain, Some(0.0));
    }

    #[test]
    fn modes_pick_the_right_gain() {
        let rg = ReplayGain {
            track_gain: Some(-8.0),
            track_peak: Some(0.5),
            album_gain: Some(-6.0),
            album_peak: Some(0.9),
        };
        let mut c = ReplayGainConfig {
            mode: ReplayGainMode::Track,
            ..ReplayGainConfig::default()
        };
        assert_eq!(c.gain_db(Some(&rg), true), Some(-8.0));
        c.mode = ReplayGainMode::Album;
        assert_eq!(c.gain_db(Some(&rg), false), Some(-6.0));
        c.mode = ReplayGainMode::Auto;
        assert_eq!(c.gain_db(Some(&rg), true), Some(-6.0));
        assert_eq!(c.gain_db(Some(&rg), false), Some(-8.0));
        c.mode = ReplayGainMode::Off;
        assert_eq!(c.gain_db(Some(&rg), true), None);
    }

    #[test]
    fn fallback_preamp_and_clipping() {
        let c = ReplayGainConfig {
            mode: ReplayGainMode::Track,
            preamp_db: 6.0,
            fallback_db: -3.0,
            prevent_clipping: true,
        };
        assert_eq!(c.gain_db(None, false), Some(3.0));
        // +6 dB would clip a peak of 0.8: limited to 20*log10(1/0.8) = 1.94 dB.
        let rg = ReplayGain {
            track_gain: Some(0.0),
            track_peak: Some(0.8),
            ..ReplayGain::default()
        };
        let g = c.gain_db(Some(&rg), false).expect("gain");
        assert!((g - 1.938).abs() < 0.01, "{g}");
        assert!((db_to_linear(-6.0) - 0.501).abs() < 0.001);
    }
}
