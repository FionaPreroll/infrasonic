// SPDX-License-Identifier: GPL-3.0-only
//! Data types returned by the Subsonic / OpenSubsonic API (JSON format).
//!
//! Field names follow the API (`camelCase`). Everything that servers are allowed to omit is
//! optional or defaults to empty, because real servers (Navidrome, gonic, Ampache, ...) differ
//! in what they send.

use serde::{Deserialize, Serialize};

use crate::de::{id, opt_id};

/// A server-side identifier. Some servers send numbers, we always normalise to strings.
pub type Id = String;

/// Metadata about the server, returned with every response.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfo {
    #[serde(default)]
    pub version: String,
    #[serde(default, rename = "type")]
    pub server_type: Option<String>,
    #[serde(default)]
    pub server_version: Option<String>,
    #[serde(default)]
    pub open_subsonic: bool,
}

/// An OpenSubsonic extension supported by the server.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct Extension {
    pub name: String,
    #[serde(default)]
    pub versions: Vec<i32>,
}

/// ReplayGain information (OpenSubsonic).
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReplayGain {
    pub track_gain: Option<f32>,
    pub album_gain: Option<f32>,
    pub track_peak: Option<f32>,
    pub album_peak: Option<f32>,
    pub base_gain: Option<f32>,
    pub fallback_gain: Option<f32>,
}

/// A song or a directory entry (`Child` in the API).
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Child {
    #[serde(deserialize_with = "id")]
    pub id: Id,
    #[serde(default, deserialize_with = "opt_id")]
    pub parent: Option<Id>,
    #[serde(default)]
    pub is_dir: bool,
    #[serde(default)]
    pub title: String,
    pub album: Option<String>,
    pub artist: Option<String>,
    pub display_artist: Option<String>,
    pub track: Option<u32>,
    pub year: Option<i32>,
    pub genre: Option<String>,
    #[serde(default, deserialize_with = "opt_id")]
    pub cover_art: Option<Id>,
    pub size: Option<u64>,
    pub content_type: Option<String>,
    pub suffix: Option<String>,
    pub transcoded_content_type: Option<String>,
    pub transcoded_suffix: Option<String>,
    /// Duration in seconds.
    pub duration: Option<u32>,
    /// Bit rate in kbit/s.
    pub bit_rate: Option<u32>,
    pub path: Option<String>,
    pub play_count: Option<u64>,
    pub disc_number: Option<u32>,
    pub created: Option<String>,
    pub starred: Option<String>,
    #[serde(default, deserialize_with = "opt_id")]
    pub album_id: Option<Id>,
    #[serde(default, deserialize_with = "opt_id")]
    pub artist_id: Option<Id>,
    #[serde(rename = "type")]
    pub media_type: Option<String>,
    pub user_rating: Option<u8>,
    pub bpm: Option<u32>,
    pub replay_gain: Option<ReplayGain>,
    pub sampling_rate: Option<u32>,
    pub channel_count: Option<u32>,
}

impl Child {
    /// Whether the entry carries a "starred" timestamp.
    pub fn is_starred(&self) -> bool {
        self.starred.is_some()
    }

    /// Artist name to show to users, preferring the OpenSubsonic display artist.
    pub fn artist_name(&self) -> &str {
        self.display_artist
            .as_deref()
            .or(self.artist.as_deref())
            .unwrap_or("")
    }
}

/// An artist in the folder index (`getIndexes`).
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct IndexArtist {
    #[serde(deserialize_with = "id")]
    pub id: Id,
    #[serde(default)]
    pub name: String,
    pub starred: Option<String>,
    pub album_count: Option<u32>,
    #[serde(default, deserialize_with = "opt_id")]
    pub cover_art: Option<Id>,
}

/// One letter group of the folder index.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct Index {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub artist: Vec<IndexArtist>,
}

/// Result of `getIndexes`.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Indexes {
    #[serde(default)]
    pub index: Vec<Index>,
    #[serde(default)]
    pub child: Vec<Child>,
    #[serde(default)]
    pub ignored_articles: String,
}

/// Result of `getMusicDirectory`.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Directory {
    #[serde(deserialize_with = "id")]
    pub id: Id,
    #[serde(default, deserialize_with = "opt_id")]
    pub parent: Option<Id>,
    #[serde(default)]
    pub name: String,
    pub starred: Option<String>,
    #[serde(default)]
    pub child: Vec<Child>,
}

/// An artist (ID3 based browsing).
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Artist {
    #[serde(deserialize_with = "id")]
    pub id: Id,
    #[serde(default)]
    pub name: String,
    #[serde(default, deserialize_with = "opt_id")]
    pub cover_art: Option<Id>,
    pub album_count: Option<u32>,
    pub starred: Option<String>,
    #[serde(default)]
    pub album: Vec<Album>,
}

/// One letter group of `getArtists`.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct ArtistIndex {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub artist: Vec<Artist>,
}

/// Result of `getArtists`.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Artists {
    #[serde(default)]
    pub index: Vec<ArtistIndex>,
    #[serde(default)]
    pub ignored_articles: String,
}

/// An album (ID3 based browsing). `song` is filled by `getAlbum`.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Album {
    #[serde(deserialize_with = "id")]
    pub id: Id,
    #[serde(default)]
    pub name: String,
    /// Some servers send `title` in addition to (or instead of) `name`.
    pub title: Option<String>,
    pub artist: Option<String>,
    #[serde(default, deserialize_with = "opt_id")]
    pub artist_id: Option<Id>,
    #[serde(default, deserialize_with = "opt_id")]
    pub cover_art: Option<Id>,
    pub song_count: Option<u32>,
    pub duration: Option<u32>,
    pub play_count: Option<u64>,
    pub created: Option<String>,
    pub starred: Option<String>,
    pub year: Option<i32>,
    pub genre: Option<String>,
    pub user_rating: Option<u8>,
    #[serde(default)]
    pub song: Vec<Child>,
}

impl Album {
    /// The album name, falling back to `title`.
    pub fn display_name(&self) -> &str {
        if self.name.is_empty() {
            self.title.as_deref().unwrap_or("")
        } else {
            &self.name
        }
    }
}

/// Result of `search3`.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct SearchResult3 {
    #[serde(default)]
    pub artist: Vec<Artist>,
    #[serde(default)]
    pub album: Vec<Album>,
    #[serde(default)]
    pub song: Vec<Child>,
}

/// Result of `getStarred2`.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct Starred2 {
    #[serde(default)]
    pub artist: Vec<Artist>,
    #[serde(default)]
    pub album: Vec<Album>,
    #[serde(default)]
    pub song: Vec<Child>,
}

/// A playlist. `entry` is filled by `getPlaylist`.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Playlist {
    #[serde(deserialize_with = "id")]
    pub id: Id,
    #[serde(default)]
    pub name: String,
    pub comment: Option<String>,
    pub owner: Option<String>,
    #[serde(default)]
    pub public: bool,
    pub song_count: Option<u32>,
    pub duration: Option<u32>,
    pub created: Option<String>,
    pub changed: Option<String>,
    #[serde(default, deserialize_with = "opt_id")]
    pub cover_art: Option<Id>,
    #[serde(default)]
    pub entry: Vec<Child>,
}

/// The play queue saved on the server (`getPlayQueue`).
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlayQueue {
    #[serde(default)]
    pub entry: Vec<Child>,
    #[serde(default, deserialize_with = "opt_id")]
    pub current: Option<Id>,
    /// Position in the current song, in milliseconds.
    pub position: Option<u64>,
    pub username: Option<String>,
    pub changed: Option<String>,
    pub changed_by: Option<String>,
}

/// Library scan status.
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ScanStatus {
    #[serde(default)]
    pub scanning: bool,
    pub count: Option<u64>,
}

/// A genre.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Genre {
    #[serde(default)]
    pub value: String,
    pub song_count: Option<u32>,
    pub album_count: Option<u32>,
}

/// A single lyrics line. `start` is in milliseconds for synced lyrics.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct LyricsLine {
    pub start: Option<i64>,
    #[serde(default)]
    pub value: String,
}

/// Structured lyrics (OpenSubsonic `songLyrics`).
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StructuredLyrics {
    #[serde(default)]
    pub lang: String,
    #[serde(default)]
    pub synced: bool,
    #[serde(default)]
    pub line: Vec<LyricsLine>,
    pub display_artist: Option<String>,
    pub display_title: Option<String>,
    pub offset: Option<i64>,
}

/// A saved position in a song (`getBookmarks`).
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Bookmark {
    /// Position in milliseconds.
    #[serde(default)]
    pub position: u64,
    pub username: Option<String>,
    pub comment: Option<String>,
    pub created: Option<String>,
    pub changed: Option<String>,
    #[serde(default)]
    pub entry: Child,
}

/// Biography and links of an artist (`getArtistInfo2`).
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ArtistInfo {
    pub biography: Option<String>,
    pub music_brainz_id: Option<String>,
    pub last_fm_url: Option<String>,
    pub small_image_url: Option<String>,
    pub medium_image_url: Option<String>,
    pub large_image_url: Option<String>,
    #[serde(default)]
    pub similar_artist: Vec<Artist>,
}

/// Plain lyrics (`getLyrics`, Subsonic 1.2).
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct Lyrics {
    pub artist: Option<String>,
    pub title: Option<String>,
    #[serde(default)]
    pub value: String,
}

/// Kinds of album lists for `getAlbumList2`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AlbumListType {
    Random,
    Newest,
    Highest,
    Frequent,
    Recent,
    AlphabeticalByName,
    AlphabeticalByArtist,
    Starred,
    /// Albums released between two years (inclusive; `from > to` sorts descending).
    ByYear {
        from: i32,
        to: i32,
    },
}

impl AlbumListType {
    /// Value of the `type` query parameter.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Random => "random",
            Self::Newest => "newest",
            Self::Highest => "highest",
            Self::Frequent => "frequent",
            Self::Recent => "recent",
            Self::AlphabeticalByName => "alphabeticalByName",
            Self::AlphabeticalByArtist => "alphabeticalByArtist",
            Self::Starred => "starred",
            Self::ByYear { .. } => "byYear",
        }
    }
}

/// An internet radio station (`getInternetRadioStations`).
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InternetRadioStation {
    #[serde(deserialize_with = "id")]
    pub id: Id,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub stream_url: String,
    pub home_page_url: Option<String>,
}

/// A public link to songs (`getShares`).
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Share {
    #[serde(deserialize_with = "id")]
    pub id: Id,
    #[serde(default)]
    pub url: String,
    pub description: Option<String>,
    pub username: Option<String>,
    pub created: Option<String>,
    pub expires: Option<String>,
    pub last_visited: Option<String>,
    #[serde(default)]
    pub visit_count: u64,
    #[serde(default)]
    pub entry: Vec<Child>,
}
