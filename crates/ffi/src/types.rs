// SPDX-License-Identifier: GPL-3.0-only
//! Records and enums that cross the language boundary, and their conversions.

use infrasonic_core::CoreError;
use infrasonic_engine as engine;
use infrasonic_subsonic as ss;
use serde::{Deserialize, Serialize};

/// Errors reported to Kotlin.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum FfiError {
    /// Wrong username, password or API key.
    #[error("{reason}")]
    Auth { reason: String },
    /// The server refused the request (Subsonic error code).
    #[error("{reason}")]
    Server { code: i32, reason: String },
    /// No connection, TLS failure, timeout or an HTTP error status.
    #[error("{reason}")]
    Network { reason: String },
    /// The server answered with something that is not a Subsonic response.
    #[error("{reason}")]
    Protocol { reason: String },
    /// The server does not offer this feature.
    #[error("{reason}")]
    NotSupported { reason: String },
    /// A call that needs a server while none is connected.
    #[error("not connected to a server")]
    NotConnected,
    #[error("{reason}")]
    Other { reason: String },
}

impl From<ss::Error> for FfiError {
    fn from(e: ss::Error) -> Self {
        let reason = error_chain(&e);
        if e.is_auth_error() {
            return FfiError::Auth { reason };
        }
        if e.is_not_supported() {
            return FfiError::NotSupported { reason };
        }
        match e {
            ss::Error::Api { code, .. } => FfiError::Server { code, reason },
            ss::Error::Http(_) => FfiError::Network { reason },
            ss::Error::Url(_) => FfiError::Other { reason },
            ss::Error::Decode { .. } | ss::Error::MissingPayload { .. } => {
                FfiError::Protocol { reason }
            }
        }
    }
}

impl From<CoreError> for FfiError {
    fn from(e: CoreError) -> Self {
        match e {
            CoreError::Subsonic(e) => e.into(),
            CoreError::Config(e) => FfiError::Other {
                reason: e.to_string(),
            },
        }
    }
}

/// The message of an error and of its causes, so that e.g. TLS problems are visible.
fn error_chain(e: &dyn std::error::Error) -> String {
    let mut s = e.to_string();
    let mut cur = e.source();
    while let Some(c) = cur {
        let m = c.to_string();
        if !s.contains(&m) {
            s.push_str(": ");
            s.push_str(&m);
        }
        cur = c.source();
    }
    s
}

pub type Result<T, E = FfiError> = std::result::Result<T, E>;

/// How to log in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ServerAuth {
    /// Salted token (`t`/`s`); the password itself is not stored. See [`token_auth`].
    Token {
        username: String,
        token: String,
        salt: String,
    },
    /// Legacy login with the password in every request, for servers that cannot check tokens
    /// (e.g. LDAP setups).
    Plain { username: String, password: String },
    /// OpenSubsonic API key.
    ApiKey { key: String },
}

impl ServerAuth {
    pub(crate) fn to_auth(&self) -> ss::Auth {
        match self.clone() {
            ServerAuth::Token {
                username,
                token,
                salt,
            } => ss::Auth::SaltedToken {
                username,
                token,
                salt,
            },
            ServerAuth::Plain { username, password } => ss::Auth::Plain { username, password },
            ServerAuth::ApiKey { key } => ss::Auth::ApiKey { key },
        }
    }

    pub(crate) fn username(&self) -> &str {
        match self {
            ServerAuth::Token { username, .. } | ServerAuth::Plain { username, .. } => username,
            ServerAuth::ApiKey { .. } => "",
        }
    }
}

/// Token login data for a password: `md5(password + salt)` with a fresh salt.
#[uniffi::export]
pub fn token_auth(username: String, password: String) -> ServerAuth {
    match ss::Auth::salted(&username, &password) {
        ss::Auth::SaltedToken {
            username,
            token,
            salt,
        } => ServerAuth::Token {
            username,
            token,
            salt,
        },
        _ => unreachable!("Auth::salted returns a salted token"),
    }
}

/// A configured server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct ServerProfile {
    /// Stable id chosen by the app.
    pub id: String,
    pub name: String,
    pub url: String,
    pub auth: ServerAuth,
    /// Accept any TLS certificate (self-signed servers in a home network).
    pub allow_self_signed: bool,
}

impl ServerProfile {
    pub(crate) fn client(&self) -> Result<ss::Client> {
        Ok(ss::Client::builder(&self.url, self.auth.to_auth())
            .client_name("infrasonic-android")
            .accept_invalid_certs(self.allow_self_signed)
            .build()?)
    }

    /// Directory name for this server's cache and pinned songs.
    pub(crate) fn key(&self) -> String {
        infrasonic_core::cache::server_key(&self.url, self.auth.username())
    }
}

/// What the server said about itself.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ServerStatus {
    /// Subsonic API version.
    pub version: String,
    /// `navidrome`, `gonic`, ... (OpenSubsonic servers only).
    pub server_type: Option<String>,
    pub server_version: Option<String>,
    pub open_subsonic: bool,
    /// Whether the session talks to the server (false in offline mode).
    pub online: bool,
}

impl ServerStatus {
    pub(crate) fn from_info(i: &ss::ServerInfo) -> Self {
        Self {
            version: i.version.clone(),
            server_type: i.server_type.clone(),
            server_version: i.server_version.clone(),
            open_subsonic: i.open_subsonic,
            online: !i.version.is_empty(),
        }
    }
}

/// A song (or podcast episode).
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Song {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_id: Option<String>,
    pub artist_id: Option<String>,
    pub parent: Option<String>,
    /// Seconds.
    pub duration: Option<u32>,
    pub track: Option<u32>,
    pub disc: Option<u32>,
    pub year: Option<i32>,
    pub genre: Option<String>,
    pub cover_art: Option<String>,
    pub suffix: Option<String>,
    pub content_type: Option<String>,
    pub transcoded_suffix: Option<String>,
    pub transcoded_content_type: Option<String>,
    /// Kbit/s.
    pub bit_rate: Option<u32>,
    /// Bytes.
    pub size: Option<u64>,
    pub starred: bool,
    /// 1–5, `None` when unrated.
    pub rating: Option<u8>,
    pub play_count: Option<u64>,
    /// `music`, `podcast`, ...
    pub media_type: Option<String>,
    pub track_gain: Option<f32>,
    pub album_gain: Option<f32>,
    pub track_peak: Option<f32>,
    pub album_peak: Option<f32>,
}

impl From<&ss::Child> for Song {
    fn from(c: &ss::Child) -> Self {
        let rg = c.replay_gain.unwrap_or_default();
        Self {
            id: c.id.clone(),
            title: c.title.clone(),
            artist: c.artist_name().to_string(),
            album: c.album.clone().unwrap_or_default(),
            album_id: c.album_id.clone(),
            artist_id: c.artist_id.clone(),
            parent: c.parent.clone(),
            duration: c.duration,
            track: c.track,
            disc: c.disc_number,
            year: c.year,
            genre: c.genre.clone(),
            cover_art: c.cover_art.clone(),
            suffix: c.suffix.clone(),
            content_type: c.content_type.clone(),
            transcoded_suffix: c.transcoded_suffix.clone(),
            transcoded_content_type: c.transcoded_content_type.clone(),
            bit_rate: c.bit_rate,
            size: c.size,
            starred: c.is_starred(),
            rating: c.user_rating.filter(|r| *r > 0),
            play_count: c.play_count,
            media_type: c.media_type.clone(),
            track_gain: rg.track_gain.or(rg.fallback_gain),
            album_gain: rg.album_gain.or(rg.fallback_gain),
            track_peak: rg.track_peak,
            album_peak: rg.album_peak,
        }
    }
}

impl From<&Song> for ss::Child {
    fn from(s: &Song) -> Self {
        let rg = ss::ReplayGain {
            track_gain: s.track_gain,
            album_gain: s.album_gain,
            track_peak: s.track_peak,
            album_peak: s.album_peak,
            ..ss::ReplayGain::default()
        };
        let has_rg = rg.track_gain.is_some() || rg.album_gain.is_some();
        ss::Child {
            id: s.id.clone(),
            parent: s.parent.clone(),
            is_dir: false,
            title: s.title.clone(),
            album: (!s.album.is_empty()).then(|| s.album.clone()),
            artist: (!s.artist.is_empty()).then(|| s.artist.clone()),
            track: s.track,
            year: s.year,
            genre: s.genre.clone(),
            cover_art: s.cover_art.clone(),
            size: s.size,
            content_type: s.content_type.clone(),
            suffix: s.suffix.clone(),
            transcoded_content_type: s.transcoded_content_type.clone(),
            transcoded_suffix: s.transcoded_suffix.clone(),
            duration: s.duration,
            bit_rate: s.bit_rate,
            play_count: s.play_count,
            disc_number: s.disc,
            starred: s.starred.then(|| "1".to_string()),
            album_id: s.album_id.clone(),
            artist_id: s.artist_id.clone(),
            media_type: s.media_type.clone(),
            user_rating: s.rating,
            replay_gain: has_rg.then_some(rg),
            ..ss::Child::default()
        }
    }
}

impl From<&engine::Track> for Song {
    fn from(t: &engine::Track) -> Self {
        let rg = t.replay_gain.unwrap_or_default();
        Self {
            id: t.id.clone(),
            title: t.title.clone(),
            artist: t.artist.clone(),
            album: t.album.clone(),
            album_id: t.album_id.clone(),
            artist_id: t.artist_id.clone(),
            parent: None,
            duration: t.duration.map(|d| d.round() as u32),
            track: t.track_number,
            disc: t.disc_number,
            year: t.year,
            genre: t.genre.clone(),
            cover_art: t.cover_art.clone(),
            suffix: t.suffix.clone(),
            content_type: t.content_type.clone(),
            transcoded_suffix: None,
            transcoded_content_type: None,
            bit_rate: t.bit_rate,
            size: None,
            starred: t.starred,
            rating: t.rating,
            play_count: None,
            media_type: None,
            track_gain: rg.track_gain,
            album_gain: rg.album_gain,
            track_peak: rg.track_peak,
            album_peak: rg.album_peak,
        }
    }
}

pub(crate) fn songs(children: &[ss::Child]) -> Vec<Song> {
    children
        .iter()
        .filter(|c| !c.is_dir)
        .map(Song::from)
        .collect()
}

pub(crate) fn children(songs: &[Song]) -> Vec<ss::Child> {
    songs.iter().map(ss::Child::from).collect()
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Album {
    pub id: String,
    pub name: String,
    pub artist: Option<String>,
    pub artist_id: Option<String>,
    pub cover_art: Option<String>,
    pub song_count: Option<u32>,
    pub duration: Option<u32>,
    pub year: Option<i32>,
    pub genre: Option<String>,
    pub starred: bool,
    pub rating: Option<u8>,
    pub play_count: Option<u64>,
    pub created: Option<String>,
    /// Filled by `album`, empty in lists.
    pub songs: Vec<Song>,
}

impl From<&ss::Album> for Album {
    fn from(a: &ss::Album) -> Self {
        Self {
            id: a.id.clone(),
            name: a.display_name().to_string(),
            artist: a.artist.clone(),
            artist_id: a.artist_id.clone(),
            cover_art: a.cover_art.clone(),
            song_count: a.song_count,
            duration: a.duration,
            year: a.year,
            genre: a.genre.clone(),
            starred: a.starred.is_some(),
            rating: a.user_rating.filter(|r| *r > 0),
            play_count: a.play_count,
            created: a.created.clone(),
            songs: songs(&a.song),
        }
    }
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Artist {
    pub id: String,
    pub name: String,
    pub cover_art: Option<String>,
    pub album_count: Option<u32>,
    pub starred: bool,
    /// Filled by `artist`, empty in lists.
    pub albums: Vec<Album>,
}

impl From<&ss::Artist> for Artist {
    fn from(a: &ss::Artist) -> Self {
        Self {
            id: a.id.clone(),
            name: a.name.clone(),
            cover_art: a.cover_art.clone(),
            album_count: a.album_count,
            starred: a.starred.is_some(),
            albums: a.album.iter().map(Album::from).collect(),
        }
    }
}

/// Artists under one index letter.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ArtistGroup {
    pub name: String,
    pub artists: Vec<Artist>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ArtistInfo {
    pub biography: Option<String>,
    pub image_url: Option<String>,
    pub last_fm_url: Option<String>,
    pub similar: Vec<Artist>,
}

/// A folder in file view.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Folder {
    pub id: String,
    pub name: String,
    pub parent: Option<String>,
    pub folders: Vec<FolderEntry>,
    pub songs: Vec<Song>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct FolderEntry {
    pub id: String,
    pub name: String,
    pub cover_art: Option<String>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Playlist {
    pub id: String,
    pub name: String,
    pub comment: Option<String>,
    pub owner: Option<String>,
    pub public: bool,
    pub song_count: Option<u32>,
    pub duration: Option<u32>,
    pub cover_art: Option<String>,
    pub changed: Option<String>,
    /// Filled by `playlist`, empty in lists.
    pub songs: Vec<Song>,
}

impl From<&ss::Playlist> for Playlist {
    fn from(p: &ss::Playlist) -> Self {
        Self {
            id: p.id.clone(),
            name: p.name.clone(),
            comment: p.comment.clone(),
            owner: p.owner.clone(),
            public: p.public,
            song_count: p.song_count,
            duration: p.duration,
            cover_art: p.cover_art.clone(),
            changed: p.changed.clone(),
            songs: songs(&p.entry),
        }
    }
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct SearchResult {
    pub artists: Vec<Artist>,
    pub albums: Vec<Album>,
    pub songs: Vec<Song>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Genre {
    pub name: String,
    pub song_count: Option<u32>,
    pub album_count: Option<u32>,
}

/// Album lists (`getAlbumList2`).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum AlbumListKind {
    Random,
    Newest,
    Highest,
    Frequent,
    Recent,
    ByName,
    ByArtist,
    Starred,
    ByYear { from: i32, to: i32 },
    ByGenre { genre: String },
}

/// One line of lyrics; `start_ms` is set for synchronised lyrics.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct LyricsLine {
    pub start_ms: Option<i64>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Lyrics {
    pub synced: bool,
    pub lang: Option<String>,
    pub lines: Vec<LyricsLine>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Bookmark {
    pub song: Song,
    pub position_ms: u64,
    pub comment: Option<String>,
    pub changed: Option<String>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Share {
    pub id: String,
    pub url: String,
    pub description: Option<String>,
    pub expires: Option<String>,
    pub visit_count: u64,
    pub songs: Vec<Song>,
}

impl From<&ss::Share> for Share {
    fn from(s: &ss::Share) -> Self {
        Self {
            id: s.id.clone(),
            url: s.url.clone(),
            description: s.description.clone(),
            expires: s.expires.clone(),
            visit_count: s.visit_count,
            songs: songs(&s.entry),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RadioStation {
    pub id: String,
    pub name: String,
    pub stream_url: String,
    pub home_page_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PodcastEpisode {
    pub id: String,
    pub channel_id: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub publish_date: Option<String>,
    /// `new`, `downloading`, `completed`, `error`, `deleted`, `skipped`.
    pub status: String,
    pub duration: Option<u32>,
    pub cover_art: Option<String>,
    /// Playable once the server has downloaded the episode.
    pub song: Option<Song>,
}

impl From<&ss::PodcastEpisode> for PodcastEpisode {
    fn from(e: &ss::PodcastEpisode) -> Self {
        Self {
            id: e.id.clone(),
            channel_id: e.channel_id.clone(),
            title: e.title.clone(),
            description: e.description.clone(),
            publish_date: e.publish_date.clone(),
            status: e.status.clone().unwrap_or_default(),
            duration: e.duration,
            cover_art: e.cover_art.clone(),
            song: e.as_song().as_ref().map(Song::from),
        }
    }
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PodcastChannel {
    pub id: String,
    pub url: String,
    pub title: String,
    pub description: Option<String>,
    pub cover_art: Option<String>,
    pub image_url: Option<String>,
    pub status: String,
    pub error: Option<String>,
    pub episodes: Vec<PodcastEpisode>,
}

impl From<&ss::PodcastChannel> for PodcastChannel {
    fn from(c: &ss::PodcastChannel) -> Self {
        Self {
            id: c.id.clone(),
            url: c.url.clone(),
            title: c.title.clone().unwrap_or_else(|| c.url.clone()),
            description: c.description.clone(),
            cover_art: c.cover_art.clone(),
            image_url: c.original_image_url.clone(),
            status: c.status.clone().unwrap_or_default(),
            error: c.error_message.clone(),
            episodes: c.episode.iter().map(PodcastEpisode::from).collect(),
        }
    }
}

/// Server-side playback (`jukeboxControl`).
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum JukeboxCommand {
    Get,
    Set {
        ids: Vec<String>,
    },
    Add {
        ids: Vec<String>,
    },
    Start,
    Stop,
    Skip {
        index: u32,
        offset_secs: u32,
    },
    Clear,
    Remove {
        index: u32,
    },
    Shuffle,
    /// 0.0–1.0.
    SetGain {
        gain: f32,
    },
}

impl From<JukeboxCommand> for ss::JukeboxAction {
    fn from(c: JukeboxCommand) -> Self {
        match c {
            JukeboxCommand::Get => Self::Get,
            JukeboxCommand::Set { ids } => Self::Set(ids),
            JukeboxCommand::Add { ids } => Self::Add(ids),
            JukeboxCommand::Start => Self::Start,
            JukeboxCommand::Stop => Self::Stop,
            JukeboxCommand::Skip { index, offset_secs } => Self::Skip {
                index,
                offset: offset_secs,
            },
            JukeboxCommand::Clear => Self::Clear,
            JukeboxCommand::Remove { index } => Self::Remove(index),
            JukeboxCommand::Shuffle => Self::Shuffle,
            JukeboxCommand::SetGain { gain } => Self::SetGain(gain),
        }
    }
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct JukeboxState {
    pub playing: bool,
    pub current_index: Option<u32>,
    pub position_secs: u32,
    pub gain: f32,
    /// Filled by [`JukeboxCommand::Get`].
    pub songs: Vec<Song>,
}

impl From<&ss::JukeboxStatus> for JukeboxState {
    fn from(s: &ss::JukeboxStatus) -> Self {
        Self {
            playing: s.playing,
            current_index: s.current_index.and_then(|i| u32::try_from(i).ok()),
            position_secs: s.position.unwrap_or(0),
            gain: s.gain.unwrap_or(1.0),
            songs: songs(&s.entry),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PlayState {
    Stopped,
    Buffering,
    Playing,
    Paused,
}

impl From<engine::PlayState> for PlayState {
    fn from(s: engine::PlayState) -> Self {
        match s {
            engine::PlayState::Stopped => Self::Stopped,
            engine::PlayState::Buffering => Self::Buffering,
            engine::PlayState::Playing => Self::Playing,
            engine::PlayState::Paused => Self::Paused,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum Repeat {
    Off,
    All,
    One,
}

impl From<engine::RepeatMode> for Repeat {
    fn from(r: engine::RepeatMode) -> Self {
        match r {
            engine::RepeatMode::Off => Self::Off,
            engine::RepeatMode::All => Self::All,
            engine::RepeatMode::One => Self::One,
        }
    }
}

impl From<Repeat> for engine::RepeatMode {
    fn from(r: Repeat) -> Self {
        match r {
            Repeat::Off => Self::Off,
            Repeat::All => Self::All,
            Repeat::One => Self::One,
        }
    }
}

/// A queue entry; `uid` stays the same while the entry is in the queue.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct QueueItem {
    pub uid: u64,
    pub song: Song,
}

impl From<&engine::QueueEntry> for QueueItem {
    fn from(e: &engine::QueueEntry) -> Self {
        Self {
            uid: e.uid,
            song: Song::from(&e.track),
        }
    }
}

/// Playback state at one moment.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PlayerState {
    pub state: PlayState,
    pub current: Option<QueueItem>,
    pub index: Option<u32>,
    /// Seconds.
    pub position: f64,
    pub duration: Option<f64>,
    pub volume: u8,
    pub repeat: Repeat,
    pub queue_length: u32,
    pub replay_gain_db: Option<f32>,
    pub underruns: u64,
}

impl From<&engine::Snapshot> for PlayerState {
    fn from(s: &engine::Snapshot) -> Self {
        Self {
            state: s.state.into(),
            current: s.current.as_ref().map(QueueItem::from),
            index: s.current_index.map(|i| i as u32),
            position: s.position,
            duration: s.duration,
            volume: s.volume,
            repeat: s.repeat.into(),
            queue_length: s.queue.len() as u32,
            replay_gain_db: s.replay_gain_db,
            underruns: s.underruns,
        }
    }
}

/// Notifications for the app; delivered on a background thread.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum PlayerEvent {
    StateChanged {
        state: PlayState,
    },
    /// `None` when playback stopped.
    TrackChanged {
        item: Option<QueueItem>,
    },
    QueueChanged,
    /// About twice per second while playing, and after seeks.
    Position {
        position: f64,
        duration: Option<f64>,
    },
    VolumeChanged {
        volume: u8,
    },
    RepeatChanged {
        repeat: Repeat,
    },
    QueueEnded,
    /// A play was submitted to the server.
    Scrobbled {
        id: String,
    },
    /// A song was downloaded for offline use.
    Pinned {
        id: String,
    },
    /// A problem that did not stop the session, e.g. a song that could not be played.
    Error {
        message: String,
    },
}

impl From<infrasonic_core::SessionEvent> for PlayerEvent {
    fn from(e: infrasonic_core::SessionEvent) -> Self {
        use infrasonic_core::SessionEvent as S;
        match e {
            S::Engine(ev) => match ev {
                engine::Event::StateChanged(s) => Self::StateChanged { state: s.into() },
                engine::Event::TrackChanged(e) => Self::TrackChanged {
                    item: e.as_ref().map(QueueItem::from),
                },
                engine::Event::QueueChanged => Self::QueueChanged,
                engine::Event::Position { position, duration } => {
                    Self::Position { position, duration }
                }
                engine::Event::VolumeChanged(v) => Self::VolumeChanged { volume: v },
                engine::Event::RepeatChanged(r) => Self::RepeatChanged { repeat: r.into() },
                engine::Event::QueueEnded => Self::QueueEnded,
                engine::Event::Error(message) => Self::Error { message },
            },
            S::Scrobbled { id } => Self::Scrobbled { id },
            S::Pinned { id } => Self::Pinned { id },
            S::Error(message) => Self::Error { message },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "snake_case")]
pub enum ReplayGainMode {
    Off,
    Track,
    Album,
    /// Album gain while an album plays in order, track gain otherwise.
    Auto,
}

impl From<ReplayGainMode> for infrasonic_core::config::ReplayGainSetting {
    fn from(m: ReplayGainMode) -> Self {
        match m {
            ReplayGainMode::Off => Self::Off,
            ReplayGainMode::Track => Self::Track,
            ReplayGainMode::Album => Self::Album,
            ReplayGainMode::Auto => Self::Auto,
        }
    }
}

/// Streaming quality: `max_bit_rate` in kbit/s, 0 for the original file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct Quality {
    pub max_bit_rate: u32,
    /// Transcoding format such as `opus` or `mp3`; `None` lets the server choose.
    pub format: Option<String>,
}

/// Settings that apply to the next session; most can also be changed while connected.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, uniffi::Record)]
#[serde(default)]
pub struct PlaybackSettings {
    pub volume: u8,
    pub scrobble: bool,
    pub quality: Quality,
    /// Quality of pinned songs; `None` pins like streaming.
    pub pin_quality: Option<Quality>,
    pub replay_gain: ReplayGainMode,
    pub replay_gain_preamp: f32,
    pub replay_gain_fallback: f32,
    pub prevent_clipping: bool,
    /// Size of the song cache in MiB; 0 turns caching off.
    pub cache_mb: u64,
    /// Songs after the next one that are downloaded in advance.
    pub preload_songs: u32,
}

impl Default for PlaybackSettings {
    fn default() -> Self {
        Self {
            volume: 100,
            scrobble: true,
            quality: Quality {
                max_bit_rate: 0,
                format: None,
            },
            pin_quality: None,
            replay_gain: ReplayGainMode::Auto,
            replay_gain_preamp: 0.0,
            replay_gain_fallback: 0.0,
            prevent_clipping: true,
            cache_mb: 2048,
            preload_songs: 2,
        }
    }
}

/// Default settings, for the app's first start.
#[uniffi::export]
pub fn default_playback_settings() -> PlaybackSettings {
    PlaybackSettings::default()
}

impl PlaybackSettings {
    pub(crate) fn to_config(&self) -> infrasonic_core::config::PlaybackConfig {
        infrasonic_core::config::PlaybackConfig {
            volume: self.volume.min(100),
            scrobble: self.scrobble,
            max_bit_rate: self.quality.max_bit_rate,
            format: self.quality.format.clone(),
            // The app saves the queue itself when it goes to the background.
            save_queue_on_exit: false,
            replay_gain: self.replay_gain.into(),
            replay_gain_preamp: self.replay_gain_preamp,
            replay_gain_fallback: self.replay_gain_fallback,
            prevent_clipping: self.prevent_clipping,
            cache_mb: self.cache_mb,
            preload_songs: self.preload_songs,
            ..infrasonic_core::config::PlaybackConfig::default()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct CacheUsage {
    pub cached_files: u64,
    pub cached_bytes: u64,
    pub pinned_files: u64,
    pub pinned_bytes: u64,
}

/// What to star or rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ItemKind {
    Song,
    Album,
    Artist,
}

/// A queue saved on the device, to resume playback (e.g. from a Bluetooth headset).
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct SavedQueue {
    pub songs: Vec<Song>,
    pub index: u32,
    /// Seconds.
    pub position: f64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct StoredQueue {
    pub songs: Vec<ss::Child>,
    pub index: u32,
    pub position: f64,
}

/// `profiles.json`: the configured servers and the one in use.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct StoredProfiles {
    pub active: Option<String>,
    pub list: Vec<ServerProfile>,
}

/// Equalizer settings: a preamp and one gain per band of [`equalizer_bands`], in dB.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct EqualizerSettings {
    pub preamp_db: f32,
    /// Missing bands count as 0 dB, extra ones are ignored.
    pub gains_db: Vec<f32>,
}

impl EqualizerSettings {
    pub(crate) fn to_engine(&self) -> infrasonic_engine::Equalizer {
        infrasonic_engine::Equalizer::new(self.preamp_db, &self.gains_db)
    }
}

/// Center frequencies of the equalizer bands in Hz.
#[uniffi::export]
pub fn equalizer_bands() -> Vec<f32> {
    infrasonic_engine::eq::BANDS.to_vec()
}

/// The largest boost or cut of a band (and of the preamp) in dB.
#[uniffi::export]
pub fn equalizer_max_gain_db() -> f32 {
    infrasonic_engine::eq::MAX_GAIN_DB
}

/// The gain applied before the filters in dB: the preamp, lowered by the largest boost so
/// that boosted bands cannot clip.
#[uniffi::export]
pub fn equalizer_headroom_db(settings: EqualizerSettings) -> f32 {
    settings.to_engine().headroom_db()
}
