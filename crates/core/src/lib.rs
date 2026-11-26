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

/// The Subsonic view of an engine track (for APIs that take songs, e.g. pinning).
pub fn child_from_track(t: &Track) -> Child {
    Child {
        id: t.id.clone(),
        title: t.title.clone(),
        artist: (!t.artist.is_empty()).then(|| t.artist.clone()),
        album: (!t.album.is_empty()).then(|| t.album.clone()),
        duration: t.duration.map(|d| d.round() as u32),
        suffix: t.suffix.clone(),
        content_type: t.content_type.clone(),
        cover_art: t.cover_art.clone(),
        track: t.track_number,
        disc_number: t.disc_number,
        year: t.year,
        genre: t.genre.clone(),
        bit_rate: t.bit_rate,
        album_id: t.album_id.clone(),
        artist_id: t.artist_id.clone(),
        starred: t.starred.then(|| "1".to_string()),
        user_rating: t.rating,
        ..Child::default()
    }
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
        rating: c.user_rating.filter(|r| *r > 0),
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
