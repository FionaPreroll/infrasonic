// SPDX-License-Identifier: GPL-3.0-only
//! Playback engine for infrasonic.
//!
//! The engine owns the play queue and turns tracks into sound:
//!
//! 1. a [`SourceProvider`] supplied by the application opens a byte stream for a track,
//! 2. [`decode::TrackDecoder`] demuxes and decodes it (Symphonia, plus a pure Rust Opus decoder),
//! 3. [`convert`] maps channels and resamples to the device format,
//! 4. an [`output::OutputFactory`] plays the samples (cpal, or a null output for tests).
//!
//! The next track is opened while the current one plays and written into the same output
//! buffer, so transitions are gapless as far as the codecs allow.
//!
//! The engine knows nothing about Subsonic, credentials or async runtimes. Frontends talk to it
//! through [`Command`]s and observe it through [`Event`]s and [`Snapshot`]s.

pub mod convert;
pub mod decode;
pub mod gain;
mod mp4edit;
pub mod output;
mod player;
pub mod queue;
pub mod source;

pub use gain::{ReplayGain, ReplayGainConfig, ReplayGainMode};
pub use player::{Command, Engine, EngineConfig, Event, PlayState, Pump, Snapshot, SnapshotReader};
pub use queue::{Queue, QueueEntry, RepeatMode};
pub use symphonia::core::io::MediaSource;

/// Errors produced by the engine.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("could not open stream: {0}")]
    Source(String),
    #[error("could not decode audio: {0}")]
    Decode(String),
    #[error("audio output failed: {0}")]
    Output(String),
}

/// Metadata of a playable track. The engine only needs `id` and a format hint; the rest is
/// carried along for frontends.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Track {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    /// Duration in seconds.
    pub duration: Option<f64>,
    /// File extension as reported by the server (`mp3`, `flac`, ...).
    pub suffix: Option<String>,
    pub content_type: Option<String>,
    pub cover_art: Option<String>,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
    pub year: Option<i32>,
    pub genre: Option<String>,
    /// Kbit/s.
    pub bit_rate: Option<u32>,
    pub album_id: Option<String>,
    pub artist_id: Option<String>,
    pub starred: bool,
    /// The user's rating, 1–5.
    pub rating: Option<u8>,
    /// ReplayGain values from the server. Tags in the file fill in what is missing.
    pub replay_gain: Option<ReplayGain>,
}

impl Track {
    /// A track with just an id and a title.
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            ..Self::default()
        }
    }

    /// Hint for the demuxer: extension or MIME type.
    pub fn format_hint(&self) -> Option<&str> {
        self.suffix.as_deref().or(self.content_type.as_deref())
    }
}

/// Opens byte streams for tracks. Implemented by the application (HTTP, cache, files).
///
/// `open` should return quickly; the returned source may block in `read` until data arrives
/// (see [`source::stream_pair`]). It is called from engine worker threads.
pub trait SourceProvider: Send + Sync {
    fn open(&self, track: &Track) -> Result<Box<dyn MediaSource>, Error>;

    /// Whether `track` can be opened and read without blocking. Only asked in pump mode
    /// ([`Engine::new_pump`]), where a blocking read would stall everything; the engine waits
    /// until this returns true. Native threads may block, so the default is true.
    fn is_ready(&self, _track: &Track) -> bool {
        true
    }
}

impl<F> SourceProvider for F
where
    F: Fn(&Track) -> Result<Box<dyn MediaSource>, Error> + Send + Sync,
{
    fn open(&self, track: &Track) -> Result<Box<dyn MediaSource>, Error> {
        self(track)
    }
}

/// Map a 0–100 volume to a linear gain (cubic curve, like most players).
pub fn volume_to_gain(percent: u8) -> f32 {
    let v = f32::from(percent.min(100)) / 100.0;
    v * v * v
}
