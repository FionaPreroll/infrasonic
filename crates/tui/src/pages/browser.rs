// SPDX-License-Identifier: GPL-3.0-only
//! Page 1: artists on the left, the selected artist's directories, albums and songs on the right.

use std::time::{Duration, Instant};

use infrasonic_core::config::BrowseMode;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::Line;

use crate::app::{App, Enqueue, Msg};
use crate::entry::Entry;
use crate::keys::Action;
use crate::list::{ListLike, ListView};
use crate::ui::{self, Column};

/// Delay before the artist under the cursor is loaded.
const PREVIEW_DELAY: Duration = Duration::from_millis(150);

/// One level of the right column.
#[derive(Debug, Clone)]
pub struct Level {
    pub entry: Entry,
    pub list: ListView<Entry>,
    pub loading: bool,
}

#[derive(Debug)]
pub struct Browser {
    pub mode: BrowseMode,
    pub artists: ListView<Entry>,
    pub stack: Vec<Level>,
    pub focus_right: bool,
    pub loading_root: bool,
    token: u64,
    preview: Option<(String, Instant)>,
}

impl Browser {
    pub fn new(mode: BrowseMode) -> Self {
        Self {
            mode,
            artists: ListView::default(),
            stack: Vec::new(),
            focus_right: false,
            loading_root: false,
            token: 0,
            preview: None,
        }
    }

    pub fn focused(&mut self) -> Option<&mut dyn ListLike> {
        if self.focus_right {
            self.stack
                .last_mut()
                .map(|f| &mut f.list as &mut dyn ListLike)
        } else {
            Some(&mut self.artists)
        }
    }

    /// The entry under the cursor of the focused column.
    pub fn selected(&self) -> Option<&Entry> {
        if self.focus_right {
            self.stack.last().and_then(|f| f.list.selected())
        } else {
            self.artists.selected()
        }
    }

    pub fn for_each_entry(&mut self, mut f: impl FnMut(&mut Entry)) {
        self.artists.items_mut().iter_mut().for_each(&mut f);
        for fr in &mut self.stack {
            f(&mut fr.entry);
            fr.list.items_mut().iter_mut().for_each(&mut f);
        }
    }

    pub fn focus_at(&mut self, col: u16, row: u16) -> bool {
        let inside = |r: Rect| col >= r.x && col < r.right() && row >= r.y && row < r.bottom();
        if inside(self.artists.area) {
            self.focus_right = false;
            true
        } else if self.stack.last().is_some_and(|f| inside(f.list.area)) {
            self.focus_right = true;
            true
        } else {
            false
        }
    }
}

impl App {
    pub fn browser_load_root(&mut self) {
        self.browser.token += 1;
        self.browser.loading_root = true;
        let token = self.browser.token;
        let s = self.session.clone();
        let mode = self.browser.mode;
        self.spawn(async move {
            let c = s.client();
            let result = match mode {
                BrowseMode::Folders => c.get_indexes(None).await.map(|idx| {
                    let mut v: Vec<Entry> = idx
                        .index
                        .into_iter()
                        .flat_map(|i| i.artist)
                        .map(Entry::from_index_artist)
                        .collect();
                    v.extend(idx.child.into_iter().map(Entry::from_child));
                    v
                }),
                BrowseMode::Tags => c.get_artists().await.map(|a| {
                    a.index
                        .into_iter()
                        .flat_map(|i| i.artist)
                        .map(Entry::from_artist)
                        .collect()
                }),
            };
            Msg::BrowserRoot {
                token,
                result: result.map_err(|e| e.to_string()),
            }
        });
    }

    pub fn browser_root_loaded(&mut self, token: u64, result: Result<Vec<Entry>, String>) {
        if token != self.browser.token {
            return;
        }
        self.browser.loading_root = false;
        match result {
            Ok(items) => {
                let keep = self.browser.artists.selected().map(|e| e.id().to_string());
                self.browser.artists.set_items(items);
                if let Some(id) = keep {
                    self.browser.artists.select_where(|e| e.id() == id);
                }
            }
            Err(e) => self.error(format!("Loading artists failed: {e}")),
        }
    }

    /// Show `entry` in the right column, as a new level or as the new root.
    fn browser_open(&mut self, entry: Entry, push: bool) {
        let frame = crate::pages::browser::Level {
            entry: entry.clone(),
            list: ListView::default(),
            loading: true,
        };
        if push {
            self.browser.stack.push(frame);
        } else {
            self.browser.stack = vec![frame];
        }
        let s = self.session.clone();
        self.spawn(async move {
            Msg::BrowserDir {
                id: entry.id().to_string(),
                result: entry.children(&s).await,
            }
        });
    }

    pub fn browser_dir_loaded(&mut self, id: &str, result: Result<Vec<Entry>, String>) {
        let Some(frame) = self
            .browser
            .stack
            .iter_mut()
            .rev()
            .find(|f| f.entry.id() == id && f.loading)
        else {
            return;
        };
        frame.loading = false;
        match result {
            Ok(items) => {
                let keep = frame.list.selected().map(|e| e.id().to_string());
                frame.list.set_items(items);
                if let Some(k) = keep {
                    frame.list.select_where(|e| e.id() == k);
                }
            }
            Err(e) => self.error(format!("Loading failed: {e}")),
        }
    }

    /// Load the artist under the cursor once it stays there for a moment.
    pub fn browser_tick(&mut self) {
        let Some(sel) = self.browser.artists.selected() else {
            return;
        };
        if sel.is_song()
            || self
                .browser
                .stack
                .first()
                .is_some_and(|f| f.entry.id() == sel.id())
        {
            self.browser.preview = None;
            return;
        }
        let id = sel.id().to_string();
        match &self.browser.preview {
            Some((p, at)) if *p == id => {
                if at.elapsed() >= PREVIEW_DELAY {
                    self.browser.preview = None;
                    let e = sel.clone();
                    self.browser_open(e, false);
                }
            }
            _ => self.browser.preview = Some((id, Instant::now())),
        }
    }

    /// Songs of the current right-hand level.
    fn browser_level_songs(&self) -> Vec<infrasonic_subsonic::Child> {
        self.browser
            .stack
            .last()
            .map(|f| {
                f.list
                    .visible_items()
                    .into_iter()
                    .filter_map(|e| e.song().cloned())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn browser_action(&mut self, a: Action) {
        let b = &mut self.browser;
        match a {
            Action::Left | Action::Back => {
                if b.focus_right {
                    if b.stack.len() > 1 {
                        let popped = b.stack.pop().map(|f| f.entry.id().to_string());
                        if let (Some(id), Some(top)) = (popped, b.stack.last_mut()) {
                            top.list.select_where(|e| e.id() == id);
                        }
                    } else {
                        b.focus_right = false;
                    }
                }
            }
            Action::FocusNext => {
                b.focus_right = !b.focus_right && !b.stack.is_empty();
            }
            Action::Right | Action::Activate => {
                let Some(sel) = b.selected().cloned() else {
                    return;
                };
                match (&sel, b.focus_right) {
                    (Entry::Song(c), false) => {
                        let c = (**c).clone();
                        self.enqueue(vec![c], Enqueue::Replace(0));
                    }
                    (_, false) => {
                        if b.stack.first().is_none_or(|f| f.entry.id() != sel.id()) {
                            self.browser_open(sel, false);
                        }
                        self.browser.focus_right = true;
                    }
                    (Entry::Song(c), true) if a == Action::Activate => {
                        let songs = self.browser_level_songs();
                        let start = songs.iter().position(|s| s.id == c.id).unwrap_or(0);
                        self.enqueue(songs, Enqueue::Replace(start));
                    }
                    (Entry::Song(_), true) => {}
                    (_, true) => self.browser_open(sel, true),
                }
            }
            Action::Append | Action::PlayNext => {
                if let Some(sel) = b.selected().cloned() {
                    let how = if a == Action::Append {
                        Enqueue::Append
                    } else {
                        Enqueue::Next
                    };
                    self.enqueue_entry(sel, how);
                }
            }
            Action::Star => {
                if let Some(sel) = b.selected() {
                    let (t, s) = (sel.star_target(), sel.starred());
                    self.toggle_star(t, s);
                }
            }
            Action::AddToPlaylist => {
                if let Some(sel) = b.selected().cloned() {
                    let s = self.session.clone();
                    self.spawn(async move { Msg::PickPlaylist(sel.songs(&s).await) });
                }
            }
            Action::Refresh => {
                self.browser_load_root();
                if let Some(top) = self.browser.stack.pop() {
                    let push = !self.browser.stack.is_empty();
                    self.browser_open(top.entry, push);
                }
            }
            Action::Similar => {
                if let Some(sel) = b.selected().cloned() {
                    let s = self.session.clone();
                    self.info(format!("Looking for songs similar to {}…", sel.name()));
                    self.spawn(async move {
                        let c = s.client();
                        let r = match &sel {
                            Entry::Artist { id, id3: true, .. } => {
                                c.get_similar_songs2(id, 50).await
                            }
                            _ => c.get_similar_songs(sel.id(), 50).await,
                        };
                        Msg::Songs {
                            how: Enqueue::Append,
                            result: r.map_err(|e| e.to_string()),
                        }
                    });
                }
            }
            _ => {}
        }
    }
}

pub fn draw(app: &mut App, f: &mut Frame, area: Rect) {
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(35), Constraint::Percentage(65)]).areas(area);
    let b = &mut app.browser;
    let title = match b.mode {
        BrowseMode::Folders => "Artists",
        BrowseMode::Tags => "Artists (tags)",
    };
    let loading = b.loading_root;
    ui::draw_list(
        f,
        left,
        &mut b.artists,
        &Column {
            title: title.into(),
            focused: !b.focus_right,
            loading,
            empty: "No artists",
        },
        |e| ui::entry_line(e, false),
    );
    let focus_right = b.focus_right;
    let depth = b.stack.len();
    match b.stack.last_mut() {
        Some(level) => {
            let name = level.entry.name();
            let title = if depth > 1 {
                format!("… / {name}")
            } else {
                name.to_string()
            };
            let loading = level.loading;
            ui::draw_list(
                f,
                right,
                &mut level.list,
                &Column {
                    title,
                    focused: focus_right,
                    loading,
                    empty: "Empty",
                },
                |e| ui::entry_line(e, true),
            );
        }
        None => {
            let hint = Line::from("Select an artist and press Enter").centered();
            ui::draw_placeholder(f, right, "", hint);
        }
    }
}
