// SPDX-License-Identifier: GPL-3.0-only
//! Page 6: the library by lists. Album lists (newest, most played, by year, by genre, …),
//! starred songs, bookmarks and songs pinned for offline use; the songs of the selected
//! album on the right.

use std::time::{Duration, Instant};

use infrasonic_subsonic::{AlbumListType, Genre};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};

use crate::app::{App, Enqueue, Msg, Overlay, PromptAction};
use crate::entry::{Entry, fmt_time};
use crate::keys::Action;
use crate::list::{Filterable, ListLike, ListView};
use crate::t;
use crate::theme;
use crate::ui::{self, Column};

const PAGE: u32 = 100;
const PREVIEW_DELAY: Duration = Duration::from_millis(150);

/// What the middle column shows.
#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    Albums(AlbumListType),
    /// Asks for years, then shows `Albums(ByYear)`.
    YearsPrompt,
    Genre(Genre),
    StarredSongs,
    Bookmarks,
    Pinned,
}

impl Source {
    pub fn label(&self) -> String {
        match self {
            Self::Albums(t) => match t {
                AlbumListType::Newest => t!("library.newest").into(),
                AlbumListType::Recent => t!("library.recent").into(),
                AlbumListType::Frequent => t!("library.frequent").into(),
                AlbumListType::Random => t!("library.random").into(),
                AlbumListType::Starred => t!("library.starred_albums").into(),
                AlbumListType::Highest => t!("library.highest").into(),
                AlbumListType::AlphabeticalByName => t!("library.by_name").into(),
                AlbumListType::AlphabeticalByArtist => t!("library.by_artist").into(),
                AlbumListType::ByYear { from, to } if from == to => from.to_string(),
                AlbumListType::ByYear { from, to } => format!("{from}–{to}"),
            },
            Self::YearsPrompt => t!("library.years").into(),
            Self::Genre(g) => g.value.clone(),
            Self::StarredSongs => t!("library.starred_songs").into(),
            Self::Bookmarks => t!("library.bookmarks").into(),
            Self::Pinned => t!("library.pinned").into(),
        }
    }

    /// Lists that can be loaded in pages.
    fn paged(&self) -> bool {
        matches!(
            self,
            Self::Albums(t) if *t != AlbumListType::Random
        ) || matches!(self, Self::Genre(_))
    }
}

impl Filterable for Source {
    fn filter_text(&self) -> String {
        self.label()
    }
}

/// A row of the middle column.
#[derive(Debug, Clone, PartialEq)]
pub struct LibItem {
    pub entry: Entry,
    /// Bookmark position in seconds.
    pub position: Option<f64>,
}

impl Filterable for LibItem {
    fn filter_text(&self) -> String {
        self.entry.filter_text()
    }
}

/// Background results for this page.
#[derive(Debug)]
pub enum LibraryMsg {
    Genres(Result<Vec<Genre>, String>),
    Items {
        token: u64,
        offset: u32,
        result: Result<Vec<LibItem>, String>,
    },
    Songs {
        album: String,
        result: Result<Vec<Entry>, String>,
    },
}

#[derive(Debug, Default)]
pub struct LibraryPage {
    pub sources: ListView<Source>,
    pub items: ListView<LibItem>,
    pub songs: ListView<Entry>,
    /// 0 sources, 1 items, 2 songs.
    pub focus: usize,
    pub shown: Option<Source>,
    pub loading_items: bool,
    pub loading_songs: bool,
    /// Album whose songs are in `songs`.
    pub album: Option<(String, String)>,
    more: bool,
    token: u64,
    loaded: bool,
    preview: Option<(String, Instant)>,
}

/// `2024` or `1990-1999` (also with `–` or `..`).
pub fn parse_years(s: &str) -> Option<(i32, i32)> {
    let s = s.trim();
    let (a, b) = s
        .split_once('-')
        .or_else(|| s.split_once('–'))
        .or_else(|| s.split_once(".."))
        .unwrap_or((s, s));
    let a: i32 = a.trim().parse().ok()?;
    let b: i32 = b.trim().trim_start_matches('.').parse().ok()?;
    ((0..=9999).contains(&a) && (0..=9999).contains(&b)).then_some((a, b))
}

impl LibraryPage {
    pub fn focused(&mut self) -> &mut dyn ListLike {
        match self.focus {
            0 => &mut self.sources,
            1 => &mut self.items,
            _ => &mut self.songs,
        }
    }

    pub fn for_each_entry(&mut self, mut f: impl FnMut(&mut Entry)) {
        self.items
            .items_mut()
            .iter_mut()
            .for_each(|i| f(&mut i.entry));
        self.songs.items_mut().iter_mut().for_each(f);
    }

    pub fn focus_at(&mut self, col: u16, row: u16) -> bool {
        let inside = |r: Rect| col >= r.x && col < r.right() && row >= r.y && row < r.bottom();
        for (i, area) in [self.sources.area, self.items.area, self.songs.area]
            .into_iter()
            .enumerate()
        {
            if inside(area) {
                self.focus = i;
                return true;
            }
        }
        false
    }

    /// A bookmark was added or removed.
    pub fn bookmarks_changed(&mut self) {
        if self.shown == Some(Source::Bookmarks) {
            self.loaded = false;
        }
    }

    pub fn pins_changed(&mut self) {
        if self.shown == Some(Source::Pinned) {
            self.loaded = false;
        }
    }

    fn selected_entry(&self) -> Option<&Entry> {
        match self.focus {
            1 => self.items.selected().map(|i| &i.entry),
            2 => self.songs.selected(),
            _ => None,
        }
    }

    fn fixed_sources() -> Vec<Source> {
        use AlbumListType as T;
        vec![
            Source::Albums(T::Newest),
            Source::Albums(T::Recent),
            Source::Albums(T::Frequent),
            Source::Albums(T::Random),
            Source::Albums(T::Starred),
            Source::Albums(T::Highest),
            Source::Albums(T::AlphabeticalByName),
            Source::Albums(T::AlphabeticalByArtist),
            Source::YearsPrompt,
            Source::StarredSongs,
            Source::Bookmarks,
            Source::Pinned,
        ]
    }
}

impl App {
    pub fn library_load_sources(&mut self) {
        if self.library.sources.is_empty() {
            self.library.sources.set_items(LibraryPage::fixed_sources());
        }
        let s = self.session.clone();
        self.spawn(async move {
            Msg::Library(LibraryMsg::Genres(
                s.client().get_genres().await.map_err(|e| e.to_string()),
            ))
        });
    }

    pub fn library_ensure_loaded(&mut self) {
        if self.library.sources.is_empty() {
            self.library_load_sources();
        }
        match &self.library.shown {
            None => self.library_open(Source::Albums(AlbumListType::Newest)),
            Some(s) if !self.library.loaded => {
                let s = s.clone();
                self.library_open(s);
            }
            _ => {}
        }
    }

    pub fn library_open_pinned(&mut self) {
        if self.library.sources.is_empty() {
            self.library.sources.set_items(LibraryPage::fixed_sources());
        }
        self.library.sources.select_where(|s| *s == Source::Pinned);
        self.library_open(Source::Pinned);
        self.library.focus = 1;
    }

    pub fn library_open_years(&mut self, from: i32, to: i32) {
        self.library_open(Source::Albums(AlbumListType::ByYear { from, to }));
        self.library.focus = 1;
    }

    /// Show `source` in the middle column.
    fn library_open(&mut self, source: Source) {
        if source == Source::YearsPrompt {
            self.overlay = Some(Overlay::Prompt {
                title: t!("prompt.years").into(),
                input: String::new(),
                action: PromptAction::Years,
            });
            return;
        }
        let lib = &mut self.library;
        lib.token += 1;
        lib.shown = Some(source.clone());
        lib.loaded = true;
        lib.more = source.paged();
        lib.items.set_items(Vec::new());
        lib.items.clear_filter();
        self.library_fetch(source, 0);
    }

    fn library_fetch(&mut self, source: Source, offset: u32) {
        let lib = &mut self.library;
        lib.loading_items = true;
        let token = lib.token;
        if source == Source::Pinned {
            let pins = self.session.cache().map(|c| c.pinned()).unwrap_or_default();
            let items = pins
                .into_iter()
                .map(|c| LibItem {
                    entry: Entry::from_child(c),
                    position: None,
                })
                .collect();
            self.library_loaded(LibraryMsg::Items {
                token,
                offset,
                result: Ok(items),
            });
            return;
        }
        let s = self.session.clone();
        self.spawn(async move {
            let c = s.client();
            let albums =
                |r: Result<Vec<infrasonic_subsonic::Album>, infrasonic_subsonic::Error>| {
                    r.map(|v| {
                        v.into_iter()
                            .map(|a| LibItem {
                                entry: Entry::from_album(a),
                                position: None,
                            })
                            .collect::<Vec<_>>()
                    })
                };
            let result = match &source {
                Source::Albums(t) => albums(c.get_album_list2(*t, PAGE, offset).await),
                Source::Genre(g) => albums(c.get_albums_by_genre(&g.value, PAGE, offset).await),
                Source::StarredSongs => c.get_starred2().await.map(|s| {
                    s.song
                        .into_iter()
                        .map(|c| LibItem {
                            entry: Entry::from_child(c),
                            position: None,
                        })
                        .collect()
                }),
                Source::Bookmarks => c.get_bookmarks().await.map(|b| {
                    b.into_iter()
                        .map(|b| LibItem {
                            entry: Entry::from_child(b.entry),
                            position: Some(b.position as f64 / 1000.0),
                        })
                        .collect()
                }),
                Source::Pinned | Source::YearsPrompt => Ok(Vec::new()),
            };
            Msg::Library(LibraryMsg::Items {
                token,
                offset,
                result: result.map_err(|e| e.to_string()),
            })
        });
    }

    pub fn library_loaded(&mut self, m: LibraryMsg) {
        let lib = &mut self.library;
        match m {
            LibraryMsg::Genres(Ok(mut genres)) => {
                genres.sort_by_key(|g| g.value.to_lowercase());
                let mut v = LibraryPage::fixed_sources();
                v.extend(
                    genres
                        .into_iter()
                        .filter(|g| g.album_count.is_none_or(|n| n > 0))
                        .map(Source::Genre),
                );
                let keep = lib.sources.selected().cloned();
                lib.sources.set_items(v);
                if let Some(k) = keep {
                    lib.sources.select_where(|s| *s == k);
                }
            }
            LibraryMsg::Genres(Err(e)) => tracing::warn!("loading genres failed: {e}"),
            LibraryMsg::Items {
                token,
                offset,
                result,
            } => {
                if token != lib.token {
                    return;
                }
                lib.loading_items = false;
                match result {
                    Ok(items) => {
                        if items.len() < PAGE as usize {
                            lib.more = false;
                        }
                        if offset == 0 {
                            let keep = lib.items.selected().map(|i| i.entry.id().to_string());
                            lib.items.set_items(items);
                            if let Some(k) = keep {
                                lib.items.select_where(|i| i.entry.id() == k);
                            }
                        } else {
                            let cursor = lib.items.cursor();
                            let mut all = lib.items.items().to_vec();
                            all.extend(items);
                            lib.items.set_items(all);
                            lib.items.set_cursor(cursor);
                        }
                    }
                    Err(e) => {
                        lib.more = false;
                        self.error(t!("error.load_list", e = e));
                    }
                }
            }
            LibraryMsg::Songs { album, result } => {
                if lib.album.as_ref().map(|(id, _)| id) != Some(&album) {
                    return;
                }
                lib.loading_songs = false;
                match result {
                    Ok(songs) => lib.songs.set_items(songs),
                    Err(e) => self.error(t!("error.load_songs", e = e)),
                }
            }
        }
    }

    fn library_open_album(&mut self, id: String, name: String) {
        let lib = &mut self.library;
        lib.album = Some((id.clone(), name));
        lib.loading_songs = true;
        lib.songs.set_items(Vec::new());
        let s = self.session.clone();
        self.spawn(async move {
            let result = s
                .client()
                .get_album(&id)
                .await
                .map(|a| a.song.into_iter().map(Entry::from_child).collect())
                .map_err(|e| e.to_string());
            Msg::Library(LibraryMsg::Songs { album: id, result })
        });
    }

    /// Load album songs under the cursor, and the next page at the end of a list.
    pub fn library_tick(&mut self) {
        if self.page != crate::app::Page::Library {
            return;
        }
        let lib = &mut self.library;
        if lib.more
            && !lib.loading_items
            && !lib.items.has_filter_applied()
            && !lib.items.is_empty()
            && lib.items.cursor() + 1 >= lib.items.len()
            && let Some(source) = lib.shown.clone()
        {
            let offset = lib.items.items().len() as u32;
            self.library_fetch(source, offset);
            return;
        }
        let Some(LibItem {
            entry: Entry::Album { id, name, .. },
            ..
        }) = self.library.items.selected()
        else {
            self.library.preview = None;
            return;
        };
        if self.library.album.as_ref().map(|(a, _)| a) == Some(id) {
            self.library.preview = None;
            return;
        }
        let (id, name) = (id.clone(), name.clone());
        match &self.library.preview {
            Some((p, at)) if *p == id => {
                if at.elapsed() >= PREVIEW_DELAY {
                    self.library.preview = None;
                    self.library_open_album(id, name);
                }
            }
            _ => self.library.preview = Some((id, Instant::now())),
        }
    }

    fn library_item_songs(&self) -> Vec<infrasonic_subsonic::Child> {
        self.library
            .items
            .visible_items()
            .into_iter()
            .filter_map(|i| i.entry.song().cloned())
            .collect()
    }

    pub fn library_action(&mut self, a: Action) {
        let lib = &mut self.library;
        match a {
            Action::Left | Action::Back => lib.focus = lib.focus.saturating_sub(1),
            Action::Right if lib.focus == 0 => {
                if let Some(s) = lib.sources.selected().cloned() {
                    if lib.shown.as_ref() != Some(&s) {
                        self.library_open(s);
                    }
                    self.library.focus = 1;
                }
            }
            Action::Right | Action::FocusNext => {
                let can_songs = lib.album.is_some()
                    && matches!(
                        lib.items.selected().map(|i| &i.entry),
                        Some(Entry::Album { .. })
                    );
                lib.focus = match lib.focus {
                    0 => 1,
                    1 if can_songs => 2,
                    1 if a == Action::Right => 1,
                    _ => 0,
                };
            }
            Action::Activate => match lib.focus {
                0 => {
                    if let Some(s) = lib.sources.selected().cloned() {
                        self.library_open(s);
                        if self.overlay.is_none() {
                            self.library.focus = 1;
                        }
                    }
                }
                1 => match lib.items.selected().cloned() {
                    Some(LibItem {
                        entry: Entry::Album { id, name, .. },
                        ..
                    }) => {
                        if lib.album.as_ref().map(|(a, _)| a) != Some(&id) {
                            self.library_open_album(id, name);
                        }
                        self.library.focus = 2;
                    }
                    Some(LibItem {
                        entry: Entry::Song(c),
                        position: Some(pos),
                    }) => self.enqueue(vec![*c], Enqueue::ReplaceAt(0, pos)),
                    Some(LibItem {
                        entry: Entry::Song(c),
                        ..
                    }) => {
                        let songs = self.library_item_songs();
                        let start = songs.iter().position(|s| s.id == c.id).unwrap_or(0);
                        self.enqueue(songs, Enqueue::Replace(start));
                    }
                    _ => {}
                },
                _ => {
                    let songs: Vec<_> = lib
                        .songs
                        .visible_items()
                        .into_iter()
                        .filter_map(|e| e.song().cloned())
                        .collect();
                    let start = lib
                        .songs
                        .selected()
                        .and_then(|e| songs.iter().position(|s| s.id == e.id()))
                        .unwrap_or(0);
                    self.enqueue(songs, Enqueue::Replace(start));
                }
            },
            Action::Append | Action::PlayNext => {
                if let Some(e) = lib.selected_entry().cloned() {
                    let how = if a == Action::Append {
                        Enqueue::Append
                    } else {
                        Enqueue::Next
                    };
                    self.enqueue_entry(e, how);
                }
            }
            Action::Star => {
                if let Some(e) = lib.selected_entry() {
                    let (t, s) = (e.star_target(), e.starred());
                    self.toggle_star(t, s);
                }
            }
            Action::Rate => {
                if let Some(e) = lib.selected_entry() {
                    let (id, name) = (e.id().to_string(), e.name().to_string());
                    self.rate(id, name);
                }
            }
            Action::Pin => {
                if let Some(e) = lib.selected_entry().cloned() {
                    self.pin_entry(e);
                }
            }
            Action::AddToPlaylist => {
                if let Some(e) = lib.selected_entry().cloned() {
                    let s = self.session.clone();
                    self.spawn(async move { Msg::PickPlaylist(e.songs(&s).await) });
                }
            }
            Action::Remove if lib.focus == 1 => {
                let Some(item) = lib.items.selected().cloned() else {
                    return;
                };
                let id = item.entry.id().to_string();
                match lib.shown {
                    Some(Source::Bookmarks) => {
                        let s = self.session.clone();
                        self.spawn(async move {
                            match s.client().delete_bookmark(&id).await {
                                Ok(()) => Msg::Info(t!("status.bookmark_deleted").into()),
                                Err(e) => Msg::Error(t!("error.bookmark", e = e)),
                            }
                        });
                        self.library.items.set_items(
                            self.library
                                .items
                                .items()
                                .iter()
                                .filter(|i| i.entry.id() != item.entry.id())
                                .cloned()
                                .collect(),
                        );
                    }
                    Some(Source::Pinned) => {
                        self.session.unpin(&[id]);
                        self.info(t!("status.unpinned", name = item.entry.name()));
                        self.library_open(Source::Pinned);
                    }
                    _ => self.error(t!("error.remove_here")),
                }
            }
            Action::Refresh => {
                self.library_load_sources();
                if let Some(s) = self.library.shown.clone() {
                    self.library_open(s);
                }
            }
            _ => {}
        }
    }
}

pub fn draw(app: &mut App, f: &mut Frame, area: Rect) {
    let lib = &mut app.library;
    // Narrow terminals show fewer columns, always including the focused one.
    let visible: Vec<usize> = match area.width {
        0..60 => vec![lib.focus],
        60..100 => {
            if lib.focus == 0 {
                vec![0, 1]
            } else {
                vec![1, 2]
            }
        }
        _ => vec![0, 1, 2],
    };
    let constraints: Vec<Constraint> = visible
        .iter()
        .map(|c| match c {
            0 if visible.len() > 1 => Constraint::Length(24),
            1 if visible.len() == 3 => Constraint::Percentage(45),
            _ => Constraint::Fill(1),
        })
        .collect();
    let areas = Layout::horizontal(constraints).split(area);
    lib.sources.area = Rect::default();
    lib.items.area = Rect::default();
    lib.songs.area = Rect::default();
    for (i, col) in visible.iter().enumerate() {
        let r = areas[i];
        match col {
            0 => ui::draw_list(
                f,
                r,
                &mut lib.sources,
                &Column {
                    title: t!("library.lists").into(),
                    focused: lib.focus == 0,
                    loading: false,
                    empty: "",
                },
                |s| match s {
                    Source::Genre(g) => Line::from(vec![
                        Span::styled("♫ ", theme::dim()),
                        Span::raw(g.value.clone()),
                        Span::styled(
                            g.album_count.map(|n| format!("  {n}")).unwrap_or_default(),
                            theme::dim(),
                        ),
                    ]),
                    other => Line::from(other.label()),
                },
            ),
            1 => {
                let title = lib.shown.as_ref().map(Source::label).unwrap_or_default();
                let empty = match lib.shown {
                    Some(Source::Pinned) => t!("library.no_pins"),
                    Some(Source::Bookmarks) => t!("library.no_bookmarks"),
                    _ => t!("library.empty"),
                };
                ui::draw_list(
                    f,
                    r,
                    &mut lib.items,
                    &Column {
                        title,
                        focused: lib.focus == 1,
                        loading: lib.loading_items,
                        empty,
                    },
                    |i| {
                        let mut line = ui::entry_line(&i.entry, true);
                        if let Some(p) = i.position {
                            line.spans.push(Span::styled(
                                format!("  @ {}", fmt_time(p)),
                                theme::accent(),
                            ));
                        }
                        line
                    },
                )
            }
            _ => {
                let title = lib
                    .album
                    .as_ref()
                    .map(|(_, n)| n.clone())
                    .unwrap_or_else(|| t!("library.songs").into());
                ui::draw_list(
                    f,
                    r,
                    &mut lib.songs,
                    &Column {
                        title,
                        focused: lib.focus == 2,
                        loading: lib.loading_songs,
                        empty: "",
                    },
                    |e| ui::entry_line(e, true),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn years() {
        assert_eq!(parse_years("2024"), Some((2024, 2024)));
        assert_eq!(parse_years("1990-1999"), Some((1990, 1999)));
        assert_eq!(parse_years("1990 – 1999"), Some((1990, 1999)));
        assert_eq!(parse_years("2000..1990"), Some((2000, 1990)));
        assert_eq!(parse_years("soon"), None);
    }
}
