// SPDX-License-Identifier: GPL-3.0-only
//! Application logic shared by the infrasonic frontends: configuration and server profiles,
//! streaming from Subsonic into the engine, scrobbling and play queue synchronisation.

pub mod cache;
pub mod config;
pub mod http;
pub mod scrobble;
pub mod session;

pub use infrasonic_engine as engine;
pub use infrasonic_subsonic as subsonic;

pub use config::{Config, ConfigError, Paths, Profile};
pub use session::{Session, SessionEvent};

use infrasonic_engine::Track;
use infrasonic_subsonic::Child;

/// Errors from core operations.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Subsonic(#[from] infrasonic_subsonic::Error),
}

/// Convert a Subsonic song into an engine track.
pub fn track_from_child(c: &Child) -> Track {
    Track {
        id: c.id.clone(),
        title: c.title.clone(),
        artist: c.artist_name().to_string(),
        album: c.album.clone().unwrap_or_default(),
        duration: c.duration.map(f64::from),
        suffix: c.transcoded_suffix.clone().or_else(|| c.suffix.clone()),
        content_type: c
            .transcoded_content_type
            .clone()
            .or_else(|| c.content_type.clone()),
        cover_art: c.cover_art.clone(),
        track_number: c.track,
        disc_number: c.disc_number,
        year: c.year,
        genre: c.genre.clone(),
        bit_rate: c.bit_rate,
        album_id: c.album_id.clone(),
        artist_id: c.artist_id.clone(),
        starred: c.is_starred(),
        replay_gain: c.replay_gain.as_ref().and_then(|r| {
            let rg = infrasonic_engine::ReplayGain {
                track_gain: r.track_gain.or(r.fallback_gain),
                track_peak: r.track_peak.filter(|p| *p > 0.0),
                album_gain: r.album_gain.or(r.fallback_gain),
                album_peak: r.album_peak.filter(|p| *p > 0.0),
            };
            (!rg.is_empty()).then_some(rg)
        }),
    }
}
