// SPDX-License-Identifier: GPL-3.0-only
//! Library items shown in the browser, search and playlist pages.

use infrasonic_core::Session;
use infrasonic_core::engine::QueueEntry;
use infrasonic_subsonic::{Album, Artist, Child, Directory, IndexArtist, Playlist, StarTarget};

use crate::list::Filterable;

/// Anything that can be listed, opened, queued or starred.
#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    /// An artist; `id3` tells whether it comes from ID3 (tag) or folder browsing.
    Artist {
        id: String,
        name: String,
        starred: bool,
        id3: bool,
    },
    /// A directory in folder browsing.
    Folder {
        id: String,
        name: String,
        starred: bool,
    },
    /// An ID3 album.
    Album {
        id: String,
        name: String,
        artist: String,
        year: Option<i32>,
        starred: bool,
    },
    Song(Box<Child>),
}

impl Entry {
    pub fn id(&self) -> &str {
        match self {
            Self::Artist { id, .. } | Self::Folder { id, .. } | Self::Album { id, .. } => id,
            Self::Song(c) => &c.id,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Self::Artist { name, .. } | Self::Folder { name, .. } | Self::Album { name, .. } => {
                name
            }
            Self::Song(c) => &c.title,
        }
    }

    pub fn is_song(&self) -> bool {
        matches!(self, Self::Song(_))
    }

    pub fn song(&self) -> Option<&Child> {
        match self {
            Self::Song(c) => Some(c),
            _ => None,
        }
    }

    pub fn starred(&self) -> bool {
        match self {
            Self::Artist { starred, .. }
            | Self::Folder { starred, .. }
            | Self::Album { starred, .. } => *starred,
            Self::Song(c) => c.is_starred(),
        }
    }

    pub fn set_starred(&mut self, value: bool) {
        match self {
            Self::Artist { starred, .. }
            | Self::Folder { starred, .. }
            | Self::Album { starred, .. } => *starred = value,
            Self::Song(c) => c.starred = value.then(|| "1".to_string()),
        }
    }

    pub fn star_target(&self) -> StarTarget {
        match self {
            Self::Artist { id, id3: true, .. } => StarTarget::artist(id.clone()),
            Self::Album { id, .. } => StarTarget::album(id.clone()),
            _ => StarTarget::id(self.id().to_string()),
        }
    }

    pub fn from_index_artist(a: IndexArtist) -> Self {
        Self::Artist {
            starred: a.starred.is_some(),
            id: a.id,
            name: a.name,
            id3: false,
        }
    }

    pub fn from_artist(a: Artist) -> Self {
        Self::Artist {
            starred: a.starred.is_some(),
            id: a.id,
            name: a.name,
            id3: true,
        }
    }

    pub fn from_album(a: Album) -> Self {
        Self::Album {
            name: a.display_name().to_string(),
            starred: a.starred.is_some(),
            artist: a.artist.unwrap_or_default(),
            year: a.year,
            id: a.id,
        }
    }

    pub fn from_child(c: Child) -> Self {
        if c.is_dir {
            Self::Folder {
                starred: c.starred.is_some(),
                id: c.id,
                name: c.title,
            }
        } else {
            Self::Song(Box::new(c))
        }
    }

    pub fn from_directory(d: Directory) -> Vec<Self> {
        d.child.into_iter().map(Self::from_child).collect()
    }

    /// Children of this entry (open it).
    pub async fn children(&self, s: &Session) -> Result<Vec<Entry>, String> {
        let c = s.client();
        let r = match self {
            Self::Artist { id, id3: true, .. } => c
                .get_artist(id)
                .await
                .map(|a| a.album.into_iter().map(Self::from_album).collect()),
            Self::Artist { id, .. } | Self::Folder { id, .. } => {
                c.get_music_directory(id).await.map(Self::from_directory)
            }
            Self::Album { id, .. } => c
                .get_album(id)
                .await
                .map(|a| a.song.into_iter().map(Self::from_child).collect()),
            Self::Song(_) => Ok(Vec::new()),
        };
        r.map_err(|e| e.to_string())
    }

    /// All songs below this entry.
    pub async fn songs(&self, s: &Session) -> Result<Vec<Child>, String> {
        let r = match self {
            Self::Artist { id, id3: true, .. } => s.artist_songs(id).await,
            Self::Artist { id, .. } | Self::Folder { id, .. } => s.directory_songs(id).await,
            Self::Album { id, .. } => {
                return s
                    .client()
                    .get_album(id)
                    .await
                    .map(|a| a.song)
                    .map_err(|e| e.to_string());
            }
            Self::Song(c) => return Ok(vec![(**c).clone()]),
        };
        r.map_err(|e| e.to_string())
    }
}

impl Filterable for Entry {
    fn filter_text(&self) -> String {
        match self {
            Self::Song(c) => format!(
                "{} {} {}",
                c.title,
                c.artist_name(),
                c.album.as_deref().unwrap_or("")
            ),
            Self::Album { name, artist, .. } => format!("{name} {artist}"),
            other => other.name().to_string(),
        }
    }
}

impl Filterable for QueueEntry {
    fn filter_text(&self) -> String {
        format!(
            "{} {} {}",
            self.track.title, self.track.artist, self.track.album
        )
    }
}

impl Filterable for Playlist {
    fn filter_text(&self) -> String {
        self.name.clone()
    }
}

/// Seconds as `m:ss` or `h:mm:ss`.
pub fn fmt_time(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time() {
        assert_eq!(fmt_time(0.0), "0:00");
        assert_eq!(fmt_time(61.9), "1:01");
        assert_eq!(fmt_time(3725.0), "1:02:05");
        assert_eq!(fmt_time(-3.0), "0:00");
    }

    #[test]
    fn star_targets() {
        let a = Entry::Artist {
            id: "1".into(),
            name: "x".into(),
            starred: false,
            id3: true,
        };
        assert_eq!(a.star_target(), StarTarget::artist("1"));
        let mut s = Entry::from_child(Child {
            id: "9".into(),
            ..Default::default()
        });
        assert_eq!(s.star_target(), StarTarget::id("9"));
        s.set_starred(true);
        assert!(s.starred());
    }
}
