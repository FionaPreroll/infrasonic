// SPDX-License-Identifier: GPL-3.0-only
//! Layout and shared widgets.

use infrasonic_core::engine::PlayState;
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Flex, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Cell, Clear, LineGauge, Paragraph, Row, Scrollbar, ScrollbarOrientation,
    ScrollbarState, Table, TableState, Wrap,
};
use unicode_width::UnicodeWidthStr;

use infrasonic_subsonic::Child;

use crate::app::{App, Overlay, Page, PickerItem};
use crate::entry::{Entry, fmt_time};
use crate::keys::Context;
use crate::list::ListView;
use crate::pages;
use crate::palette::ItemKind;
use crate::t;
use crate::theme;

/// Title and state of a list column.
#[derive(Debug)]
pub struct Column {
    pub title: String,
    pub focused: bool,
    pub loading: bool,
    pub empty: &'static str,
}

impl Column {
    fn block<T>(&self, list: &ListView<T>, spinner: &str) -> Block<'static> {
        let mut title = vec![Span::raw(format!(" {} ", self.title))];
        if self.loading {
            title.push(Span::styled(format!("{spinner} "), theme::accent()));
        }
        if list.filtering || !list.filter.is_empty() {
            let cursor = if list.filtering { "█" } else { "" };
            title.push(Span::styled(
                format!("/{}{cursor} ", list.filter),
                theme::warn(),
            ));
        }
        let count = if !list.is_empty() {
            format!(" {}/{} ", list.cursor() + 1, list.len())
        } else {
            String::new()
        };
        Block::default()
            .borders(Borders::ALL)
            .border_style(theme::border(self.focused))
            .title(Line::from(title))
            .title_bottom(Line::from(Span::styled(count, theme::dim())).right_aligned())
    }
}

thread_local! {
    static SPINNER: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

fn spinner() -> String {
    SPINNER.with(|s| s.borrow().clone())
}

/// A one-column list.
pub fn draw_list<T>(
    f: &mut Frame,
    area: Rect,
    list: &mut ListView<T>,
    col: &Column,
    line: impl Fn(&T) -> Line<'static>,
) {
    draw_table(
        f,
        area,
        list,
        col,
        Row::default(),
        &[Constraint::Fill(1)],
        |t| Row::new(vec![line(t)]),
    );
}

/// A table with optional header; scrolls so that the selection stays visible.
pub fn draw_table<T>(
    f: &mut Frame,
    area: Rect,
    list: &mut ListView<T>,
    col: &Column,
    header: Row<'static>,
    widths: &[Constraint],
    row: impl Fn(&T) -> Row<'static>,
) {
    let has_header = widths.len() > 1;
    list.area = area;
    list.header_rows = u16::from(has_header);
    let block = col.block(list, &spinner());
    let inner = block.inner(area);
    f.render_widget(block, area);
    if list.is_empty() {
        if !col.loading && !col.empty.is_empty() {
            f.render_widget(
                Paragraph::new(col.empty)
                    .style(theme::dim())
                    .wrap(Wrap { trim: true }),
                inner,
            );
        }
        return;
    }
    let body = inner.height.saturating_sub(u16::from(has_header)) as usize;
    list.scroll_to_selection(body);
    let rows: Vec<Row> = list
        .visible_items()
        .into_iter()
        .skip(list.offset)
        .take(body)
        .map(&row)
        .collect();
    let mut table = Table::new(rows, widths.to_vec())
        .row_highlight_style(theme::selected(col.focused))
        .column_spacing(1);
    if has_header {
        table = table.header(header);
    }
    let mut state = TableState::default().with_selected(Some(list.cursor() - list.offset));
    f.render_stateful_widget(table, inner, &mut state);
    if list.len() > body {
        let mut sb = ScrollbarState::new(list.len().saturating_sub(body)).position(list.offset);
        f.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .track_symbol(None)
                .thumb_style(theme::dim()),
            area.inner(ratatui::layout::Margin {
                vertical: 1,
                horizontal: 0,
            }),
            &mut sb,
        );
    }
}

pub fn draw_placeholder(f: &mut Frame, area: Rect, title: &str, text: Line<'static>) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::border(false))
        .title(format!(" {title} "));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let [mid] = Layout::vertical([Constraint::Length(1)])
        .flex(Flex::Center)
        .areas(inner);
    f.render_widget(Paragraph::new(text).style(theme::dim()), mid);
}

/// `★★★☆☆` for a rating of 3.
pub fn rating_stars(r: u8) -> String {
    let r = usize::from(r.min(5));
    format!("{}{}", "★".repeat(r), "☆".repeat(5 - r))
}

/// Star and rating after a song title.
fn song_marks(c: &Child) -> Vec<Span<'static>> {
    let mut v = Vec::new();
    if c.is_starred() {
        v.push(Span::styled(" ★", theme::accent()));
    }
    if let Some(r) = c.user_rating.filter(|r| *r > 0) {
        v.push(Span::styled(format!(" {r}/5"), theme::dim()));
    }
    v
}

/// A song title with its marks.
pub fn song_title(c: &Child) -> Line<'static> {
    let mut v = vec![Span::raw(c.title.clone())];
    v.extend(song_marks(c));
    Line::from(v)
}

/// A library entry as one line. `detail` adds track numbers, years and durations.
pub fn entry_line(e: &Entry, detail: bool) -> Line<'static> {
    let star = if e.starred() {
        Span::styled(" ★", theme::accent())
    } else {
        Span::raw("")
    };
    match e {
        Entry::Artist { name, .. } => Line::from(vec![Span::raw(name.clone()), star]),
        Entry::Folder { name, .. } => Line::from(vec![
            Span::styled("▸ ", theme::dim()),
            Span::raw(name.clone()),
            star,
        ]),
        Entry::Album {
            name, artist, year, ..
        } => {
            let mut v = vec![Span::styled("◉ ", theme::dim()), Span::raw(name.clone())];
            if detail {
                let mut extra = String::new();
                if let Some(y) = year {
                    extra.push_str(&format!(" ({y})"));
                }
                if !artist.is_empty() {
                    extra.push_str(&format!(" · {artist}"));
                }
                v.push(Span::styled(extra, theme::dim()));
            }
            v.push(star);
            Line::from(v)
        }
        Entry::Song(c) => {
            let mut v = Vec::new();
            if detail && let Some(t) = c.track {
                v.push(Span::styled(format!("{t:>2}. "), theme::dim()));
            }
            v.push(Span::raw(c.title.clone()));
            if detail {
                let mut extra = String::new();
                let artist = c.artist_name();
                if !artist.is_empty() {
                    extra.push_str(&format!(" · {artist}"));
                }
                if let Some(d) = c.duration {
                    extra.push_str(&format!(" · {}", fmt_time(f64::from(d))));
                }
                v.push(Span::styled(extra, theme::dim()));
            }
            v.extend(song_marks(c));
            Line::from(v)
        }
    }
}

/// Draw the whole screen.
pub fn draw(app: &mut App, f: &mut Frame) {
    let frames: Vec<char> = app.ui.spinner.chars().collect();
    let spin = if frames.is_empty() {
        String::new()
    } else {
        frames[app.spinner_frame % frames.len()].to_string()
    };
    SPINNER.with(|s| *s.borrow_mut() = spin.clone());

    let [tabs, main, player, status] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(3),
        Constraint::Length(1),
    ])
    .areas(f.area());
    draw_tabs(app, f, tabs, &spin);
    match app.page {
        Page::Browser => pages::browser::draw(app, f, main),
        Page::Queue => pages::queue::draw(app, f, main),
        Page::Playlists => pages::playlists::draw(app, f, main),
        Page::Search => pages::search::draw(app, f, main),
        Page::Log => pages::logpage::draw(&mut app.log, f, main),
        Page::Library => pages::library::draw(app, f, main),
        Page::Lyrics => pages::lyrics::draw(app, f, main),
    }
    draw_player(app, f, player);
    draw_status(app, f, status);
    if app.overlay.is_some() {
        draw_overlay(app, f);
    }
}

fn draw_tabs(app: &mut App, f: &mut Frame, area: Rect, spin: &str) {
    app.tab_areas.clear();
    let mut spans = Vec::new();
    let mut x = area.x;
    for (i, p) in Page::ALL.iter().enumerate() {
        let label = format!(" {} {} ", i + 1, p.title());
        let w = label.width() as u16;
        let style = if *p == app.page {
            theme::selected(true).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        app.tab_areas.push((*p, Rect::new(x, area.y, w, 1)));
        x += w + 1;
        spans.push(Span::styled(label, style));
        spans.push(Span::raw(" "));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
    let info = app.session.server_info();
    let mut right = if app.session.is_online() {
        format!(
            "{}@{} ",
            app.session.profile_name(),
            info.server_type.as_deref().unwrap_or("subsonic")
        )
    } else {
        format!("{} · {} ", app.session.profile_name(), t!("player.offline"))
    };
    if app.busy() > 0 {
        right = format!("{spin} {right}");
    }
    f.render_widget(
        Paragraph::new(Span::styled(right, theme::dim())).alignment(Alignment::Right),
        area,
    );
}

fn draw_player(app: &mut App, f: &mut Frame, area: Rect) {
    let snap = &app.snap;
    let icon = match snap.state {
        PlayState::Playing => "▶",
        PlayState::Paused => "⏸",
        PlayState::Buffering => "…",
        PlayState::Stopped => "■",
    };
    let mut flags = Vec::new();
    let repeat = app.repeat_label();
    if !repeat.is_empty() {
        flags.push(repeat.to_string());
    }
    if let Some(db) = snap.replay_gain_db {
        flags.push(format!("RG {db:+.1} dB"));
    }
    flags.push(t!("player.volume", n = snap.volume));
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(theme::dim())
        .title(Line::from(format!(" {} ", flags.join(" · "))).right_aligned());
    let inner = block.inner(area);
    f.render_widget(block, area);
    let [line1, line2] =
        Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(inner);
    let title = match &snap.current {
        Some(c) => {
            let t = &c.track;
            let mut v = vec![
                Span::styled(format!("{icon} "), theme::accent()),
                Span::styled(
                    t.title.clone(),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
            ];
            if !t.artist.is_empty() {
                v.push(Span::raw(format!("  {}", t.artist)));
            }
            if !t.album.is_empty() {
                v.push(Span::styled(format!("  {}", t.album), theme::dim()));
            }
            if t.starred {
                v.push(Span::styled(" ★", theme::accent()));
            }
            Line::from(v)
        }
        None => Line::from(vec![
            Span::styled(format!("{icon} "), theme::dim()),
            Span::styled(t!("player.nothing"), theme::dim()),
        ]),
    };
    f.render_widget(Paragraph::new(title), line1);
    let (position, duration) = match snap.current {
        Some(_) => (snap.position, snap.duration),
        None => (0.0, None),
    };
    let dur = duration.unwrap_or(0.0);
    let ratio = if dur > 0.0 {
        (position / dur).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let label = format!(
        "{} / {}",
        fmt_time(position),
        duration.map(fmt_time).unwrap_or_else(|| "–:––".into())
    );
    let lw = label.width() as u16 + 1;
    let [gauge, time] =
        Layout::horizontal([Constraint::Min(1), Constraint::Length(lw)]).areas(line2);
    app.progress_area = gauge;
    f.render_widget(
        LineGauge::default()
            .ratio(ratio)
            .filled_style(theme::accent())
            .unfilled_style(theme::dim())
            .label(""),
        gauge,
    );
    f.render_widget(Paragraph::new(label).alignment(Alignment::Right), time);
}

fn draw_status(app: &mut App, f: &mut Frame, area: Rect) {
    let line = match &app.status {
        Some(s) => Line::from(Span::styled(
            s.text.clone(),
            if s.error {
                theme::error()
            } else {
                Style::default()
            },
        )),
        None => Line::from(Span::styled(t!("status.hint"), theme::dim())),
    };
    f.render_widget(Paragraph::new(line), area);
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let [r] = Layout::horizontal([Constraint::Length(w.min(area.width))])
        .flex(Flex::Center)
        .areas(area);
    let [r] = Layout::vertical([Constraint::Length(h.min(area.height))])
        .flex(Flex::Center)
        .areas(r);
    r
}

fn draw_overlay(app: &mut App, f: &mut Frame) {
    let screen = f.area();
    let contexts = app.contexts();
    let current_profile = app.session.profile_name().to_string();
    match &mut app.overlay {
        Some(Overlay::Help { scroll }) => {
            let mut lines = Vec::new();
            let mut last: Option<Context> = None;
            for (ctx, keys, help) in app.keys.help_rows() {
                if last != Some(ctx) {
                    if last.is_some() {
                        lines.push(Line::from(""));
                    }
                    lines.push(Line::from(Span::styled(ctx.title(), theme::header())));
                    last = Some(ctx);
                }
                lines.push(Line::from(vec![
                    Span::styled(format!("  {keys:<18}"), theme::accent()),
                    Span::raw(help),
                ]));
            }
            let area = centered(screen, 72, screen.height.saturating_sub(4));
            let max = (lines.len() as u16).saturating_sub(area.height.saturating_sub(2));
            *scroll = (*scroll).min(max);
            f.render_widget(Clear, area);
            f.render_widget(
                Paragraph::new(lines).scroll((*scroll, 0)).block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(theme::accent())
                        .title(format!(" {} ", t!("help.title")))
                        .title_bottom(
                            Line::from(format!(" {} ", t!("help.footer"))).right_aligned(),
                        ),
                ),
                area,
            );
        }
        Some(Overlay::Prompt { title, input, .. }) => {
            let area = centered(screen, 60, 3);
            f.render_widget(Clear, area);
            f.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::raw(input.clone()),
                    Span::styled("█", theme::accent()),
                ]))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(theme::accent())
                        .title(format!(" {title} "))
                        .title_bottom(
                            Line::from(format!(" {} ", t!("prompt.footer"))).right_aligned(),
                        ),
                ),
                area,
            );
        }
        Some(Overlay::Confirm { text, .. }) => {
            let area = centered(screen, 60, 4);
            f.render_widget(Clear, area);
            f.render_widget(
                Paragraph::new(text.clone())
                    .wrap(Wrap { trim: true })
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .border_style(theme::warn())
                            .title(format!(" {} ", t!("confirm.title")))
                            .title_bottom(
                                Line::from(format!(" {} ", t!("confirm.footer"))).right_aligned(),
                            ),
                    ),
                area,
            );
        }
        Some(Overlay::PlaylistPicker { songs, list }) => {
            let area = centered(screen, 50, 16);
            f.render_widget(Clear, area);
            let title = t!("picker.title", n = songs.len());
            draw_list(
                f,
                area,
                list,
                &Column {
                    title,
                    focused: true,
                    loading: false,
                    empty: "",
                },
                |p| match p {
                    PickerItem::New => Line::from(Span::styled(t!("picker.new"), theme::accent())),
                    PickerItem::Existing(p) => Line::from(p.name.clone()),
                },
            );
        }
        Some(Overlay::Palette { input, list }) => {
            let h = (list.len() as u16 + 5).clamp(8, 22);
            let area = centered(screen, 76, h);
            f.render_widget(Clear, area);
            let [inp, rest] =
                Layout::vertical([Constraint::Length(3), Constraint::Min(3)]).areas(area);
            f.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(":", theme::accent()),
                    Span::raw(input.clone()),
                    Span::styled("█", theme::accent()),
                ]))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(theme::accent())
                        .title(format!(" {} ", t!("palette.title")))
                        .title_bottom(
                            Line::from(format!(" {} ", t!("palette.footer"))).right_aligned(),
                        ),
                ),
                inp,
            );
            draw_table(
                f,
                rest,
                list,
                &Column {
                    title: crate::palette::title(&contexts),
                    focused: true,
                    loading: false,
                    empty: t!("palette.nothing"),
                },
                Row::default(),
                &[Constraint::Fill(1), Constraint::Length(16)],
                |i| {
                    let label = match &i.kind {
                        ItemKind::Action(_) => Line::from(i.label.clone()),
                        ItemKind::Command(c) => Line::from(vec![
                            Span::styled(format!("{c} "), theme::accent()),
                            Span::styled(i.label.clone(), theme::dim()),
                        ]),
                    };
                    Row::new(vec![
                        Cell::from(label),
                        Cell::from(Span::styled(i.keys.clone(), theme::dim())),
                    ])
                },
            );
        }
        Some(Overlay::Rate { name, .. }) => {
            let area = centered(screen, 60, 4);
            f.render_widget(Clear, area);
            f.render_widget(
                Paragraph::new(t!("rate.text"))
                    .wrap(Wrap { trim: true })
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .border_style(theme::accent())
                            .title(format!(" {} ", t!("rate.title", name = name))),
                    ),
                area,
            );
        }
        Some(Overlay::ProfilePicker { list }) => {
            let h = (list.len() as u16 + 2).clamp(4, 16);
            let area = centered(screen, 50, h);
            f.render_widget(Clear, area);
            draw_list(
                f,
                area,
                list,
                &Column {
                    title: t!("profile.title").to_string(),
                    focused: true,
                    loading: false,
                    empty: "",
                },
                |p| {
                    if *p == current_profile {
                        Line::from(vec![
                            Span::raw(p.clone()),
                            Span::styled(" ●", theme::accent()),
                        ])
                    } else {
                        Line::from(p.clone())
                    }
                },
            );
        }
        None => {}
    }
}
