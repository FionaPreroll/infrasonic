// SPDX-License-Identifier: GPL-3.0-only
//! The Kotlin interface against Navidrome and gonic (`scripts/e2e-servers.sh`). The calls are
//! the ones the app makes; without servers the tests return early.

use std::sync::Arc;
use std::time::{Duration, Instant};

use infrasonic_e2e::{library, servers};
use infrasonic_ffi::*;
use parking_lot::Mutex;

#[derive(Default)]
struct Recorder(Mutex<Vec<PlayerEvent>>);

impl PlayerListener for Recorder {
    fn on_event(&self, event: PlayerEvent) {
        self.0.lock().push(event);
    }
}

impl Recorder {
    fn wait(&self, what: &str, timeout: Duration, f: impl Fn(&[PlayerEvent]) -> bool) {
        let end = Instant::now() + timeout;
        while Instant::now() < end {
            if f(&self.0.lock()) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("timed out waiting for {what}: {:?}", self.0.lock().last());
    }
}

fn profile(url: &str, auth: ServerAuth) -> ServerProfile {
    ServerProfile {
        id: "test".into(),
        name: "test".into(),
        url: url.into(),
        auth,
        allow_self_signed: false,
    }
}

fn app(dir: &tempfile::TempDir, speed: f64) -> Arc<Infrasonic> {
    Infrasonic::with_null_output(
        dir.path().join("data").to_string_lossy().into(),
        dir.path().join("cache").to_string_lossy().into(),
        speed,
    )
    .expect("app")
}

/// Run an async call the way Kotlin does: from a thread outside the runtime.
/// A minimal executor: the futures only wait for runtime tasks.
fn block<T>(f: impl Future<Output = T>) -> T {
    use std::pin::pin;
    use std::task::{Context, Poll, Wake, Waker};
    struct ThreadWaker(std::thread::Thread);
    impl Wake for ThreadWaker {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(ThreadWaker(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut f = pin!(f);
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
        std::thread::park_timeout(Duration::from_millis(50));
    }
}

#[test]
fn errors_without_server() {
    let dir = tempfile::tempdir().expect("tmp");
    let a = app(&dir, 1.0);
    assert_eq!(block(a.artists()), Err(FfiError::NotConnected));
    assert!(a.state().is_none());
    let e = block(a.test_connection(profile(
        "http://127.0.0.1:9",
        token_auth("u".into(), "p".into()),
    )))
    .expect_err("no server");
    assert!(matches!(e, FfiError::Network { .. }), "{e:?}");
}

#[test]
fn profiles_and_settings_are_saved() {
    let dir = tempfile::tempdir().expect("tmp");
    let a = app(&dir, 1.0);
    assert!(a.profiles().is_empty() && a.active_profile_id().is_none());
    let p = profile(
        "https://music.example.org",
        token_auth("me".into(), "pw".into()),
    );
    let k = ServerProfile {
        id: "k".into(),
        auth: ServerAuth::ApiKey { key: "abc".into() },
        allow_self_signed: true,
        ..p.clone()
    };
    a.save_profiles(vec![p.clone(), k.clone()], Some("k".into()))
        .expect("save");
    let mut st = a.settings();
    st.replay_gain = ReplayGainMode::Album;
    st.pin_quality = Some(Quality {
        max_bit_rate: 192,
        format: Some("opus".into()),
    });
    a.set_settings(st.clone());
    drop(a);

    let b = app(&dir, 1.0);
    assert_eq!(b.profiles(), vec![p, k]);
    assert_eq!(b.active_profile_id().as_deref(), Some("k"));
    assert_eq!(b.settings(), st);
    let text = std::fs::read_to_string(dir.path().join("data/profiles.json")).expect("file");
    assert!(!text.contains("\"pw\""), "password stored: {text}");
}

#[test]
fn library_and_playback() {
    for s in servers() {
        let dir = tempfile::tempdir().expect("tmp");
        let a = app(&dir, 20.0);
        let rec = Arc::new(Recorder::default());
        a.set_listener(Some(rec.clone()));

        let bad =
            block(a.test_connection(profile(&s.url, token_auth("admin".into(), "wrong".into()))))
                .expect_err("wrong password");
        assert!(matches!(bad, FfiError::Auth { .. }), "{}: {bad:?}", s.name);

        let p = profile(&s.url, token_auth("admin".into(), "admin".into()));
        let status = block(a.connect(p.clone())).expect("connect");
        assert!(status.online && !status.version.is_empty(), "{status:?}");
        assert_eq!(a.connected_profile().as_deref(), Some("test"));

        let groups = block(a.artists()).expect("artists");
        let names: Vec<_> = groups
            .iter()
            .flat_map(|g| &g.artists)
            .map(|a| a.name.clone())
            .collect();
        for n in library::ARTISTS {
            assert!(names.iter().any(|x| x == n), "{}: {names:?}", s.name);
        }
        let albums = block(a.album_list(AlbumListKind::ByName, 50, 0)).expect("albums");
        let sine = albums
            .iter()
            .find(|x| x.name == library::SINE_ALBUM)
            .expect("sine album");
        let album = block(a.album(sine.id.clone())).expect("album");
        assert!(album.songs.len() >= 2, "{}: {album:?}", s.name);

        let found = block(a.search("song".into(), 20, 0)).expect("search");
        assert!(!found.songs.is_empty(), "{}", s.name);
        let folder = block(a.folder(None)).expect("folders");
        assert!(!folder.folders.is_empty(), "{}", s.name);
        let sub = block(a.folder(Some(folder.folders[0].id.clone()))).expect("folder");
        assert!(
            !sub.folders.is_empty() || !sub.songs.is_empty(),
            "{}",
            s.name
        );

        // Star a song and see it among the starred ones.
        let song = album.songs[0].clone();
        block(a.set_starred(ItemKind::Song, song.id.clone(), true)).expect("star");
        let starred = block(a.starred()).expect("starred");
        assert!(starred.songs.iter().any(|x| x.id == song.id), "{}", s.name);
        block(a.set_starred(ItemKind::Song, song.id.clone(), false)).expect("unstar");

        // Play the album to the end at 20× speed.
        a.play(album.songs.clone(), 0).expect("play");
        rec.wait("queue end", Duration::from_secs(60), |ev| {
            ev.contains(&PlayerEvent::QueueEnded)
        });
        let titles: Vec<String> = rec
            .0
            .lock()
            .iter()
            .filter_map(|e| match e {
                PlayerEvent::TrackChanged { item: Some(i) } => Some(i.song.title.clone()),
                _ => None,
            })
            .collect();
        let want: Vec<String> = album.songs.iter().map(|x| x.title.clone()).collect();
        assert_eq!(titles, want, "{}", s.name);
        let errors: Vec<_> = rec
            .0
            .lock()
            .iter()
            .filter(|e| matches!(e, PlayerEvent::Error { .. }))
            .cloned()
            .collect();
        assert!(errors.is_empty(), "{}: {errors:?}", s.name);
        assert_eq!(a.queue().len(), album.songs.len());

        // Queue commands.
        a.command(PlayerCommand::PlayIndex { index: 1 })
            .expect("cmd");
        rec.wait("playing", Duration::from_secs(10), |_| {
            a.state()
                .is_some_and(|st| st.state == PlayState::Playing && st.index == Some(1))
        });
        a.command(PlayerCommand::Pause).expect("pause");
        rec.wait("paused", Duration::from_secs(10), |_| {
            a.state().is_some_and(|st| st.state == PlayState::Paused)
        });

        // Server play queue round trip.
        block(a.save_server_queue()).expect("save queue");
        a.command(PlayerCommand::Clear).expect("clear");
        let n = block(a.load_server_queue()).expect("load queue");
        assert_eq!(n as usize, album.songs.len(), "{}", s.name);

        // Local queue for resuming playback.
        rec.wait("restored queue", Duration::from_secs(10), |_| {
            a.queue().len() == n as usize
        });
        a.save_local_queue().expect("save local queue");
        let q = a.local_queue().expect("local queue");
        assert_eq!(q.songs.len(), album.songs.len());
        assert_eq!(q.songs[0].id, album.songs[0].id);

        // Bookmarks.
        block(a.create_bookmark(song.id.clone(), 1500, Some("here".into()))).expect("bookmark");
        let marks = block(a.bookmarks()).expect("bookmarks");
        assert!(
            marks
                .iter()
                .any(|b| b.song.id == song.id && b.position_ms == 1500),
            "{}: {marks:?}",
            s.name
        );
        block(a.delete_bookmark(song.id.clone())).expect("delete bookmark");

        // Cover art comes through the client.
        if let Some(cover) = &album.songs[0].cover_art {
            let _ = block(a.cover_art(cover.clone(), Some(64)));
        }
        a.disconnect();
        assert!(a.state().is_none());
    }
}

#[test]
fn pinned_songs_play_offline() {
    for s in servers() {
        let dir = tempfile::tempdir().expect("tmp");
        let a = app(&dir, 20.0);
        let rec = Arc::new(Recorder::default());
        a.set_listener(Some(rec.clone()));
        let good = profile(&s.url, token_auth("admin".into(), "admin".into()));
        block(a.connect(good.clone())).expect("connect");
        let albums = block(a.album_list(AlbumListKind::ByName, 50, 0)).expect("albums");
        let sine = albums
            .iter()
            .find(|x| x.name == library::SINE_ALBUM)
            .expect("sine album");
        let songs = block(a.album(sine.id.clone())).expect("album").songs;

        let failed = block(a.pin_and_wait(songs.clone())).expect("pin");
        assert_eq!(failed, 0, "{}", s.name);
        assert_eq!(a.pinned().len(), songs.len());
        assert!(a.is_pinned(songs[0].id.clone()));
        let usage = a.cache_usage();
        assert_eq!(usage.pinned_files as usize, songs.len(), "{usage:?}");
        assert!(usage.pinned_bytes > 0);

        // A new app instance with a server that cannot be reached: pinned songs still play.
        a.disconnect();
        drop(a);
        let b = app(&dir, 20.0);
        let rec = Arc::new(Recorder::default());
        b.set_listener(Some(rec.clone()));
        let unreachable = ServerProfile {
            url: s.url.clone(),
            ..good.clone()
        };
        let status = b.connect_offline(unreachable).expect("offline");
        assert!(!status.online);
        let pinned = b.pinned();
        assert_eq!(pinned.len(), songs.len(), "{}", s.name);
        b.play(pinned, 0).expect("play");
        rec.wait("queue end", Duration::from_secs(60), |ev| {
            ev.contains(&PlayerEvent::QueueEnded)
        });
        b.unpin(vec![songs[0].id.clone()]).expect("unpin");
        assert_eq!(b.pinned().len(), songs.len() - 1);
    }
}

#[test]
fn podcasts_jukebox_and_settings() {
    for s in servers() {
        let dir = tempfile::tempdir().expect("tmp");
        let a = app(&dir, 20.0);
        block(a.connect(profile(&s.url, token_auth("admin".into(), "admin".into()))))
            .expect("connect");
        // gonic has podcasts (none in the test library), Navidrome answers 501.
        match block(a.podcasts(true)) {
            Ok(c) => assert!(c.is_empty(), "{}: {c:?}", s.name),
            Err(FfiError::NotSupported { reason }) => {
                assert!(!reason.contains("t="), "login data in {reason}")
            }
            Err(e) => panic!("{}: {e:?}", s.name),
        }
        // Jukebox needs an audio device on the server: an error is fine, a crash is not.
        let _ = block(a.jukebox(JukeboxCommand::Get));

        let mut st = a.settings();
        st.quality = Quality {
            max_bit_rate: 128,
            format: Some("mp3".into()),
        };
        st.replay_gain = ReplayGainMode::Track;
        st.cache_mb = 10;
        a.set_settings(st.clone());
        assert_eq!(a.settings(), st);

        let genres = block(a.genres()).expect("genres");
        if let Some(g) = genres.first() {
            block(a.songs_by_genre(g.name.clone(), 10, 0)).expect("genre songs");
        }
        let random = block(a.random_songs(3, None)).expect("random");
        assert_eq!(random.len(), 3, "{}", s.name);
        let lyrics = block(a.lyrics(random[0].clone()));
        assert!(lyrics.is_ok(), "{}: {lyrics:?}", s.name);
    }
}

#[test]
fn equalizer_settings() {
    let dir = tempfile::tempdir().expect("tmp");
    let a = app(&dir, 1.0);
    assert_eq!(equalizer_bands().len(), 10);
    assert_eq!(equalizer_max_gain_db(), 12.0);
    assert_eq!(a.equalizer(), None);
    let eq = EqualizerSettings {
        preamp_db: -2.0,
        gains_db: vec![6.0, 3.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 30.0],
    };
    // Works before a connection: the next session starts with it.
    a.set_equalizer(Some(eq.clone()));
    assert_eq!(a.equalizer(), Some(eq.clone()));
    assert_eq!(equalizer_headroom_db(eq), -14.0, "16 kHz clamped to +12 dB");
    a.set_equalizer(None);
    assert_eq!(a.equalizer(), None);
}
