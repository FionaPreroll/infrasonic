// SPDX-License-Identifier: GPL-3.0-only
//! HTTP level tests against a mock server.

use infrasonic_subsonic::{Auth, Client, Error, PlaylistUpdate, StarTarget};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn ok(payload: &str) -> ResponseTemplate {
    let body = format!(
        r#"{{"subsonic-response":{{"status":"ok","version":"1.16.1","type":"mock","serverVersion":"1.0","openSubsonic":true{payload}}}}}"#
    );
    ResponseTemplate::new(200).set_body_raw(body, "application/json")
}

fn client(server: &MockServer) -> Client {
    Client::new(
        &server.uri(),
        Auth::Token {
            username: "alice".into(),
            password: "pw".into(),
        },
    )
    .expect("client")
}

#[tokio::test]
async fn connect_detects_extensions_and_sends_auth() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/ping"))
        .and(query_param("u", "alice"))
        .and(query_param("v", "1.16.1"))
        .and(query_param("c", "infrasonic"))
        .and(query_param("f", "json"))
        .respond_with(ok(""))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/rest/getOpenSubsonicExtensions"))
        .respond_with(ok(
            r#","openSubsonicExtensions":[{"name":"formPost","versions":[1]},{"name":"songLyrics","versions":[1,2]}]"#,
        ))
        .mount(&server)
        .await;

    let c = client(&server);
    let info = c.connect().await.expect("connect");
    assert_eq!(info.server_type.as_deref(), Some("mock"));
    assert!(info.open_subsonic);
    assert!(c.supports("formPost"));
    assert!(c.supports("songLyrics"));
    assert!(!c.supports("transcodeOffset"));
}

#[tokio::test]
async fn wrong_password_is_auth_error() {
    let server = MockServer::start().await;
    Mock::given(path("/rest/ping"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            r#"{"subsonic-response":{"status":"failed","version":"1.16.1","error":{"code":40,"message":"Wrong username or password"}}}"#,
            "application/json",
        ))
        .mount(&server)
        .await;
    let err = client(&server).ping().await.expect_err("must fail");
    assert!(err.is_auth_error());
}

#[tokio::test]
async fn http_errors_are_reported() {
    let server = MockServer::start().await;
    Mock::given(path("/rest/ping"))
        .respond_with(ResponseTemplate::new(502))
        .mount(&server)
        .await;
    let err = client(&server).ping().await.expect_err("must fail");
    assert!(matches!(err, Error::Http(_)), "{err}");
}

#[tokio::test]
async fn long_parameter_lists_use_form_post() {
    let server = MockServer::start().await;
    Mock::given(path("/rest/ping"))
        .respond_with(ok(""))
        .mount(&server)
        .await;
    Mock::given(path("/rest/getOpenSubsonicExtensions"))
        .respond_with(ok(
            r#","openSubsonicExtensions":[{"name":"formPost","versions":[1]}]"#,
        ))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/rest/savePlayQueue"))
        .respond_with(ok(""))
        .expect(1)
        .mount(&server)
        .await;
    let c = client(&server);
    c.connect().await.expect("connect");
    let ids: Vec<String> = (0..500).map(|i| format!("song-{i}")).collect();
    c.save_play_queue(&ids, Some("song-3"), Some(12_000))
        .await
        .expect("save");
}

#[tokio::test]
async fn empty_queue_is_not_sent() {
    let server = MockServer::start().await;
    Mock::given(path("/rest/savePlayQueue"))
        .respond_with(ok(""))
        .expect(0)
        .mount(&server)
        .await;
    client(&server)
        .save_play_queue(&[], None, None)
        .await
        .expect("noop");
}

#[tokio::test]
async fn playlist_and_star_calls() {
    let server = MockServer::start().await;
    Mock::given(path("/rest/updatePlaylist"))
        .and(query_param("playlistId", "p1"))
        .and(query_param("songIdToAdd", "s1"))
        .and(query_param("songIndexToRemove", "3"))
        .respond_with(ok(""))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path("/rest/star"))
        .and(query_param("albumId", "a1"))
        .respond_with(ok(""))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path("/rest/getPlayQueue"))
        .respond_with(ok(""))
        .mount(&server)
        .await;
    let c = client(&server);
    c.update_playlist(
        "p1",
        &PlaylistUpdate {
            add_song_ids: vec!["s1".into()],
            remove_indexes: vec![3],
            ..Default::default()
        },
    )
    .await
    .expect("update");
    c.star(&StarTarget::album("a1")).await.expect("star");
    assert_eq!(c.get_play_queue().await.expect("queue"), None);
}
