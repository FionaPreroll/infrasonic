// SPDX-License-Identifier: GPL-3.0-only
//! Page 7: lyrics of the playing song. Synchronised lyrics (OpenSubsonic `songLyrics`) follow
//! the playback position; plain lyrics (`getLyrics`) are shown as they are.

use std::sync::Arc;

use infrasonic_core::Session;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;

use crate::app::{App, Msg};
use crate::list::{Filterable, ListView};
use crate::t;
use crate::ui::{self, Column};

/// One line; `start` in seconds for synchronised lyrics.
#[derive(Debug, Clone, PartialEq)]
pub struct LyricLine {
    pub start: Option<f64>,
    pub text: String,
}

impl Filterable for LyricLine {
    fn filter_text(&self) -> String {
        self.text.clone()
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Lyrics {
    pub synced: bool,
    pub lines: Vec<LyricLine>,
}

impl Lyrics {
    /// Index of the line sung at `pos` seconds.
    pub fn line_at(&self, pos: f64) -> Option<usize> {
        if !self.synced {
            return None;
        }
        self.lines
            .iter()
            .rposition(|l| l.start.is_some_and(|s| s <= pos))
    }
}

#[derive(Debug)]
pub struct LyricsPage {
    /// Song the lyrics belong to.
    pub song: Option<String>,
    pub title: String,
    pub lyrics: Option<Lyrics>,
    pub list: ListView<LyricLine>,
    pub loading: bool,
    /// Keep the current line in view (until the user scrolls).
    pub follow: bool,
}

impl Default for LyricsPage {
    fn default() -> Self {
        Self {
            song: None,
            title: String::new(),
            lyrics: None,
            list: ListView::default(),
            loading: false,
            follow: true,
        }
    }
}

/// Synchronised lyrics if the server has them, plain ones otherwise. Missing lyrics are not
/// an error.
async fn fetch(s: Arc<Session>, id: String, artist: String, title: String) -> Lyrics {
    let c = s.client();
    if c.supports("songLyrics") {
        match c.get_lyrics_by_song_id(&id).await {
            Ok(list) => {
                let best = list
                    .iter()
                    .filter(|l| !l.line.is_empty())
                    .max_by_key(|l| l.synced);
                if let Some(l) = best {
                    let offset = l.offset.unwrap_or(0) as f64 / 1000.0;
                    return Lyrics {
                        synced: l.synced,
                        lines: l
                            .line
                            .iter()
                            .map(|x| LyricLine {
                                start: x
                                    .start
                                    .filter(|_| l.synced)
                                    .map(|ms| (ms as f64 / 1000.0 - offset).max(0.0)),
                                text: x.value.clone(),
                            })
                            .collect(),
                    };
                }
            }
            Err(e) => tracing::debug!("getLyricsBySongId: {e}"),
        }
    }
    if artist.is_empty() && title.is_empty() {
        return Lyrics::default();
    }
    match c.get_lyrics(&artist, &title).await {
        Ok(l) => Lyrics {
            synced: false,
            lines: l
                .value
                .lines()
                .map(|t| LyricLine {
                    start: None,
                    text: t.trim_end().to_string(),
                })
                .collect(),
        },
        Err(e) => {
            tracing::debug!("getLyrics: {e}");
            Lyrics::default()
        }
    }
}

impl App {
    /// Fetch the lyrics of the new song.
    pub fn lyrics_track_changed(&mut self) {
        let Some(cur) = self.snap.current.clone() else {
            return;
        };
        let t = &cur.track;
        if self.lyrics.song.as_deref() == Some(t.id.as_str()) {
            return;
        }
        self.lyrics.song = Some(t.id.clone());
        self.lyrics.title = if t.artist.is_empty() {
            t.title.clone()
        } else {
            format!("{} · {}", t.title, t.artist)
        };
        self.lyrics.lyrics = None;
        self.lyrics.list.set_items(Vec::new());
        self.lyrics.loading = true;
        self.lyrics.follow = true;
        if !self.session.is_online() {
            self.lyrics.loading = false;
            return;
        }
        let s = self.session.clone();
        let (id, artist, title) = (t.id.clone(), t.artist.clone(), t.title.clone());
        self.spawn(async move {
            let l = fetch(s, id.clone(), artist, title).await;
            Msg::Lyrics { id, result: Ok(l) }
        });
    }

    pub fn lyrics_loaded(&mut self, id: &str, result: Result<Lyrics, String>) {
        if self.lyrics.song.as_deref() != Some(id) {
            return;
        }
        self.lyrics.loading = false;
        match result {
            Ok(l) => {
                self.lyrics.list.set_items(l.lines.clone());
                self.lyrics.lyrics = Some(l);
            }
            Err(e) => self.error(e),
        }
    }

    /// Move the highlight along with the song.
    pub fn lyrics_tick(&mut self) {
        if !self.lyrics.follow {
            return;
        }
        let Some(l) = &self.lyrics.lyrics else {
            return;
        };
        if let Some(i) = l.line_at(self.snap.position)
            && !self.lyrics.list.has_filter_applied()
        {
            self.lyrics.list.set_cursor(i);
        }
    }

    /// The line sung right now, for other pages.
    pub fn lyrics_now(&self) -> Option<&str> {
        let l = self.lyrics.lyrics.as_ref()?;
        let current = self.snap.current.as_ref()?;
        if self.lyrics.song.as_deref() != Some(current.track.id.as_str()) {
            return None;
        }
        l.line_at(self.snap.position)
            .and_then(|i| l.lines.get(i))
            .map(|l| l.text.as_str())
            .filter(|t| !t.trim().is_empty())
    }
}

pub fn draw(app: &mut App, f: &mut Frame, area: Rect) {
    let p = &mut app.lyrics;
    let synced = p.lyrics.as_ref().is_some_and(|l| l.synced);
    let title = if p.title.is_empty() {
        t!("page.lyrics").to_string()
    } else if synced {
        format!("{} · {}", p.title, t!("lyrics.synced"))
    } else {
        p.title.clone()
    };
    let empty = if p.song.is_none() {
        t!("lyrics.nothing_playing")
    } else {
        t!("lyrics.none")
    };
    let height = area.height.saturating_sub(2) as usize;
    if p.follow && synced {
        // Keep the current line in the middle.
        let c = p.list.cursor();
        p.list.offset = c.saturating_sub(height / 2);
    }
    let follow = p.follow;
    ui::draw_list(
        f,
        area,
        &mut p.list,
        &Column {
            title,
            // The highlight marks the line sung right now; plain lyrics only scroll.
            focused: synced && follow,
            loading: p.loading,
            empty,
        },
        |l| Line::from(l.text.clone()).centered(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_line() {
        let l = Lyrics {
            synced: true,
            lines: [(0.5, "a"), (2.0, "b"), (4.0, "c")]
                .into_iter()
                .map(|(s, t)| LyricLine {
                    start: Some(s),
                    text: t.into(),
                })
                .collect(),
        };
        assert_eq!(l.line_at(0.0), None);
        assert_eq!(l.line_at(1.0), Some(0));
        assert_eq!(l.line_at(2.0), Some(1));
        assert_eq!(l.line_at(99.0), Some(2));
        let plain = Lyrics { synced: false, ..l };
        assert_eq!(plain.line_at(3.0), None);
    }
}
