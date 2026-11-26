// SPDX-License-Identifier: GPL-3.0-only
//! Page 4: server search with artists, albums and songs side by side.

use crossterm::event::{KeyCode, KeyEvent};
use infrasonic_subsonic::{SearchOptions, SearchResult3};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::app::{App, Enqueue, Msg};
use crate::entry::Entry;
use crate::keys::Action;
use crate::list::{ListLike, ListView};
use crate::t;
use crate::theme;
use crate::ui::{self, Column};

#[derive(Debug, Default)]
pub struct SearchPage {
    pub query: String,
    pub editing: bool,
    pub searching: bool,
    /// Query of the results shown.
    pub shown: String,
    pub columns: [ListView<Entry>; 3],
    pub focus: usize,
}

impl SearchPage {
    pub fn focused(&mut self) -> &mut dyn ListLike {
        &mut self.columns[self.focus]
    }

    pub fn for_each_entry(&mut self, mut f: impl FnMut(&mut Entry)) {
        for c in &mut self.columns {
            c.items_mut().iter_mut().for_each(&mut f);
        }
    }

    pub fn focus_at(&mut self, col: u16, row: u16) -> bool {
        for (i, c) in self.columns.iter().enumerate() {
            let r = c.area;
            if col >= r.x && col < r.right() && row >= r.y && row < r.bottom() {
                self.focus = i;
                return true;
            }
        }
        false
    }
}

impl App {
    pub fn search_input_key(&mut self, k: KeyEvent) {
        let s = &mut self.search;
        match k.code {
            KeyCode::Esc => s.editing = false,
            KeyCode::Enter => {
                s.editing = false;
                let q = s.query.trim().to_string();
                if !q.is_empty() {
                    self.search_run(q);
                }
            }
            KeyCode::Backspace => {
                s.query.pop();
            }
            KeyCode::Char(c) => s.query.push(c),
            _ => {}
        }
    }

    fn search_run(&mut self, query: String) {
        self.search.searching = true;
        let s = self.session.clone();
        self.spawn(async move {
            let opts = SearchOptions {
                artist_count: 50,
                album_count: 100,
                song_count: 200,
                ..Default::default()
            };
            let result = s
                .client()
                .search3(&query, opts)
                .await
                .map_err(|e| e.to_string());
            Msg::Search { query, result }
        });
    }

    pub fn search_loaded(&mut self, query: String, r: Result<SearchResult3, String>) {
        if query.trim() != self.search.query.trim() {
            return;
        }
        self.search.searching = false;
        match r {
            Ok(r) => {
                let s = &mut self.search;
                s.shown = query;
                s.columns[0].set_items(r.artist.into_iter().map(Entry::from_artist).collect());
                s.columns[1].set_items(r.album.into_iter().map(Entry::from_album).collect());
                s.columns[2].set_items(r.song.into_iter().map(Entry::from_child).collect());
                for c in &mut s.columns {
                    c.top();
                }
                s.focus = s.columns.iter().position(|c| !c.is_empty()).unwrap_or(0);
            }
            Err(e) => self.error(t!("error.search", e = e)),
        }
    }

    pub fn search_action(&mut self, a: Action) {
        let s = &mut self.search;
        let sel = s.columns[s.focus].selected().cloned();
        match a {
            Action::EditQuery => s.editing = true,
            Action::Left => s.focus = s.focus.saturating_sub(1),
            Action::Right => s.focus = (s.focus + 1).min(2),
            Action::FocusNext => s.focus = (s.focus + 1) % 3,
            Action::Back => s.editing = true,
            Action::Activate => match sel {
                Some(Entry::Song(c)) => {
                    let songs: Vec<_> = s.columns[2]
                        .visible_items()
                        .into_iter()
                        .filter_map(|e| e.song().cloned())
                        .collect();
                    let start = songs.iter().position(|x| x.id == c.id).unwrap_or(0);
                    self.enqueue(songs, Enqueue::Replace(start));
                }
                Some(e) => self.enqueue_entry(e, Enqueue::Replace(0)),
                None => {}
            },
            Action::Append | Action::PlayNext => {
                if let Some(e) = sel {
                    let how = if a == Action::Append {
                        Enqueue::Append
                    } else {
                        Enqueue::Next
                    };
                    self.enqueue_entry(e, how);
                }
            }
            Action::Star => {
                if let Some(e) = sel {
                    self.toggle_star(e.star_target(), e.starred());
                }
            }
            Action::Rate => {
                if let Some(e) = sel {
                    self.rate(e.id().to_string(), e.name().to_string());
                }
            }
            Action::Pin => {
                if let Some(e) = sel {
                    self.pin_entry(e);
                }
            }
            Action::AddToPlaylist => {
                if let Some(e) = sel {
                    let s = self.session.clone();
                    self.spawn(async move { Msg::PickPlaylist(e.songs(&s).await) });
                }
            }
            _ => {}
        }
    }
}

pub fn draw(app: &mut App, f: &mut Frame, area: Rect) {
    let [input, results] =
        Layout::vertical([Constraint::Length(3), Constraint::Min(0)]).areas(area);
    let s = &mut app.search;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::border(s.editing))
        .title(format!(" {} ", t!("page.search")));
    let mut spans = vec![Span::raw(s.query.clone())];
    if s.editing {
        spans.push(Span::styled("█", theme::accent()));
    } else if s.query.is_empty() {
        spans.push(Span::styled(t!("search.hint"), theme::dim()));
    }
    f.render_widget(Paragraph::new(Line::from(spans)).block(block), input);
    let cols = Layout::horizontal([
        Constraint::Percentage(25),
        Constraint::Percentage(30),
        Constraint::Percentage(45),
    ])
    .split(results);
    let searching = s.searching;
    let focus = s.focus;
    let editing = s.editing;
    for (i, list) in s.columns.iter_mut().enumerate() {
        ui::draw_list(
            f,
            cols[i],
            list,
            &Column {
                title: [
                    t!("search.artists"),
                    t!("search.albums"),
                    t!("search.songs"),
                ][i]
                    .into(),
                focused: !editing && focus == i,
                loading: searching,
                empty: "",
            },
            |e| ui::entry_line(e, true),
        );
    }
}
