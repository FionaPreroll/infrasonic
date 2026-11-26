// SPDX-License-Identifier: GPL-3.0-only
//! Drives the TUI headless (ratatui `TestBackend`) against Navidrome and gonic.
//! Runs only with `scripts/e2e-servers.sh`, see `crates/e2e`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use infrasonic::app::{App, Msg, Page};
use infrasonic::cover::Cover;
use infrasonic::logging::LogBuffer;
use infrasonic_core::config::{PlaybackConfig, UiConfig};
use infrasonic_core::{Session, SessionEvent};
use infrasonic_e2e::{Server, null_output, servers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use tokio::sync::mpsc::UnboundedReceiver;

struct Harness {
    app: App,
    msgs: UnboundedReceiver<Msg>,
    events: UnboundedReceiver<SessionEvent>,
    term: Terminal<TestBackend>,
    name: String,
}

impl Harness {
    async fn new(server: &Server) -> Self {
        let playback = PlaybackConfig {
            save_queue_on_exit: false,
            scrobble: false,
            ..PlaybackConfig::default()
        };
        let (session, events) = Session::connect(
            &server.name,
            &server.profile(),
            playback.clone(),
            null_output(10.0),
        )
        .await
        .expect("connect");
        let (mut app, msgs) = App::new(
            Arc::new(session),
            UiConfig::default(),
            playback,
            LogBuffer::default(),
            Cover::disabled(),
            None,
        );
        app.start();
        infrasonic::i18n::set_language(infrasonic::i18n::Lang::En);
        let term = Terminal::new(TestBackend::new(130, 40)).expect("terminal");
        Self {
            app,
            msgs,
            events,
            term,
            name: server.name.clone(),
        }
    }

    /// Process messages and ticks until nothing happens for a while.
    async fn settle(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut quiet_since = Instant::now();
        while Instant::now() < deadline {
            tokio::select! {
                Some(m) = self.msgs.recv() => { self.app.on_msg(m); quiet_since = Instant::now(); }
                Some(e) = self.events.recv() => self.app.on_session(e),
                _ = tokio::time::sleep(Duration::from_millis(50)) => self.app.on_tick(),
            }
            if self.app.busy() == 0 && quiet_since.elapsed() > Duration::from_millis(400) {
                return;
            }
        }
        panic!(
            "{}: app did not settle; busy={}",
            self.name,
            self.app.busy()
        );
    }

    /// Process events until the screen contains `text`.
    async fn wait_for(&mut self, text: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let s = self.screen();
            if s.contains(text) {
                return s;
            }
            assert!(
                Instant::now() < deadline,
                "{}: `{text}` not on screen:\n{s}",
                self.name
            );
            tokio::select! {
                Some(m) = self.msgs.recv() => self.app.on_msg(m),
                Some(e) = self.events.recv() => self.app.on_session(e),
                _ = tokio::time::sleep(Duration::from_millis(50)) => self.app.on_tick(),
            }
        }
    }

    fn key(&mut self, code: KeyCode) {
        self.app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn chars(&mut self, s: &str) {
        for c in s.chars() {
            self.key(KeyCode::Char(c));
        }
    }

    fn screen(&mut self) -> String {
        let app = &mut self.app;
        self.term
            .draw(|f| infrasonic::ui::draw(app, f))
            .expect("draw");
        let buf = self.term.backend().buffer();
        let w = buf.area.width as usize;
        buf.content
            .chunks(w)
            .map(|row| row.iter().map(|c| c.symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

async fn check(server: &Server) {
    let mut h = Harness::new(server).await;
    h.settle().await;
    h.wait_for("The Codec Quartet").await;
    h.wait_for("Black Noise Collective").await;

    // Browser: filter to the artist, open its album folder and play from the first song.
    h.key(KeyCode::Char('/'));
    h.chars("codec");
    h.key(KeyCode::Enter);
    h.wait_for("Formats Live").await;
    h.key(KeyCode::Char('l'));
    h.key(KeyCode::Enter);
    h.wait_for("MP3 Song").await;
    h.key(KeyCode::Enter);
    let s = h.wait_for("▶ MP3 Song").await;
    assert!(s.contains("The Codec Quartet"), "{}:\n{s}", h.name);

    // Queue: the album folder (Navidrome files the AAC song under its own album, so three or
    // four songs); remove the last one.
    h.key(KeyCode::Char('2'));
    let n = h.app.snap.queue.len();
    assert!((3..=4).contains(&n), "{}: {n} songs", h.name);
    let s = h.wait_for(&format!("Queue · {n} songs")).await;
    assert!(s.contains("Opus Song"), "{}:\n{s}", h.name);
    h.key(KeyCode::Char('G'));
    h.key(KeyCode::Char('d'));
    let s = h.wait_for(&format!("Queue · {} songs", n - 1)).await;
    assert!(!s.contains("Opus Song") || n == 4, "{}:\n{s}", h.name);

    // Save the queue as a playlist, see it on the playlists page, delete it again.
    let name = format!("tui-{}-{}", h.name, std::process::id());
    h.key(KeyCode::Char('s'));
    h.wait_for("Save queue as playlist").await;
    h.chars(&name);
    h.key(KeyCode::Enter);
    h.settle().await;
    h.key(KeyCode::Char('3'));
    h.settle().await;
    h.wait_for(&name).await;
    h.key(KeyCode::Char('/'));
    h.chars(&name);
    h.key(KeyCode::Enter);
    h.settle().await;
    let s = h.wait_for("MP3 Song").await;
    assert!(s.contains("Vorbis Song"), "{}:\n{s}", h.name);
    h.key(KeyCode::Char('d'));
    h.wait_for("Delete playlist").await;
    h.key(KeyCode::Char('y'));
    h.settle().await;
    h.key(KeyCode::Esc);
    let s = h.screen();
    assert!(!s.contains(&name), "{}: playlist not deleted:\n{s}", h.name);

    // Search, then star and unstar a song.
    h.key(KeyCode::Char('4'));
    assert!(h.app.search.editing);
    h.chars("black");
    h.key(KeyCode::Enter);
    h.settle().await;
    let s = h.wait_for("Black Hole").await;
    assert!(s.contains("Black Noise Collective"), "{}:\n{s}", h.name);
    h.app.search.focus = 2;
    h.app.search.columns[2].select_where(|e| e.name() == "Black Hole");
    let id = h.app.search.columns[2]
        .selected()
        .expect("song")
        .id()
        .to_string();
    h.key(KeyCode::Char('y'));
    h.settle().await;
    let starred = h
        .app
        .session
        .client()
        .get_starred2()
        .await
        .expect("starred");
    assert!(starred.song.iter().any(|s| s.id == id), "{}", h.name);
    assert!(h.app.search.columns[2].selected().expect("song").starred());
    h.wait_for("Black Hole · Black Noise Collective · 0:03 ★")
        .await;
    h.key(KeyCode::Char('y'));
    h.settle().await;
    let starred = h
        .app
        .session
        .client()
        .get_starred2()
        .await
        .expect("starred");
    assert!(!starred.song.iter().any(|s| s.id == id), "{}", h.name);

    // Rate the song, see the rating, remove it again.
    h.key(KeyCode::Char('*'));
    h.wait_for("Rate Black Hole").await;
    h.key(KeyCode::Char('4'));
    h.settle().await;
    h.wait_for("Black Hole · Black Noise Collective · 0:03 4/5")
        .await;
    let c = h.app.session.client();
    assert_eq!(c.get_song(&id).await.expect("song").user_rating, Some(4));
    h.key(KeyCode::Char('*'));
    h.key(KeyCode::Char('0'));
    h.settle().await;
    let c = h.app.session.client();
    let r = c.get_song(&id).await.expect("song").user_rating;
    assert!(r.is_none_or(|r| r == 0), "{}: {r:?}", h.name);

    // Command palette: an action by its description, and a command with an argument.
    h.key(KeyCode::Char(':'));
    h.wait_for(" Command ").await;
    h.chars("volume 40");
    h.key(KeyCode::Enter);
    h.wait_for("vol 40%").await;
    h.key(KeyCode::Char(':'));
    h.chars("redraw");
    h.wait_for("Redraw the screen").await;
    h.key(KeyCode::Enter);
    assert!(h.app.overlay.is_none());
    h.key(KeyCode::Char(':'));
    h.chars("language de");
    h.key(KeyCode::Enter);
    h.wait_for("2 Warteschlange").await;
    h.key(KeyCode::Char(':'));
    h.chars("language en");
    h.key(KeyCode::Enter);
    h.wait_for("2 Queue").await;

    // Library: the newest albums, then the songs of one.
    h.key(KeyCode::Char('6'));
    assert_eq!(h.app.page, Page::Library);
    h.settle().await;
    h.wait_for("Recently added").await;
    h.wait_for("Formats Live").await;

    // Lyrics of the playing song (the test files have none).
    h.key(KeyCode::Char('7'));
    assert_eq!(h.app.page, Page::Lyrics);
    h.settle().await;
    h.wait_for("No lyrics for this song").await;

    // Help and quit.
    h.key(KeyCode::Char('?'));
    h.wait_for(" Keys ").await;
    h.key(KeyCode::Esc);
    h.key(KeyCode::Char('5'));
    assert_eq!(h.app.page, Page::Log);
    h.key(KeyCode::Char('q'));
    assert!(h.app.quit);
    h.app.session.finish().await.expect("finish");
}

#[tokio::test(flavor = "multi_thread")]
async fn tui_against_servers() {
    for s in servers() {
        check(&s).await;
    }
}
