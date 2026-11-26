// SPDX-License-Identifier: GPL-3.0-only
//! Application state and the event loop.

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use crossterm::event::{Event as TermEvent, KeyEvent, KeyEventKind, MouseButton, MouseEventKind};
use futures::StreamExt;
use infrasonic_core::config::{PlaybackConfig, UiConfig};
use infrasonic_core::engine::{Command, Event, PlayState, RepeatMode, Snapshot};
use infrasonic_core::{Session, SessionEvent};
use infrasonic_subsonic::{Child, Playlist, PlaylistUpdate, StarTarget};
use ratatui::layout::Rect;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::cover::Cover;
use crate::entry::Entry;
use crate::keys::{self, Action, Context};
use crate::list::{ListLike, ListView};
use crate::logging::LogBuffer;
use crate::media::{Media, MediaAction};
use crate::pages::browser::Browser;
use crate::pages::logpage::LogPage;
use crate::pages::playlists::PlaylistsPage;
use crate::pages::queue::QueuePage;
use crate::pages::search::SearchPage;

/// The pages, in tab order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Browser,
    Queue,
    Playlists,
    Search,
    Log,
}

impl Page {
    pub const ALL: [Page; 5] = [
        Page::Browser,
        Page::Queue,
        Page::Playlists,
        Page::Search,
        Page::Log,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Browser => "Browser",
            Self::Queue => "Queue",
            Self::Playlists => "Playlists",
            Self::Search => "Search",
            Self::Log => "Log",
        }
    }

    fn context(self) -> Option<Context> {
        match self {
            Self::Browser => Some(Context::Browser),
            Self::Queue => Some(Context::Queue),
            Self::Playlists => Some(Context::Playlists),
            Self::Search => Some(Context::Search),
            Self::Log => None,
        }
    }
}

/// How songs go into the queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Enqueue {
    /// Replace the queue and start playing at this index.
    Replace(usize),
    Append,
    Next,
}

/// Results of background work.
#[derive(Debug)]
pub enum Msg {
    BrowserRoot {
        token: u64,
        result: Result<Vec<Entry>, String>,
    },
    BrowserDir {
        id: String,
        result: Result<Vec<Entry>, String>,
    },
    Playlists(Result<Vec<Playlist>, String>),
    PlaylistEntries {
        id: String,
        result: Result<Vec<Child>, String>,
    },
    Search {
        query: String,
        result: Result<infrasonic_subsonic::SearchResult3, String>,
    },
    Songs {
        how: Enqueue,
        result: Result<Vec<Child>, String>,
    },
    /// Songs to add to a playlist (opens the picker).
    PickPlaylist(Result<Vec<Child>, String>),
    Starred {
        target: StarTarget,
        starred: bool,
    },
    Info(String),
    Error(String),
    Cover(crate::cover::CoverMsg),
    Media(MediaAction),
}

/// Text prompts.
#[derive(Debug, Clone, PartialEq)]
pub enum PromptAction {
    NewPlaylist { songs: Vec<String> },
    SaveQueue,
}

/// Questions that need a yes.
#[derive(Debug, Clone, PartialEq)]
pub enum ConfirmAction {
    DeletePlaylist { id: String },
    OverwritePlaylist { id: String },
    LoadServerQueue,
}

/// An entry of the playlist picker.
#[derive(Debug, Clone, PartialEq)]
pub enum PickerItem {
    New,
    Existing(Playlist),
}

impl crate::list::Filterable for PickerItem {
    fn filter_text(&self) -> String {
        match self {
            Self::New => "new playlist".into(),
            Self::Existing(p) => p.name.clone(),
        }
    }
}

/// Modal windows on top of the page.
#[derive(Debug)]
pub enum Overlay {
    Help {
        scroll: u16,
    },
    Prompt {
        title: String,
        input: String,
        action: PromptAction,
    },
    Confirm {
        text: String,
        action: ConfirmAction,
    },
    PlaylistPicker {
        songs: Vec<String>,
        list: ListView<PickerItem>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub text: String,
    pub error: bool,
    pub at: Instant,
}

/// The whole UI state.
#[derive(Debug)]
pub struct App {
    pub session: Arc<Session>,
    tx: UnboundedSender<Msg>,
    busy: Arc<AtomicUsize>,
    pub ui: UiConfig,
    pub playback: PlaybackConfig,
    pub page: Page,
    pub browser: Browser,
    pub queue: QueuePage,
    pub playlists: PlaylistsPage,
    pub search: SearchPage,
    pub log: LogPage,
    pub overlay: Option<Overlay>,
    pub status: Option<Status>,
    pub snap: Snapshot,
    pub quit: bool,
    pub redraw_all: bool,
    pub spinner_frame: usize,
    /// Screen areas of the last frame, for the mouse.
    pub tab_areas: Vec<(Page, Rect)>,
    pub progress_area: Rect,
    last_click: Option<(Instant, u16, u16)>,
    pub cover: Cover,
    media: Option<Media>,
}

impl App {
    pub fn new(
        session: Arc<Session>,
        ui: UiConfig,
        playback: PlaybackConfig,
        log: LogBuffer,
        cover: Cover,
        media: Option<Media>,
    ) -> (Self, UnboundedReceiver<Msg>) {
        let (tx, rx) = unbounded_channel();
        let snap = session.snapshot();
        let browser = Browser::new(ui.browse_mode);
        let app = Self {
            session,
            tx,
            busy: Arc::new(AtomicUsize::new(0)),
            ui,
            playback,
            page: Page::Browser,
            browser,
            queue: QueuePage::default(),
            playlists: PlaylistsPage::default(),
            search: SearchPage::default(),
            log: LogPage::new(log),
            overlay: None,
            status: None,
            snap,
            quit: false,
            redraw_all: false,
            spinner_frame: 0,
            tab_areas: Vec::new(),
            progress_area: Rect::default(),
            last_click: None,
            cover,
            media,
        };
        (app, rx)
    }

    pub fn set_media(&mut self, m: Media) {
        self.media = Some(m);
    }

    /// A sender for messages into the app (media keys use it).
    pub fn sender(&self) -> UnboundedSender<Msg> {
        self.tx.clone()
    }

    /// Initial loads.
    pub fn start(&mut self) {
        self.browser_load_root();
        self.playlists_load();
        if self.playback.save_queue_on_exit && self.snap.queue.is_empty() {
            let s = self.session.clone();
            self.spawn(async move {
                match s.load_play_queue().await {
                    Ok(0) => Msg::Info("Connected".into()),
                    Ok(n) => Msg::Info(format!("Restored {n} songs from the server queue")),
                    Err(e) => Msg::Error(format!("Loading the server queue failed: {e}")),
                }
            });
        }
    }

    /// Number of background requests in flight.
    pub fn busy(&self) -> usize {
        self.busy.load(Ordering::Relaxed)
    }

    /// Run `fut` in the background and feed its result back as a message.
    pub fn spawn<F>(&self, fut: F)
    where
        F: Future<Output = Msg> + Send + 'static,
    {
        let tx = self.tx.clone();
        let busy = self.busy.clone();
        busy.fetch_add(1, Ordering::Relaxed);
        tokio::spawn(async move {
            let m = fut.await;
            busy.fetch_sub(1, Ordering::Relaxed);
            let _ = tx.send(m);
        });
    }

    pub fn info(&mut self, text: impl Into<String>) {
        let text = text.into();
        tracing::info!("{text}");
        self.status = Some(Status {
            text,
            error: false,
            at: Instant::now(),
        });
    }

    pub fn error(&mut self, text: impl Into<String>) {
        let text = text.into();
        tracing::warn!("{text}");
        self.status = Some(Status {
            text,
            error: true,
            at: Instant::now(),
        });
    }

    pub fn send(&self, cmd: Command) {
        self.session.send(cmd);
    }

    // ---------------------------------------------------------------- queueing

    /// Put songs into the queue.
    pub fn enqueue(&mut self, songs: Vec<Child>, how: Enqueue) {
        let songs: Vec<Child> = songs.into_iter().filter(|c| !c.is_dir).collect();
        if songs.is_empty() {
            self.error("Nothing to play");
            return;
        }
        let n = songs.len();
        match how {
            Enqueue::Replace(start) => {
                self.session.play_songs(&songs, start);
                self.info(format!("Playing {n} songs"));
            }
            Enqueue::Append => {
                self.session.enqueue(&songs);
                self.info(format!("Added {n} songs to the queue"));
            }
            Enqueue::Next => {
                self.session.enqueue_next(&songs);
                self.info(format!("{n} songs play next"));
            }
        }
    }

    /// Expand an entry to songs in the background, then queue them.
    pub fn enqueue_entry(&mut self, entry: Entry, how: Enqueue) {
        if let Entry::Song(c) = entry {
            self.enqueue(vec![*c], how);
            return;
        }
        let s = self.session.clone();
        self.info(format!("Loading {}…", entry.name()));
        self.spawn(async move {
            Msg::Songs {
                how,
                result: entry.songs(&s).await,
            }
        });
    }

    /// Star or unstar in the background.
    pub fn toggle_star(&mut self, target: StarTarget, currently: bool) {
        let s = self.session.clone();
        self.spawn(async move {
            match s.set_starred(&target, !currently).await {
                Ok(starred) => Msg::Starred { target, starred },
                Err(e) => Msg::Error(format!("Starring failed: {e}")),
            }
        });
    }

    fn on_starred(&mut self, target: &StarTarget, starred: bool) {
        let ids: Vec<&String> = target
            .ids
            .iter()
            .chain(&target.album_ids)
            .chain(&target.artist_ids)
            .collect();
        for id in &ids {
            self.session.mark_track_starred(id, starred);
        }
        let apply = |e: &mut Entry| {
            if ids.iter().any(|i| *i == e.id()) {
                e.set_starred(starred);
            }
        };
        self.browser.for_each_entry(apply);
        self.search.for_each_entry(apply);
        self.playlists.for_each_entry(apply);
        self.info(if starred { "Starred" } else { "Unstarred" });
    }

    /// Open the playlist picker for these songs.
    pub fn pick_playlist(&mut self, songs: Vec<String>) {
        if songs.is_empty() {
            return;
        }
        let mut items = vec![PickerItem::New];
        items.extend(
            self.playlists
                .lists
                .items()
                .iter()
                .cloned()
                .map(PickerItem::Existing),
        );
        self.overlay = Some(Overlay::PlaylistPicker {
            songs,
            list: ListView::new(items),
        });
    }

    fn add_to_playlist(&mut self, playlist: Playlist, songs: Vec<String>) {
        let s = self.session.clone();
        self.spawn(async move {
            let update = PlaylistUpdate {
                add_song_ids: songs.clone(),
                ..Default::default()
            };
            match s.client().update_playlist(&playlist.id, &update).await {
                Ok(()) => Msg::Info(format!("Added {} songs to {}", songs.len(), playlist.name)),
                Err(e) => Msg::Error(format!("Updating {} failed: {e}", playlist.name)),
            }
        });
        self.playlists.mark_dirty(None);
    }

    // ---------------------------------------------------------------- events

    /// Handle a terminal event.
    pub fn on_terminal(&mut self, ev: TermEvent) {
        match ev {
            TermEvent::Key(k) if k.kind != KeyEventKind::Release => self.on_key(k),
            TermEvent::Mouse(m) if self.ui.mouse => self.on_mouse(m),
            TermEvent::Resize(..) => self.redraw_all = true,
            _ => {}
        }
    }

    pub fn on_key(&mut self, k: KeyEvent) {
        if self.overlay.is_some() {
            self.overlay_key(k);
            return;
        }
        if self.page == Page::Search && self.search.editing {
            self.search_input_key(k);
            return;
        }
        if let Some(list) = self.focused_list()
            && list.filtering()
        {
            use crossterm::event::KeyCode;
            match k.code {
                KeyCode::Esc => list.end_filter(false),
                KeyCode::Enter => list.end_filter(true),
                KeyCode::Backspace => list.filter_pop(),
                KeyCode::Up => list.move_by(-1),
                KeyCode::Down => list.move_by(1),
                KeyCode::Char(c) => list.filter_push(c),
                _ => {}
            }
            return;
        }
        let mut contexts = Vec::with_capacity(3);
        contexts.extend(self.page.context());
        contexts.push(Context::List);
        contexts.push(Context::Global);
        if let Some(a) = keys::lookup(&k, &contexts) {
            self.action(a);
        }
    }

    /// Perform an action from a key binding.
    pub fn action(&mut self, a: Action) {
        let step = f64::from(self.playback.seek_step.max(1));
        match a {
            Action::Quit => self.quit = true,
            Action::Help => self.overlay = Some(Overlay::Help { scroll: 0 }),
            Action::Page(i) => self.goto(Page::ALL[i.min(Page::ALL.len() - 1)]),
            Action::NextPage | Action::PrevPage => {
                let i = Page::ALL.iter().position(|p| *p == self.page).unwrap_or(0);
                let n = Page::ALL.len();
                let j = if a == Action::NextPage {
                    (i + 1) % n
                } else {
                    (i + n - 1) % n
                };
                self.goto(Page::ALL[j]);
            }
            Action::TogglePause => {
                if self.snap.state == PlayState::Stopped && !self.snap.queue.is_empty() {
                    self.send(Command::Play);
                } else {
                    self.send(Command::TogglePause);
                }
            }
            Action::Stop => self.send(Command::Stop),
            Action::Next => self.send(Command::Next),
            Action::Previous => self.send(Command::Previous),
            Action::SeekForward => self.send(Command::SeekBy(step)),
            Action::SeekBack => self.send(Command::SeekBy(-step)),
            Action::VolumeUp => self.send(Command::SetVolume(
                self.snap.volume.saturating_add(5).min(100),
            )),
            Action::VolumeDown => self.send(Command::SetVolume(self.snap.volume.saturating_sub(5))),
            Action::CycleRepeat => self.send(Command::SetRepeat(self.snap.repeat.cycle())),
            Action::AddRandom => {
                let s = self.session.clone();
                let n = self.playback.random_songs.max(1);
                self.spawn(async move {
                    Msg::Songs {
                        how: Enqueue::Append,
                        result: s
                            .client()
                            .get_random_songs(n, None)
                            .await
                            .map_err(|e| e.to_string()),
                    }
                });
            }
            Action::Scan => {
                let s = self.session.clone();
                self.spawn(async move {
                    match s.client().start_scan().await {
                        Ok(_) => Msg::Info("Library scan started".into()),
                        Err(e) => Msg::Error(format!("Scan failed: {e}")),
                    }
                });
            }
            Action::StarPlaying => {
                if let Some(cur) = &self.snap.current {
                    let t = &cur.track;
                    self.toggle_star(StarTarget::id(t.id.clone()), t.starred);
                }
            }
            Action::Redraw => self.redraw_all = true,
            Action::Up => self.with_list(|l| l.move_by(-1)),
            Action::Down => self.with_list(|l| l.move_by(1)),
            Action::PageUp => self.with_list(|l| l.move_by(-l.page())),
            Action::PageDown => self.with_list(|l| l.move_by(l.page())),
            Action::Top => self.with_list(|l| l.top()),
            Action::Bottom => self.with_list(|l| l.bottom()),
            Action::Filter => self.with_list(|l| l.begin_filter()),
            Action::Back => {
                if let Some(l) = self.focused_list()
                    && l.has_filter()
                {
                    l.end_filter(false);
                } else {
                    self.page_action(Action::Back);
                }
            }
            other => self.page_action(other),
        }
    }

    fn with_list(&mut self, f: impl FnOnce(&mut dyn ListLike)) {
        if let Some(l) = self.focused_list() {
            f(l);
        }
    }

    fn page_action(&mut self, a: Action) {
        match self.page {
            Page::Browser => self.browser_action(a),
            Page::Queue => self.queue_action(a),
            Page::Playlists => self.playlists_action(a),
            Page::Search => self.search_action(a),
            Page::Log => {}
        }
    }

    pub fn goto(&mut self, page: Page) {
        self.page = page;
        match page {
            Page::Playlists => self.playlists_ensure_loaded(),
            Page::Search if self.search.query.is_empty() => self.search.editing = true,
            Page::Queue => self.queue_follow(),
            _ => {}
        }
    }

    /// The list that has keyboard focus.
    pub fn focused_list(&mut self) -> Option<&mut dyn ListLike> {
        match self.page {
            Page::Browser => self.browser.focused(),
            Page::Queue => Some(&mut self.queue.list),
            Page::Playlists => Some(self.playlists.focused()),
            Page::Search => Some(self.search.focused()),
            Page::Log => Some(&mut self.log.list),
        }
    }

    fn overlay_key(&mut self, k: KeyEvent) {
        use crossterm::event::KeyCode;
        let Some(overlay) = self.overlay.as_mut() else {
            return;
        };
        match overlay {
            Overlay::Help { scroll } => match k.code {
                KeyCode::Down | KeyCode::Char('j') => *scroll = scroll.saturating_add(1),
                KeyCode::Up | KeyCode::Char('k') => *scroll = scroll.saturating_sub(1),
                KeyCode::PageDown | KeyCode::Char(' ') => *scroll = scroll.saturating_add(10),
                KeyCode::PageUp => *scroll = scroll.saturating_sub(10),
                _ => self.overlay = None,
            },
            Overlay::Prompt { input, .. } => match k.code {
                KeyCode::Esc => self.overlay = None,
                KeyCode::Enter => {
                    if let Some(Overlay::Prompt { input, action, .. }) = self.overlay.take() {
                        self.prompt_done(input.trim().to_string(), action);
                    }
                }
                KeyCode::Backspace => {
                    input.pop();
                }
                KeyCode::Char(c) => input.push(c),
                _ => {}
            },
            Overlay::Confirm { .. } => match k.code {
                KeyCode::Char('y' | 'Y') | KeyCode::Enter => {
                    if let Some(Overlay::Confirm { action, .. }) = self.overlay.take() {
                        self.confirmed(action);
                    }
                }
                _ => self.overlay = None,
            },
            Overlay::PlaylistPicker { list, .. } => match k.code {
                KeyCode::Esc => self.overlay = None,
                KeyCode::Up => list.move_by(-1),
                KeyCode::Down => list.move_by(1),
                KeyCode::Backspace => ListLike::filter_pop(list),
                KeyCode::Enter => {
                    if let Some(Overlay::PlaylistPicker { songs, list }) = self.overlay.take() {
                        match list.selected().cloned() {
                            Some(PickerItem::Existing(p)) => self.add_to_playlist(p, songs),
                            Some(PickerItem::New) => {
                                self.overlay = Some(Overlay::Prompt {
                                    title: "New playlist".into(),
                                    input: String::new(),
                                    action: PromptAction::NewPlaylist { songs },
                                });
                            }
                            None => {}
                        }
                    }
                }
                KeyCode::Char(c) => ListLike::filter_push(list, c),
                _ => {}
            },
        }
    }

    fn prompt_done(&mut self, name: String, action: PromptAction) {
        if name.is_empty() {
            return;
        }
        match action {
            PromptAction::NewPlaylist { songs } => {
                let s = self.session.clone();
                self.spawn(async move {
                    match s.client().create_playlist(&name, &songs).await {
                        Ok(_) => Msg::Info(format!("Created playlist {name}")),
                        Err(e) => Msg::Error(format!("Creating {name} failed: {e}")),
                    }
                });
                self.playlists.mark_dirty(None);
            }
            PromptAction::SaveQueue => {
                let existing = self
                    .playlists
                    .lists
                    .items()
                    .iter()
                    .find(|p| p.name == name)
                    .map(|p| p.id.clone());
                if let Some(id) = existing {
                    self.overlay = Some(Overlay::Confirm {
                        text: format!("Replace the songs of playlist “{name}” with the queue?"),
                        action: ConfirmAction::OverwritePlaylist { id },
                    });
                } else {
                    self.save_queue_as(name, None);
                }
            }
        }
    }

    fn save_queue_as(&mut self, name: String, id: Option<String>) {
        let s = self.session.clone();
        let target = id.clone();
        self.spawn(async move {
            match s.save_queue_as_playlist(&name, id.as_deref()).await {
                Ok(()) => Msg::Info(format!("Saved the queue as {name}")),
                Err(e) => Msg::Error(format!("Saving {name} failed: {e}")),
            }
        });
        self.playlists.mark_dirty(target);
    }

    fn confirmed(&mut self, action: ConfirmAction) {
        match action {
            ConfirmAction::DeletePlaylist { id } => {
                let s = self.session.clone();
                self.spawn(async move {
                    match s.client().delete_playlist(&id).await {
                        Ok(()) => Msg::Info("Playlist deleted".into()),
                        Err(e) => Msg::Error(format!("Deleting failed: {e}")),
                    }
                });
                self.playlists.mark_dirty(None);
            }
            ConfirmAction::OverwritePlaylist { id } => {
                let name = self
                    .playlists
                    .lists
                    .items()
                    .iter()
                    .find(|p| p.id == id)
                    .map(|p| p.name.clone())
                    .unwrap_or_default();
                self.save_queue_as(name, Some(id));
            }
            ConfirmAction::LoadServerQueue => {
                let s = self.session.clone();
                self.spawn(async move {
                    match s.load_play_queue().await {
                        Ok(0) => Msg::Info("The server queue is empty".into()),
                        Ok(n) => Msg::Info(format!("Loaded {n} songs from the server queue")),
                        Err(e) => Msg::Error(format!("Loading the server queue failed: {e}")),
                    }
                });
            }
        }
    }

    fn on_mouse(&mut self, m: crossterm::event::MouseEvent) {
        let (col, row) = (m.column, m.row);
        let inside =
            |r: Rect| col >= r.x && col < r.x + r.width && row >= r.y && row < r.y + r.height;
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if self.overlay.is_some() {
                    self.overlay = None;
                    return;
                }
                if let Some((p, _)) = self.tab_areas.iter().find(|(_, r)| inside(*r)) {
                    let p = *p;
                    self.goto(p);
                    return;
                }
                if inside(self.progress_area)
                    && let Some(d) = self.snap.duration
                {
                    let frac = f64::from(col - self.progress_area.x)
                        / f64::from(self.progress_area.width.max(1));
                    self.send(Command::Seek(frac * d));
                    return;
                }
                let double = self.last_click.is_some_and(|(t, c, r)| {
                    t.elapsed() < Duration::from_millis(400) && c == col && r == row
                });
                self.last_click = Some((Instant::now(), col, row));
                if self.focus_at(col, row) {
                    let hit = self
                        .focused_list()
                        .and_then(|l| l.row_at(row).map(|r| (l, r)));
                    if let Some((l, r)) = hit {
                        l.select_row(r);
                        if double {
                            self.page_action(Action::Activate);
                        }
                    }
                }
            }
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                let d = if m.kind == MouseEventKind::ScrollDown {
                    3
                } else {
                    -3
                };
                if self.overlay.is_none() && self.focus_at(col, row) {
                    self.with_list(|l| l.move_by(d));
                }
            }
            _ => {}
        }
    }

    /// Focus the list under the mouse. Returns false if there is none.
    fn focus_at(&mut self, col: u16, row: u16) -> bool {
        match self.page {
            Page::Browser => self.browser.focus_at(col, row),
            Page::Playlists => self.playlists.focus_at(col, row),
            Page::Search => self.search.focus_at(col, row),
            Page::Queue | Page::Log => true,
        }
    }

    /// Handle a session event.
    pub fn on_session(&mut self, ev: SessionEvent) {
        match &ev {
            SessionEvent::Engine(Event::Error(e)) => self.error(e.clone()),
            SessionEvent::Error(e) => self.error(e.clone()),
            SessionEvent::Scrobbled { id } => tracing::debug!("scrobbled {id}"),
            SessionEvent::Engine(Event::TrackChanged(Some(entry))) => {
                tracing::info!("playing {} - {}", entry.track.artist, entry.track.title);
            }
            SessionEvent::Engine(Event::QueueEnded) => self.info("End of the queue"),
            _ => {}
        }
        self.refresh_snapshot();
        if let SessionEvent::Engine(Event::TrackChanged(_)) = ev {
            self.queue_follow();
            let wanted = self
                .snap
                .current
                .as_ref()
                .and_then(|c| c.track.cover_art.clone());
            if let Some(job) = self.cover.want(wanted) {
                let s = self.session.clone();
                self.spawn(async move { Msg::Cover(job.run(s).await) });
            }
            self.media_track_changed();
        }
        if let SessionEvent::Engine(Event::StateChanged(_) | Event::Position { .. }) = ev
            && let Some(m) = &mut self.media
        {
            m.state_changed(&self.snap);
        }
    }

    fn media_track_changed(&mut self) {
        if let Some(m) = &mut self.media {
            let file = self.cover.file.as_ref().map(|(_, p)| p.as_path());
            m.track_changed(&self.snap, file);
        }
    }

    fn refresh_snapshot(&mut self) {
        self.snap = self.session.snapshot();
        self.queue.sync(&self.snap);
    }

    /// Handle a background result.
    pub fn on_msg(&mut self, msg: Msg) {
        match msg {
            Msg::Info(t) => self.info(t),
            Msg::Error(t) => self.error(t),
            Msg::Songs { how, result } => match result {
                Ok(songs) => self.enqueue(songs, how),
                Err(e) => self.error(format!("Loading songs failed: {e}")),
            },
            Msg::Starred { target, starred } => self.on_starred(&target, starred),
            Msg::PickPlaylist(r) => match r {
                Ok(songs) => {
                    let ids = songs
                        .into_iter()
                        .filter(|c| !c.is_dir)
                        .map(|c| c.id)
                        .collect();
                    self.pick_playlist(ids);
                }
                Err(e) => self.error(format!("Loading songs failed: {e}")),
            },
            Msg::BrowserRoot { token, result } => self.browser_root_loaded(token, result),
            Msg::BrowserDir { id, result } => self.browser_dir_loaded(&id, result),
            Msg::Playlists(r) => self.playlists_loaded(r),
            Msg::PlaylistEntries { id, result } => self.playlist_entries_loaded(&id, result),
            Msg::Search { query, result } => self.search_loaded(query, result),
            Msg::Cover(c) => {
                self.cover.loaded(c);
                self.media_track_changed();
            }
            Msg::Media(a) => match a {
                MediaAction::Command(c) => self.send(c),
                MediaAction::TogglePause => self.action(Action::TogglePause),
                MediaAction::Quit => self.quit = true,
            },
        }
        // Writes to the server change the playlists; refresh once they are done.
        if self.busy() == 0 && self.playlists.dirty {
            self.playlists_reload();
        }
    }

    /// Periodic work (debounced previews, snapshot refresh, spinner).
    pub fn on_tick(&mut self) {
        self.spinner_frame = self.spinner_frame.wrapping_add(1);
        self.refresh_snapshot();
        self.browser_tick();
        self.playlists_tick();
        self.log.sync();
        if self
            .status
            .as_ref()
            .is_some_and(|s| s.at.elapsed() > Duration::from_secs(if s.error { 10 } else { 4 }))
        {
            self.status = None;
        }
    }

    pub fn repeat_label(&self) -> &'static str {
        match self.snap.repeat {
            RepeatMode::Off => "",
            RepeatMode::All => "repeat all",
            RepeatMode::One => "repeat one",
        }
    }
}

/// Drive the app until it quits.
pub async fn run<B: ratatui::backend::Backend>(
    terminal: &mut ratatui::Terminal<B>,
    app: &mut App,
    mut msgs: UnboundedReceiver<Msg>,
    mut session_events: UnboundedReceiver<SessionEvent>,
) -> anyhow::Result<()>
where
    B::Error: Send + Sync + 'static,
{
    let mut term_events = crossterm::event::EventStream::new();
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    app.start();
    while !app.quit {
        if app.redraw_all {
            terminal.clear()?;
            app.redraw_all = false;
        }
        terminal.draw(|f| crate::ui::draw(app, f))?;
        tokio::select! {
            ev = term_events.next() => match ev {
                Some(Ok(ev)) => app.on_terminal(ev),
                Some(Err(e)) => return Err(e.into()),
                None => break,
            },
            Some(ev) = session_events.recv() => app.on_session(ev),
            Some(m) = msgs.recv() => app.on_msg(m),
            _ = tick.tick() => app.on_tick(),
        }
        // Handle everything that is already queued before drawing again.
        while let Ok(ev) = session_events.try_recv() {
            app.on_session(ev);
        }
        while let Ok(m) = msgs.try_recv() {
            app.on_msg(m);
        }
    }
    Ok(())
}
