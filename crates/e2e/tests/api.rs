// SPDX-License-Identifier: GPL-3.0-only
//! The Subsonic client against Navidrome and gonic.

use infrasonic_e2e::{Server, library, servers};
use infrasonic_engine::decode::TrackDecoder;
use infrasonic_engine::source::StreamReader;
use infrasonic_subsonic::{
    AlbumListType, Auth, Child, Client, PlaylistUpdate, SearchOptions, StarTarget, StreamOptions,
};

async fn all_songs(c: &Client) -> Vec<Child> {
    let mut songs = c.get_random_songs(100, None).await.expect("random songs");
    songs.sort_by(|a, b| a.title.cmp(&b.title));
    songs
}

/// All songs of albums called `name`. Servers group albums differently (Navidrome may split
/// an album when tags differ between formats), so this merges same-named albums.
async fn album(c: &Client, name: &str) -> infrasonic_subsonic::Album {
    let artists = c.get_artists().await.expect("artists");
    let mut merged: Option<infrasonic_subsonic::Album> = None;
    for idx in artists.index {
        for a in idx.artist {
            let full = c.get_artist(&a.id).await.expect("artist");
            for al in full.album {
                if al.name.contains(name) {
                    let got = c.get_album(&al.id).await.expect("album");
                    match &mut merged {
                        Some(m) => m.song.extend(got.song),
                        None => merged = Some(got),
                    }
                }
            }
        }
    }
    merged.unwrap_or_else(|| panic!("album {name} not found"))
}

async fn check_server(s: &Server) {
    let c = s.client();
    let info = c.connect().await.expect("connect");
    assert_eq!(info.server_type.as_deref(), Some(s.name.as_str()));
    assert!(c.supports("formPost"), "{}: formPost", s.name);

    // Wrong password is an auth error.
    let bad = Client::new(
        &s.url,
        Auth::Token {
            username: "admin".into(),
            password: "nope".into(),
        },
    )
    .expect("client");
    assert!(bad.ping().await.expect_err("bad login").is_auth_error());

    // Folder browsing.
    let idx = c.get_indexes(None).await.expect("indexes");
    let mut names: Vec<String> = idx
        .index
        .iter()
        .flat_map(|i| i.artist.iter().map(|a| a.name.clone()))
        .collect();
    names.sort();
    assert_eq!(names, library::ARTISTS, "{}", s.name);
    let quartet = idx
        .index
        .iter()
        .flat_map(|i| &i.artist)
        .find(|a| a.name == "The Codec Quartet")
        .expect("artist");
    let dir = c.get_music_directory(&quartet.id).await.expect("dir");
    let mut n = 0;
    for album_dir in dir.child.iter().filter(|c| c.is_dir) {
        let songs = c.get_music_directory(&album_dir.id).await.expect("album");
        n += songs.child.iter().filter(|c| !c.is_dir).count();
    }
    assert_eq!(n, 4, "{}", s.name);

    // ID3 browsing.
    let formats = album(&c, library::FORMATS_ALBUM).await;
    let mut suffixes: Vec<String> = formats
        .song
        .iter()
        .filter_map(|s| s.suffix.clone())
        .collect();
    suffixes.sort();
    assert_eq!(
        suffixes,
        ["m4a", "mp3", "ogg", "opus"],
        "{}: {:#?}",
        s.name,
        formats
            .song
            .iter()
            .map(|x| (&x.title, &x.suffix))
            .collect::<Vec<_>>()
    );

    // Search.
    let r = c
        .search3("black", SearchOptions::default())
        .await
        .expect("search");
    assert!(
        r.artist.iter().any(|a| a.name == "Black Noise Collective"),
        "{}",
        s.name
    );
    assert!(r.song.iter().any(|x| x.title == "Black Hole"), "{}", s.name);

    // Random songs and album lists.
    let songs = all_songs(&c).await;
    assert_eq!(songs.len(), library::SONGS, "{}", s.name);
    let newest = c
        .get_album_list2(AlbumListType::AlphabeticalByName, 10, 0)
        .await
        .expect("album list");
    // Navidrome files the AAC song under its own album because MP4 tags differ slightly.
    assert!(
        (3..=4).contains(&newest.len()),
        "{}: {} albums",
        s.name,
        newest.len()
    );

    // Album lists by year and genre.
    let years = c
        .get_album_list2(
            AlbumListType::ByYear {
                from: 2024,
                to: 2025,
            },
            10,
            0,
        )
        .await
        .expect("by year");
    let mut names: Vec<&str> = years.iter().map(|a| a.name.as_str()).collect();
    names.sort();
    names.dedup();
    assert_eq!(names, ["Formats Live", "Sine Studies"], "{}", s.name);
    let genres = c.get_genres().await.expect("genres");
    assert!(
        genres.iter().any(|g| g.value == "Jazz"),
        "{}: {genres:?}",
        s.name
    );
    let jazz = c
        .get_albums_by_genre("Jazz", 10, 0)
        .await
        .expect("by genre");
    assert!(
        !jazz.is_empty() && jazz.iter().all(|a| a.name == "Formats Live"),
        "{}: {jazz:?}",
        s.name
    );

    // Stars.
    let song = &songs[0];
    c.star(&StarTarget::id(&song.id)).await.expect("star");
    let starred = c.get_starred2().await.expect("starred");
    assert!(starred.song.iter().any(|x| x.id == song.id), "{}", s.name);
    c.unstar(&StarTarget::id(&song.id)).await.expect("unstar");
    let starred = c.get_starred2().await.expect("starred");
    assert!(!starred.song.iter().any(|x| x.id == song.id), "{}", s.name);

    // Playlists.
    let name = format!("e2e-{}", std::process::id());
    c.create_playlist(&name, &[songs[0].id.clone(), songs[1].id.clone()])
        .await
        .expect("create");
    let pl = c
        .get_playlists()
        .await
        .expect("playlists")
        .into_iter()
        .find(|p| p.name == name)
        .expect("created playlist listed");
    assert_eq!(c.get_playlist(&pl.id).await.expect("get").entry.len(), 2);
    c.update_playlist(
        &pl.id,
        &PlaylistUpdate {
            add_song_ids: vec![songs[2].id.clone()],
            remove_indexes: vec![0],
            ..Default::default()
        },
    )
    .await
    .expect("update");
    let entries = c.get_playlist(&pl.id).await.expect("get").entry;
    assert_eq!(
        entries.iter().map(|e| e.id.clone()).collect::<Vec<_>>(),
        [songs[1].id.clone(), songs[2].id.clone()],
        "{}",
        s.name
    );
    let all: Vec<String> = songs.iter().map(|x| x.id.clone()).collect();
    c.replace_playlist(&pl.id, &all).await.expect("replace");
    assert_eq!(
        c.get_playlist(&pl.id).await.expect("get").entry.len(),
        all.len()
    );
    c.delete_playlist(&pl.id).await.expect("delete");
    assert!(
        !c.get_playlists()
            .await
            .expect("playlists")
            .iter()
            .any(|p| p.id == pl.id)
    );

    // Play queue.
    let ids: Vec<String> = songs[..3].iter().map(|x| x.id.clone()).collect();
    c.save_play_queue(&ids, Some(&ids[1]), Some(1500))
        .await
        .expect("save queue");
    let pq = c
        .get_play_queue()
        .await
        .expect("load queue")
        .expect("queue");
    assert_eq!(
        pq.entry.iter().map(|e| e.id.clone()).collect::<Vec<_>>(),
        ids,
        "{}",
        s.name
    );
    assert_eq!(pq.current.as_deref(), Some(ids[1].as_str()));
    assert_eq!(pq.position, Some(1500), "{}", s.name);

    // Ratings.
    c.set_rating(&song.id, 4).await.expect("rate");
    assert_eq!(
        c.get_song(&song.id).await.expect("song").user_rating,
        Some(4),
        "{}",
        s.name
    );
    c.set_rating(&song.id, 0).await.expect("unrate");
    assert!(
        c.get_song(&song.id)
            .await
            .expect("song")
            .user_rating
            .is_none_or(|r| r == 0),
        "{}",
        s.name
    );

    // Bookmarks.
    c.create_bookmark(&song.id, 12_345, Some("e2e"))
        .await
        .expect("bookmark");
    let marks = c.get_bookmarks().await.expect("bookmarks");
    let m = marks
        .iter()
        .find(|b| b.entry.id == song.id)
        .unwrap_or_else(|| panic!("{}: bookmark missing: {marks:?}", s.name));
    assert_eq!(m.position, 12_345, "{}", s.name);
    assert_eq!(m.entry.title, song.title, "{}", s.name);
    c.delete_bookmark(&song.id).await.expect("delete bookmark");
    assert!(
        !c.get_bookmarks()
            .await
            .expect("bookmarks")
            .iter()
            .any(|b| b.entry.id == song.id),
        "{}",
        s.name
    );

    // Artist info answers (external services are off, so it is mostly empty).
    c.get_artist_info2(&quartet.id, 5).await.ok();

    // Scrobbling and scans.
    c.scrobble(&song.id, false, None)
        .await
        .expect("now playing");
    c.scrobble(&song.id, true, Some(1_700_000_000_000))
        .await
        .expect("submission");
    c.start_scan().await.expect("scan");

    // Lyrics endpoint answers (the test files have none).
    if c.supports("songLyrics") {
        let l = c.get_lyrics_by_song_id(&song.id).await.expect("lyrics");
        assert!(l.iter().all(|x| x.line.is_empty()), "{}", s.name);
    }

    // Every format streams and decodes.
    for song in &formats.song {
        let url = c.stream_url(&song.id, &StreamOptions::default());
        let bytes = c
            .http()
            .get(url)
            .send()
            .await
            .expect("stream")
            .bytes()
            .await
            .expect("body")
            .to_vec();
        let mut dec = TrackDecoder::open(
            Box::new(StreamReader::from_bytes(bytes)),
            song.suffix.as_deref(),
        )
        .unwrap_or_else(|e| panic!("{} {}: {e}", s.name, song.title));
        let mut frames = 0usize;
        while let Some(ch) = dec.next_chunk().expect("decode") {
            frames += ch.samples.len() / ch.channels;
        }
        let secs = frames as f64 / f64::from(dec.sample_rate());
        assert!(
            (secs - 5.0).abs() < 0.2,
            "{} {}: {secs}s",
            s.name,
            song.title
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn subsonic_api() {
    for s in servers() {
        check_server(&s).await;
    }
}

async fn check_radio_and_shares(s: &Server) {
    let c = s.client();
    // A salted token (what the web player stores) logs in like the password.
    let salted = Client::new(&s.url, Auth::salted("admin", "admin")).expect("client");
    salted.ping().await.expect("salted token login");

    let name = format!("e2e radio {}", s.name);
    c.create_internet_radio_station(&name, "https://radio.example.org/stream", None)
        .await
        .expect("create station");
    let stations = c.get_internet_radio_stations().await.expect("stations");
    let st = stations
        .iter()
        .find(|x| x.name == name)
        .unwrap_or_else(|| panic!("{}: station missing in {stations:?}", s.name));
    assert_eq!(st.stream_url, "https://radio.example.org/stream");
    c.delete_internet_radio_station(&st.id)
        .await
        .expect("delete station");
    let after = c.get_internet_radio_stations().await.expect("stations");
    assert!(after.iter().all(|x| x.name != name), "{}", s.name);

    // Shares: Navidrome (with sharing enabled); gonic has none and says so.
    let al = album(&c, "Sine Studies").await;
    match c.create_share(&[&al.id], Some("e2e"), None).await {
        Ok(share) => {
            assert!(share.url.starts_with("http"), "{}: {share:?}", s.name);
            let shares = c.get_shares().await.expect("shares");
            assert!(shares.iter().any(|x| x.id == share.id), "{}", s.name);
            c.delete_share(&share.id).await.expect("delete share");
            let after = c.get_shares().await.expect("shares");
            assert!(after.iter().all(|x| x.id != share.id), "{}", s.name);
        }
        Err(e) => assert_eq!(s.name, "gonic", "{}: {e}", s.name),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn radio_and_shares() {
    for s in servers() {
        check_radio_and_shares(&s).await;
    }
}
