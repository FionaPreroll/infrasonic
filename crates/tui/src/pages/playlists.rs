// SPDX-License-Identifier: GPL-3.0-only
//! Page 3: playlists on the left, songs of the selected one on the right.

use std::time::{Duration, Instant};

use infrasonic_subsonic::{Child, Playlist, PlaylistUpdate};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Row};

use crate::app::{App, ConfirmAction, Enqueue, Msg, Overlay, PromptAction};
use crate::entry::{Entry, fmt_time};
use crate::keys::Action;
use crate::list::{ListLike, ListView};
use crate::t;
use crate::theme;
use crate::ui::{self, Column};

const PREVIEW_DELAY: Duration = Duration::from_millis(150);

#[derive(Debug, Default)]
pub struct PlaylistsPage {
    pub lists: ListView<Playlist>,
    pub entries: ListView<Entry>,
    /// Playlist whose songs are in `entries`.
    pub shown: Option<String>,
    pub loading_entries: bool,
    pub focus_right: bool,
    loaded: bool,
    loading: bool,
    /// Something changed on the server; reload when idle.
    pub dirty: bool,
    dirty_entries: Option<String>,
    preview: Option<(String, Instant)>,
}

impl PlaylistsPage {
    pub fn focused(&mut self) -> &mut dyn ListLike {
        if self.focus_right {
            &mut self.entries
        } else {
            &mut self.lists
        }
    }

    pub fn mark_dirty(&mut self, entries_of: Option<String>) {
        self.dirty = true;
        if entries_of.is_some() {
            self.dirty_entries = entries_of;
        }
    }

    pub fn for_each_entry(&mut self, f: impl FnMut(&mut Entry)) {
        self.entries.items_mut().iter_mut().for_each(f);
    }

    pub fn focus_at(&mut self, col: u16, row: u16) -> bool {
        let inside = |r: Rect| col >= r.x && col < r.right() && row >= r.y && row < r.bottom();
        if inside(self.lists.area) {
            self.focus_right = false;
            true
        } else if inside(self.entries.area) {
            self.focus_right = true;
            true
        } else {
            false
        }
    }

    fn songs(&self) -> Vec<Child> {
        self.entries
            .items()
            .iter()
            .filter_map(|e| e.song().cloned())
            .collect()
    }
}

impl App {
    pub fn playlists_load(&mut self) {
        if self.playlists.loading {
            return;
        }
        self.playlists.loading = true;
        let s = self.session.clone();
        self.spawn(async move {
            Msg::Playlists(s.client().get_playlists().await.map_err(|e| e.to_string()))
        });
    }

    pub fn playlists_ensure_loaded(&mut self) {
        if !self.playlists.loaded {
            self.playlists_load();
        }
    }

    /// Reload after changes.
    pub fn playlists_reload(&mut self) {
        self.playlists.dirty = false;
        self.playlists_load();
        let shown = self.playlists.shown.clone();
        if let Some(id) = self.playlists.dirty_entries.take()
            && shown.as_deref() == Some(id.as_str())
        {
            self.playlist_open(id);
        }
    }

    pub fn playlists_loaded(&mut self, r: Result<Vec<Playlist>, String>) {
        self.playlists.loading = false;
        match r {
            Ok(lists) => {
                self.playlists.loaded = true;
                let keep = self.playlists.lists.selected().map(|p| p.id.clone());
                self.playlists.lists.set_items(lists);
                if let Some(id) = keep {
                    self.playlists.lists.select_where(|p| p.id == id);
                }
                if let Some(id) = &self.playlists.shown
                    && !self.playlists.lists.items().iter().any(|p| &p.id == id)
                {
                    self.playlists.shown = None;
                    self.playlists.entries.set_items(Vec::new());
                }
            }
            Err(e) => self.error(t!("error.load_playlists", e = e)),
        }
    }

    fn playlist_open(&mut self, id: String) {
        self.playlists.shown = Some(id.clone());
        self.playlists.loading_entries = true;
        let s = self.session.clone();
        self.spawn(async move {
            let result = s
                .client()
                .get_playlist(&id)
                .await
                .map(|p| p.entry)
                .map_err(|e| e.to_string());
            Msg::PlaylistEntries { id, result }
        });
    }

    pub fn playlist_entries_loaded(&mut self, id: &str, r: Result<Vec<Child>, String>) {
        if self.playlists.shown.as_deref() != Some(id) {
            return;
        }
        self.playlists.loading_entries = false;
        match r {
            Ok(songs) => {
                let cursor = self.playlists.entries.cursor();
                self.playlists
                    .entries
                    .set_items(songs.into_iter().map(Entry::from_child).collect());
                self.playlists.entries.set_cursor(cursor);
            }
            Err(e) => self.error(t!("error.load_playlist", e = e)),
        }
    }

    pub fn playlists_tick(&mut self) {
        if self.page != crate::app::Page::Playlists {
            return;
        }
        let Some(sel) = self.playlists.lists.selected() else {
            return;
        };
        if self.playlists.shown.as_deref() == Some(sel.id.as_str()) {
            self.playlists.preview = None;
            return;
        }
        let id = sel.id.clone();
        match &self.playlists.preview {
            Some((p, at)) if *p == id => {
                if at.elapsed() >= PREVIEW_DELAY {
                    self.playlists.preview = None;
                    self.playlist_open(id);
                }
            }
            _ => self.playlists.preview = Some((id, Instant::now())),
        }
    }

    /// Run `f` with the songs of the selected playlist, loading them if needed.
    fn with_selected_playlist_songs(&mut self, how: Enqueue) {
        let Some(sel) = self.playlists.lists.selected() else {
            return;
        };
        if self.playlists.shown.as_deref() == Some(sel.id.as_str())
            && !self.playlists.loading_entries
        {
            let songs = self.playlists.songs();
            self.enqueue(songs, how);
            return;
        }
        let id = sel.id.clone();
        let s = self.session.clone();
        self.spawn(async move {
            Msg::Songs {
                how,
                result: s
                    .client()
                    .get_playlist(&id)
                    .await
                    .map(|p| p.entry)
                    .map_err(|e| e.to_string()),
            }
        });
    }

    pub fn playlists_action(&mut self, a: Action) {
        let p = &mut self.playlists;
        match a {
            Action::Left | Action::Back => p.focus_right = false,
            Action::Right | Action::FocusNext if !p.focus_right => {
                p.focus_right = p.shown.is_some();
            }
            Action::FocusNext => p.focus_right = false,
            Action::Activate if !p.focus_right => {
                if let Some(sel) = p.lists.selected() {
                    if p.shown.as_deref() != Some(sel.id.as_str()) {
                        let id = sel.id.clone();
                        self.playlist_open(id);
                    }
                    self.playlists.focus_right = true;
                }
            }
            Action::Activate => {
                let start = p.entries.selected_index().unwrap_or(0);
                let songs = p.songs();
                self.enqueue(songs, Enqueue::Replace(start));
            }
            Action::Append | Action::PlayNext => {
                let how = if a == Action::Append {
                    Enqueue::Append
                } else {
                    Enqueue::Next
                };
                if p.focus_right {
                    if let Some(e) = p.entries.selected().cloned() {
                        self.enqueue_entry(e, how);
                    }
                } else {
                    self.with_selected_playlist_songs(how);
                }
            }
            Action::NewPlaylist => {
                self.overlay = Some(Overlay::Prompt {
                    title: t!("prompt.new_playlist").into(),
                    input: String::new(),
                    action: PromptAction::NewPlaylist { songs: Vec::new() },
                });
            }
            Action::DeletePlaylist if !p.focus_right => {
                if let Some(sel) = p.lists.selected() {
                    self.overlay = Some(Overlay::Confirm {
                        text: t!("confirm.delete_playlist", name = sel.name),
                        action: ConfirmAction::DeletePlaylist { id: sel.id.clone() },
                    });
                }
            }
            Action::DeletePlaylist => {
                let (Some(id), Some(idx)) = (p.shown.clone(), p.entries.selected_index()) else {
                    return;
                };
                let s = self.session.clone();
                self.spawn(async move {
                    let u = PlaylistUpdate {
                        remove_indexes: vec![idx],
                        ..Default::default()
                    };
                    match s.client().update_playlist(&id, &u).await {
                        Ok(()) => Msg::Info(t!("status.removed_from_playlist").into()),
                        Err(e) => Msg::Error(t!("error.remove", e = e)),
                    }
                });
                let shown = self.playlists.shown.clone();
                self.playlists.mark_dirty(shown);
            }
            Action::Star if p.focus_right => {
                if let Some(e) = p.entries.selected() {
                    let (t, s) = (e.star_target(), e.starred());
                    self.toggle_star(t, s);
                }
            }
            Action::Rate if p.focus_right => {
                if let Some(e) = p.entries.selected() {
                    let (id, name) = (e.id().to_string(), e.name().to_string());
                    self.rate(id, name);
                }
            }
            Action::Pin if p.focus_right => {
                if let Some(e) = p.entries.selected().cloned() {
                    self.pin_entry(e);
                }
            }
            Action::Pin => {
                let songs = p.songs();
                if p.shown.as_deref() == p.lists.selected().map(|l| l.id.as_str()) {
                    self.pin_songs(songs);
                }
            }
            Action::Refresh => {
                let shown = self.playlists.shown.clone();
                self.playlists.mark_dirty(shown);
                self.playlists_reload();
            }
            _ => {}
        }
    }
}

pub fn draw(app: &mut App, f: &mut Frame, area: Rect) {
    let p = &mut app.playlists;
    let (left, right) = if area.width < 80 {
        if p.focus_right {
            (Rect::default(), area)
        } else {
            (area, Rect::default())
        }
    } else {
        let [l, r] = Layout::horizontal([Constraint::Percentage(35), Constraint::Percentage(65)])
            .areas(area);
        (l, r)
    };
    let loading = p.loading && !p.loaded;
    ui::draw_list(
        f,
        left,
        &mut p.lists,
        &Column {
            title: t!("page.playlists").into(),
            focused: !p.focus_right,
            loading,
            empty: t!("playlists.none"),
        },
        |pl| {
            let mut spans = vec![Span::raw(pl.name.clone())];
            if let Some(n) = pl.song_count {
                spans.push(Span::styled(format!("  {n}"), theme::dim()));
            }
            Line::from(spans)
        },
    );
    let title = p
        .shown
        .as_ref()
        .and_then(|id| p.lists.items().iter().find(|x| &x.id == id))
        .map(|pl| {
            format!(
                "{} · {}",
                pl.name,
                pl.duration
                    .map(|d| fmt_time(f64::from(d)))
                    .unwrap_or_default()
            )
        })
        .unwrap_or_else(|| t!("library.songs").into());
    let header = Row::new([
        t!("column.title"),
        t!("column.artist"),
        t!("column.album"),
        t!("column.time"),
    ])
    .style(theme::header());
    let widths = [
        Constraint::Fill(3),
        Constraint::Fill(2),
        Constraint::Fill(2),
        Constraint::Length(7),
    ];
    let loading = p.loading_entries;
    let focused = p.focus_right;
    ui::draw_table(
        f,
        right,
        &mut p.entries,
        &Column {
            title,
            focused,
            loading,
            empty: t!("playlists.empty"),
        },
        header,
        &widths,
        |e| match e.song() {
            Some(c) => Row::new(vec![
                Cell::from(ui::song_title(c)),
                Cell::from(c.artist_name().to_string()),
                Cell::from(c.album.clone().unwrap_or_default()),
                Cell::from(
                    c.duration
                        .map(|d| fmt_time(f64::from(d)))
                        .unwrap_or_default(),
                ),
            ]),
            None => Row::new(vec![Cell::from(e.name().to_string())]),
        },
    );
}
