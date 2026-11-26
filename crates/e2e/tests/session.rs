// SPDX-License-Identifier: GPL-3.0-only
//! Core session (HTTP streaming into the engine) against real servers.

use std::time::{Duration, Instant};

use infrasonic_core::config::PlaybackConfig;
use infrasonic_core::{Session, SessionEvent};
use infrasonic_e2e::{Server, capturing_output, library, servers};
use infrasonic_engine::{Command, Event, PlayState};
use infrasonic_subsonic::Child;
use tokio::sync::mpsc::UnboundedReceiver;

async fn wait_for(
    rx: &mut UnboundedReceiver<SessionEvent>,
    timeout: Duration,
    mut pred: impl FnMut(&SessionEvent) -> bool,
) -> Vec<SessionEvent> {
    let deadline = Instant::now() + timeout;
    let mut seen = Vec::new();
    while Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(50), rx.recv()).await {
            Ok(Some(ev)) => {
                let done = pred(&ev);
                seen.push(ev);
                if done {
                    return seen;
                }
            }
            Ok(None) => break,
            Err(_) => {}
        }
    }
    panic!("timeout; events: {seen:#?}");
}

async fn formats_album_songs(s: &Session) -> Vec<Child> {
    let r = s
        .client()
        .search3("Song", Default::default())
        .await
        .expect("search");
    let mut songs: Vec<Child> = r
        .song
        .into_iter()
        .filter(|c| c.album.as_deref() == Some(library::FORMATS_ALBUM))
        .collect();
    songs.sort_by_key(|c| c.track);
    songs
}

async fn check(server: &Server) {
    let playback = PlaybackConfig {
        save_queue_on_exit: false,
        ..PlaybackConfig::default()
    };
    let (out, capture) = capturing_output(20.0);
    let (session, mut rx) = Session::connect(&server.name, &server.profile(), playback, out)
        .await
        .expect("connect");

    // Play all formats in a row over HTTP.
    let songs = formats_album_songs(&session).await;
    assert_eq!(songs.len(), 4, "{}", server.name);
    session.play_songs(&songs, 0);
    let events = wait_for(&mut rx, Duration::from_secs(30), |e| {
        *e == SessionEvent::Engine(Event::QueueEnded)
    })
    .await;
    let played: Vec<String> = events
        .iter()
        .filter_map(|e| match e {
            SessionEvent::Engine(Event::TrackChanged(Some(t))) => Some(t.track.title.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        played,
        ["MP3 Song", "Vorbis Song", "Opus Song", "AAC Song"],
        "{}",
        server.name
    );
    let errors: Vec<&SessionEvent> = events
        .iter()
        .filter(|e| {
            matches!(
                e,
                SessionEvent::Engine(Event::Error(_)) | SessionEvent::Error(_)
            )
        })
        .collect();
    assert!(errors.is_empty(), "{}: {errors:#?}", server.name);
    let secs = capture.lock().len() as f64 / 2.0 / 44_100.0;
    assert!((secs - 20.0).abs() < 0.5, "{}: played {secs}s", server.name);

    // Recursive folder expansion (STMPS "a" on an artist).
    let idx = session.client().get_indexes(None).await.expect("indexes");
    let quartet = idx
        .index
        .iter()
        .flat_map(|i| &i.artist)
        .find(|a| a.name == "The Codec Quartet")
        .expect("artist");
    let all = session.directory_songs(&quartet.id).await.expect("songs");
    assert_eq!(all.len(), 4, "{}", server.name);

    // Queue sync: save, clear, restore paused at the saved position.
    session.play_songs(&songs, 1);
    wait_for(&mut rx, Duration::from_secs(10), |e| {
        *e == SessionEvent::Engine(Event::StateChanged(PlayState::Playing))
    })
    .await;
    session.send(Command::Seek(2.0));
    session.send(Command::Pause);
    wait_for(&mut rx, Duration::from_secs(10), |e| {
        *e == SessionEvent::Engine(Event::StateChanged(PlayState::Paused))
    })
    .await;
    session.save_play_queue().await.expect("save");
    session.send(Command::Clear);
    wait_for(&mut rx, Duration::from_secs(5), |e| {
        *e == SessionEvent::Engine(Event::StateChanged(PlayState::Stopped))
    })
    .await;
    let n = session.load_play_queue().await.expect("load");
    assert_eq!(n, 4, "{}", server.name);
    wait_for(&mut rx, Duration::from_secs(10), |e| {
        *e == SessionEvent::Engine(Event::StateChanged(PlayState::Paused))
    })
    .await;
    let snap = session.snapshot();
    assert_eq!(snap.current_index, Some(1), "{}", server.name);
    assert!(
        (snap.position - 2.0).abs() < 0.3,
        "{}: position {}",
        server.name,
        snap.position
    );

    // Saving the queue as a playlist.
    let name = format!("e2e-queue-{}", std::process::id());
    session
        .save_queue_as_playlist(&name, None)
        .await
        .expect("save as playlist");
    let pl = session
        .client()
        .get_playlists()
        .await
        .expect("playlists")
        .into_iter()
        .find(|p| p.name == name)
        .expect("playlist");
    assert_eq!(pl.song_count, Some(4), "{}", server.name);
    session
        .client()
        .delete_playlist(&pl.id)
        .await
        .expect("delete");

    session.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "multi_thread")]
async fn session_plays_from_server() {
    for s in servers() {
        check(&s).await;
    }
}
