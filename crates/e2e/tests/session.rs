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

/// Streamed songs land in the cache; pinned songs play without the server.
async fn check_cache(server: &Server) {
    use std::sync::Arc;

    use infrasonic_core::cache::{AudioCache, server_key};
    let dir = tempfile::tempdir().expect("tempdir");
    let key = server_key(&server.url, "admin");
    let make_cache = || {
        Arc::new(AudioCache::new(
            &dir.path().join("cache"),
            &dir.path().join("data"),
            &key,
            64 << 20,
        ))
    };
    let playback = PlaybackConfig {
        save_queue_on_exit: false,
        scrobble: false,
        ..PlaybackConfig::default()
    };
    let (out, _) = capturing_output(20.0);
    let (session, mut rx) =
        Session::connect(&server.name, &server.profile(), playback.clone(), out)
            .await
            .expect("connect");
    session.set_cache(Some(make_cache()));
    let songs = formats_album_songs(&session).await;
    session.play_songs(&songs, 0);
    wait_for(&mut rx, Duration::from_secs(30), |e| {
        *e == SessionEvent::Engine(Event::QueueEnded)
    })
    .await;
    let cache = session.cache().expect("cache");
    // Downloads finish a moment after decoding.
    let deadline = Instant::now() + Duration::from_secs(10);
    while cache.usage().files < 4 && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(cache.usage().files, 4, "{}: cached songs", server.name);
    for s in &songs {
        assert!(
            cache.lookup(&s.id, "raw").is_some(),
            "{}: {}",
            server.name,
            s.title
        );
    }

    // Pin two songs.
    session.pin(songs[..2].to_vec());
    let mut pinned = 0;
    wait_for(&mut rx, Duration::from_secs(20), |e| {
        if matches!(e, SessionEvent::Pinned { .. }) {
            pinned += 1;
        }
        pinned == 2
    })
    .await;
    assert_eq!(cache.pinned().len(), 2);
    session.shutdown().await.expect("shutdown");

    // Offline: an unreachable server, but the pinned songs still play.
    let mut profile = server.profile();
    profile.url = "http://127.0.0.1:9".into();
    let (out, capture) = capturing_output(20.0);
    let (offline, mut rx) =
        Session::offline(&server.name, &profile, playback, out).expect("offline session");
    assert!(!offline.is_online());
    // Same cache directories, as the next start would use them.
    offline.set_cache(Some(make_cache()));
    let pins = offline.cache().expect("cache").pinned();
    assert_eq!(pins.len(), 2);
    offline.play_songs(&pins, 0);
    let events = wait_for(&mut rx, Duration::from_secs(30), |e| {
        *e == SessionEvent::Engine(Event::QueueEnded)
    })
    .await;
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, SessionEvent::Engine(Event::Error(_)))),
        "{}: {events:#?}",
        server.name
    );
    let secs = capture.lock().len() as f64 / 2.0 / 44_100.0;
    assert!(
        (secs - 10.0).abs() < 0.5,
        "{}: played {secs}s offline",
        server.name
    );
    offline.shutdown().await.expect("shutdown");
}

/// While the first song plays, the following ones are downloaded in advance, but never more
/// than the playing and the next song at once.
async fn check_prefetch(server: &Server) {
    use std::sync::Arc;

    use infrasonic_core::cache::{AudioCache, server_key};
    let dir = tempfile::tempdir().expect("tempdir");
    let cache = Arc::new(AudioCache::new(
        &dir.path().join("cache"),
        &dir.path().join("data"),
        &server_key(&server.url, "admin"),
        64 << 20,
    ));
    let playback = PlaybackConfig {
        save_queue_on_exit: false,
        scrobble: false,
        preload_songs: 2,
        ..PlaybackConfig::default()
    };
    let (out, _) = capturing_output(1.0);
    let (session, mut rx) = Session::connect(&server.name, &server.profile(), playback, out)
        .await
        .expect("connect");
    session.set_cache(Some(cache.clone()));
    let songs = formats_album_songs(&session).await;
    session.play_songs(&songs, 0);
    wait_for(&mut rx, Duration::from_secs(10), |e| {
        *e == SessionEvent::Engine(Event::StateChanged(PlayState::Playing))
    })
    .await;
    session.send(Command::Pause);
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut most = 0;
    while songs.iter().any(|s| !cache.contains(&s.id, "raw")) && Instant::now() < deadline {
        most = most.max(cache.downloads());
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    for s in &songs {
        assert!(
            cache.contains(&s.id, "raw"),
            "{}: {} not cached",
            server.name,
            s.title
        );
    }
    assert!(most <= 2, "{}: {most} downloads at once", server.name);
    session.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "multi_thread")]
async fn session_prefetches_one_at_a_time() {
    for s in servers() {
        check_prefetch(&s).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn session_caches_and_pins() {
    for s in servers() {
        check_cache(&s).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn session_plays_from_server() {
    for s in servers() {
        check(&s).await;
    }
}
