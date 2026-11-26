// SPDX-License-Identifier: GPL-3.0-only
//! Page 2: the play queue, with details and cover art of the playing song.

use std::sync::Arc;

use infrasonic_core::engine::{Command, QueueEntry, Snapshot};
use infrasonic_subsonic::StarTarget;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Wrap};

use crate::app::{App, ConfirmAction, Overlay, PromptAction};
use crate::entry::fmt_time;
use crate::keys::Action;
use crate::list::ListView;
use crate::t;
use crate::theme;
use crate::ui;

#[derive(Debug, Default)]
pub struct QueuePage {
    pub list: ListView<QueueEntry>,
    synced: Option<Arc<Vec<QueueEntry>>>,
    /// Keep the cursor on the playing song until the user moves it.
    follow: bool,
}

impl QueuePage {
    /// Copy the queue from the engine when it changed.
    pub fn sync(&mut self, snap: &Snapshot) {
        if self
            .synced
            .as_ref()
            .is_some_and(|q| Arc::ptr_eq(q, &snap.queue))
        {
            return;
        }
        let keep = self.list.selected().map(|e| e.uid);
        self.list.set_items(snap.queue.to_vec());
        if let Some(uid) = keep {
            self.list.select_where(|e| e.uid == uid);
        }
        self.synced = Some(snap.queue.clone());
    }
}

impl App {
    /// Move the cursor to the playing song (if the user did not move it away).
    pub fn queue_follow(&mut self) {
        self.queue.sync(&self.snap);
        if (self.queue.follow || self.queue.list.selected().is_none())
            && let Some(cur) = &self.snap.current
        {
            let uid = cur.uid;
            self.queue.list.select_where(|e| e.uid == uid);
            self.queue.follow = true;
        }
    }

    pub fn queue_action(&mut self, a: Action) {
        self.queue.follow = false;
        let sel = self.queue.list.selected_index();
        match a {
            Action::Activate => {
                if let Some(i) = sel {
                    self.send(Command::PlayIndex(i));
                    self.queue.follow = true;
                }
            }
            Action::Remove => {
                if let Some(i) = sel {
                    self.send(Command::Remove(i));
                }
            }
            Action::ClearQueue => self.send(Command::Clear),
            Action::MoveUp | Action::MoveDown => {
                if self.queue.list.has_filter_applied() {
                    self.error(t!("error.filter_move"));
                    return;
                }
                if let Some(i) = sel {
                    let to = if a == Action::MoveUp {
                        i.checked_sub(1)
                    } else {
                        (i + 1 < self.queue.list.items().len()).then_some(i + 1)
                    };
                    if let Some(to) = to {
                        self.send(Command::Move { from: i, to });
                        self.queue.list.set_cursor(to);
                    }
                }
            }
            Action::Star => {
                if let Some(e) = self.queue.list.selected() {
                    let t = &e.track;
                    self.toggle_star(StarTarget::id(t.id.clone()), t.starred);
                }
            }
            Action::Rate => {
                if let Some(e) = self.queue.list.selected() {
                    let (id, name) = (e.track.id.clone(), e.track.title.clone());
                    self.rate(id, name);
                }
            }
            Action::Pin => {
                if let Some(e) = self.queue.list.selected() {
                    let c = infrasonic_core::child_from_track(&e.track);
                    self.pin_songs(vec![c]);
                }
            }
            Action::AddToPlaylist => {
                if let Some(e) = self.queue.list.selected() {
                    let id = e.track.id.clone();
                    self.pick_playlist(vec![id]);
                }
            }
            Action::SaveQueue => {
                if self.snap.queue.is_empty() {
                    self.error(t!("error.queue_empty"));
                } else {
                    self.playlists_ensure_loaded();
                    self.overlay = Some(Overlay::Prompt {
                        title: t!("prompt.save_queue").into(),
                        input: String::new(),
                        action: PromptAction::SaveQueue,
                    });
                }
            }
            Action::Shuffle => self.send(Command::Shuffle),
            Action::LoadServerQueue => {
                self.overlay = Some(Overlay::Confirm {
                    text: t!("confirm.load_server_queue").into(),
                    action: ConfirmAction::LoadServerQueue,
                });
            }
            Action::JumpToPlaying => {
                self.queue.follow = true;
                self.queue_follow();
            }
            _ => {}
        }
    }
}

pub fn draw(app: &mut App, f: &mut Frame, area: Rect) {
    let wide = area.width >= 100;
    let (list_area, side) = if wide {
        let [l, r] = Layout::horizontal([Constraint::Min(60), Constraint::Length(34)]).areas(area);
        (l, Some(r))
    } else {
        (area, None)
    };
    let current = app.snap.current.as_ref().map(|c| c.uid);
    let total: f64 = app.snap.queue.iter().filter_map(|e| e.track.duration).sum();
    let title = t!(
        "queue.title",
        n = app.snap.queue.len(),
        time = fmt_time(total)
    );
    let header = Row::new([
        "",
        t!("column.title"),
        t!("column.artist"),
        t!("column.album"),
        t!("column.time"),
    ])
    .style(theme::header());
    let widths = [
        Constraint::Length(2),
        Constraint::Fill(3),
        Constraint::Fill(2),
        Constraint::Fill(2),
        Constraint::Length(7),
    ];
    ui::draw_table(
        f,
        list_area,
        &mut app.queue.list,
        &ui::Column {
            title,
            focused: true,
            loading: false,
            empty: t!("queue.empty"),
        },
        header,
        &widths,
        |e| {
            let playing = Some(e.uid) == current;
            let t = &e.track;
            let mark = if playing {
                "▶"
            } else if t.starred {
                "★"
            } else {
                ""
            };
            let style = if playing {
                theme::playing()
            } else {
                Style::default()
            };
            Row::new(vec![
                Cell::from(mark),
                Cell::from(t.title.clone()),
                Cell::from(t.artist.clone()),
                Cell::from(t.album.clone()),
                Cell::from(t.duration.map(fmt_time).unwrap_or_default()),
            ])
            .style(style)
        },
    );
    if let Some(side) = side {
        draw_details(app, f, side);
    }
}

fn draw_details(app: &mut App, f: &mut Frame, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::border(false))
        .title(format!(" {} ", t!("queue.now_playing")));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let Some(cur) = app.snap.current.clone() else {
        f.render_widget(
            Paragraph::new(t!("player.nothing")).style(theme::dim()),
            inner,
        );
        return;
    };
    let t = &cur.track;
    let img_h = inner.width.div_ceil(2).min(inner.height.saturating_sub(6));
    let [img, text] =
        Layout::vertical([Constraint::Length(img_h), Constraint::Min(0)]).areas(inner);
    app.cover.draw(f, img);
    let mut lines = vec![
        Line::from(Span::styled(
            t.title.clone(),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(t.artist.clone()),
        Line::from(Span::styled(t.album.clone(), theme::dim())),
    ];
    let mut facts = Vec::new();
    if let Some(y) = t.year {
        facts.push(y.to_string());
    }
    if let Some(g) = &t.genre {
        facts.push(g.clone());
    }
    if let Some(s) = &t.suffix {
        facts.push(s.to_uppercase());
    }
    if let Some(b) = t.bit_rate {
        facts.push(format!("{b} kbit/s"));
    }
    if !facts.is_empty() {
        lines.push(Line::from(Span::styled(facts.join(" · "), theme::dim())));
    }
    let mut marks = Vec::new();
    if t.starred {
        marks.push(Span::styled(
            format!("★ {}", t!("queue.starred")),
            theme::accent(),
        ));
    }
    if let Some(r) = t.rating {
        marks.push(Span::styled(
            format!("  {}", ui::rating_stars(r)),
            theme::accent(),
        ));
    }
    if !marks.is_empty() {
        lines.push(Line::from(marks));
    }
    if let Some(db) = app.snap.replay_gain_db {
        lines.push(Line::from(Span::styled(
            format!("ReplayGain {db:+.1} dB"),
            theme::dim(),
        )));
    }
    if let Some(l) = app.lyrics_now() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(format!("♪ {l}"), theme::accent())));
    }
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), text);
}
