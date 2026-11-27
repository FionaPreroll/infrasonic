// SPDX-License-Identifier: GPL-3.0-only
//! Typed async client for the [Subsonic](http://www.subsonic.org/pages/api.jsp) and
//! [OpenSubsonic](https://opensubsonic.netlify.app/) API.
//!
//! The client is runtime agnostic apart from `reqwest`: on native targets the caller must run it
//! inside a Tokio runtime, on `wasm32` it uses the browser's `fetch`.
//!
//! ```no_run
//! # async fn demo() -> infrasonic_subsonic::Result<()> {
//! use infrasonic_subsonic::{Auth, Client};
//! let client = Client::new(
//!     "https://music.example.com",
//!     Auth::Token { username: "me".into(), password: "secret".into() },
//! )?;
//! let info = client.connect().await?;
//! println!("connected to {:?} {}", info.server_type, info.version);
//! # Ok(()) }
//! ```

mod auth;
mod de;
mod error;
pub mod model;

use std::sync::{Arc, RwLock};

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use url::Url;

pub use auth::{Auth, token};
pub use error::{Error, Result};
pub use model::*;

/// API version this client speaks.
pub const API_VERSION: &str = "1.16.1";
/// Default client name sent as `c` parameter.
pub const DEFAULT_CLIENT_NAME: &str = "infrasonic";

/// Options for `search3`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchOptions {
    pub artist_count: u32,
    pub artist_offset: u32,
    pub album_count: u32,
    pub album_offset: u32,
    pub song_count: u32,
    pub song_offset: u32,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            artist_count: 20,
            artist_offset: 0,
            album_count: 20,
            album_offset: 0,
            song_count: 20,
            song_offset: 0,
        }
    }
}

/// Options for stream URLs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StreamOptions {
    /// Maximum bit rate in kbit/s; `None` or `Some(0)` means unlimited.
    pub max_bit_rate: Option<u32>,
    /// Target format, e.g. `"raw"`, `"mp3"`, `"opus"`.
    pub format: Option<String>,
    /// Start offset in seconds (only meaningful for transcoded streams).
    pub time_offset: Option<u32>,
    /// Ask the server to estimate `Content-Length` for transcoded streams.
    pub estimate_content_length: bool,
}

/// What to star or unstar.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StarTarget {
    pub ids: Vec<Id>,
    pub album_ids: Vec<Id>,
    pub artist_ids: Vec<Id>,
}

impl StarTarget {
    /// Target a single song or directory.
    pub fn id(id: impl Into<Id>) -> Self {
        Self {
            ids: vec![id.into()],
            ..Self::default()
        }
    }
    /// Target a single album (ID3).
    pub fn album(id: impl Into<Id>) -> Self {
        Self {
            album_ids: vec![id.into()],
            ..Self::default()
        }
    }
    /// Target a single artist (ID3).
    pub fn artist(id: impl Into<Id>) -> Self {
        Self {
            artist_ids: vec![id.into()],
            ..Self::default()
        }
    }
}

/// Changes for `updatePlaylist`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlaylistUpdate {
    pub name: Option<String>,
    pub comment: Option<String>,
    pub public: Option<bool>,
    pub add_song_ids: Vec<Id>,
    /// Zero based indexes of songs to remove.
    pub remove_indexes: Vec<usize>,
}

/// Builder for [`Client`].
#[derive(Debug)]
pub struct ClientBuilder {
    base_url: String,
    auth: Auth,
    client_name: String,
    http: Option<reqwest::Client>,
}

impl ClientBuilder {
    /// Set the `c` parameter sent with every request.
    pub fn client_name(mut self, name: impl Into<String>) -> Self {
        self.client_name = name.into();
        self
    }

    /// Use a preconfigured `reqwest::Client` (proxy, certificates, ...).
    pub fn http_client(mut self, http: reqwest::Client) -> Self {
        self.http = Some(http);
        self
    }

    /// Build the client.
    pub fn build(self) -> Result<Client> {
        let mut base = Url::parse(self.base_url.trim())?;
        // Make relative joins append to the path instead of replacing the last segment.
        if !base.path().ends_with('/') {
            let p = format!("{}/", base.path());
            base.set_path(&p);
        }
        base.set_query(None);
        base.set_fragment(None);
        let _ = base.set_username("");
        let _ = base.set_password(None);
        let http = match self.http {
            Some(h) => h,
            None => default_http_client()?,
        };
        Ok(Client {
            inner: Arc::new(Inner {
                http,
                base,
                auth: self.auth,
                client_name: self.client_name,
                extensions: RwLock::new(Vec::new()),
            }),
        })
    }
}

/// Install the `ring` TLS provider for rustls if the process has none yet.
pub fn ensure_crypto_provider() {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
}

fn default_http_client() -> Result<reqwest::Client> {
    ensure_crypto_provider();
    let b = reqwest::Client::builder();
    #[cfg(not(target_arch = "wasm32"))]
    let b = b
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(60))
        .user_agent(concat!("infrasonic/", env!("CARGO_PKG_VERSION")));
    #[cfg(target_os = "android")]
    let b = android_roots(b);
    Ok(b.build()?)
}

/// The platform verifier needs a Java VM on Android, which a plain executable (run from
/// `adb shell`) does not have. Use the system CA store from the file system instead.
#[cfg(target_os = "android")]
fn android_roots(b: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
    let mut certs = Vec::new();
    for dir in [
        "/apex/com.android.conscrypt/cacerts",
        "/system/etc/security/cacerts",
    ] {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for e in entries.flatten() {
            if let Ok(pem) = std::fs::read(e.path())
                && let Ok(c) = reqwest::Certificate::from_pem(&pem)
            {
                certs.push(c);
            }
        }
        if !certs.is_empty() {
            break;
        }
    }
    tracing::debug!("{} CA certificates from the system store", certs.len());
    if certs.is_empty() {
        b
    } else {
        b.tls_certs_only(certs)
    }
}

#[derive(Debug)]
struct Inner {
    http: reqwest::Client,
    base: Url,
    auth: Auth,
    client_name: String,
    extensions: RwLock<Vec<Extension>>,
}

/// A Subsonic API client. Cheap to clone.
#[derive(Debug, Clone)]
pub struct Client {
    inner: Arc<Inner>,
}

/// Parse a complete Subsonic JSON response body.
///
/// `path` names the nested payload, e.g. `["playlists", "playlist"]`. When `required` is false a
/// missing payload yields `T::default()` (servers omit empty lists).
pub fn parse_response<T: DeserializeOwned + Default>(
    endpoint: &str,
    body: &[u8],
    path: &[&str],
    required: bool,
) -> Result<(ServerInfo, T)> {
    let decode = |source| Error::Decode {
        endpoint: endpoint.to_string(),
        source,
    };
    let mut root: Map<String, Value> = serde_json::from_slice(body).map_err(decode)?;
    let Some(Value::Object(mut resp)) = root.remove("subsonic-response") else {
        return Err(Error::MissingPayload {
            endpoint: endpoint.into(),
            key: "subsonic-response".into(),
        });
    };
    if resp.get("status").and_then(Value::as_str) != Some("ok") {
        let err = resp.remove("error").unwrap_or(Value::Null);
        let code = err
            .get("code")
            .and_then(Value::as_i64)
            .and_then(|c| i32::try_from(c).ok())
            .unwrap_or(0);
        let message = err
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown error")
            .to_string();
        return Err(Error::Api { code, message });
    }
    let info = ServerInfo {
        version: resp
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        server_type: resp.get("type").and_then(Value::as_str).map(str::to_string),
        server_version: resp
            .get("serverVersion")
            .and_then(Value::as_str)
            .map(str::to_string),
        open_subsonic: resp
            .get("openSubsonic")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    };
    let mut cur = Value::Object(resp);
    for key in path {
        match cur {
            Value::Object(mut m) => match m.remove(*key) {
                Some(v) => cur = v,
                None if required => {
                    return Err(Error::MissingPayload {
                        endpoint: endpoint.into(),
                        key: (*key).into(),
                    });
                }
                None => return Ok((info, T::default())),
            },
            // Some servers send `""` or `[]` instead of an empty object.
            _ if !required => return Ok((info, T::default())),
            _ => {
                return Err(Error::MissingPayload {
                    endpoint: endpoint.into(),
                    key: (*key).into(),
                });
            }
        }
    }
    if path.is_empty() || (cur.is_null() && !required) {
        // gonic sends `null` for empty lists.
        return Ok((info, T::default()));
    }
    let value = serde_json::from_value(cur).map_err(decode)?;
    Ok((info, value))
}

type Params = Vec<(&'static str, String)>;

impl Client {
    /// Create a client with default settings.
    pub fn new(base_url: &str, auth: Auth) -> Result<Self> {
        Self::builder(base_url, auth).build()
    }

    /// Start building a client.
    pub fn builder(base_url: &str, auth: Auth) -> ClientBuilder {
        ClientBuilder {
            base_url: base_url.to_string(),
            auth,
            client_name: DEFAULT_CLIENT_NAME.to_string(),
            http: None,
        }
    }

    /// The server base URL (always ends with `/`).
    pub fn base_url(&self) -> &Url {
        &self.inner.base
    }

    /// The configured authentication.
    pub fn auth(&self) -> &Auth {
        &self.inner.auth
    }

    /// The underlying HTTP client, e.g. to download streams.
    pub fn http(&self) -> &reqwest::Client {
        &self.inner.http
    }

    /// Whether the server announced the given OpenSubsonic extension (after [`Client::connect`]).
    pub fn supports(&self, extension: &str) -> bool {
        self.inner
            .extensions
            .read()
            .map(|e| e.iter().any(|x| x.name == extension))
            .unwrap_or(false)
    }

    /// Extensions announced by the server (after [`Client::connect`]).
    pub fn known_extensions(&self) -> Vec<Extension> {
        self.inner
            .extensions
            .read()
            .map(|e| e.clone())
            .unwrap_or_default()
    }

    fn url(&self, endpoint: &str, params: &[(&str, String)]) -> Url {
        let mut url = self
            .inner
            .base
            .join(&format!("rest/{endpoint}"))
            .unwrap_or_else(|_| self.inner.base.clone());
        {
            let mut q = url.query_pairs_mut();
            for (k, v) in self.common_params() {
                q.append_pair(k, &v);
            }
            for (k, v) in params {
                q.append_pair(k, v);
            }
        }
        url
    }

    fn common_params(&self) -> Params {
        let mut p = self.inner.auth.params();
        p.push(("v", API_VERSION.to_string()));
        p.push(("c", self.inner.client_name.clone()));
        p.push(("f", "json".to_string()));
        p
    }

    async fn fetch(&self, endpoint: &str, params: &[(&str, String)]) -> Result<Vec<u8>> {
        // Long parameter lists (playlists, play queue) go into a form body when supported.
        let long = params.len() > 40 || params.iter().map(|(_, v)| v.len()).sum::<usize>() > 1500;
        let resp = if long && self.supports("formPost") {
            let url = self
                .inner
                .base
                .join(&format!("rest/{endpoint}"))
                .unwrap_or_else(|_| self.inner.base.clone());
            let mut form: Vec<(&str, String)> = self.common_params();
            form.extend(params.iter().map(|(k, v)| (*k, v.clone())));
            self.inner.http.post(url).form(&form).send().await?
        } else {
            self.inner
                .http
                .get(self.url(endpoint, params))
                .send()
                .await?
        };
        let resp = resp.error_for_status()?;
        Ok(resp.bytes().await?.to_vec())
    }

    async fn call<T: DeserializeOwned + Default>(
        &self,
        endpoint: &str,
        params: &[(&str, String)],
        path: &[&str],
        required: bool,
    ) -> Result<T> {
        tracing::debug!(endpoint, "subsonic request");
        let body = self.fetch(endpoint, params).await?;
        parse_response(endpoint, &body, path, required).map(|(_, v)| v)
    }

    async fn call_unit(&self, endpoint: &str, params: &[(&str, String)]) -> Result<()> {
        self.call::<()>(endpoint, params, &[], false).await
    }

    /// `ping`: check credentials and return server information.
    pub async fn ping(&self) -> Result<ServerInfo> {
        let body = self.fetch("ping", &[]).await?;
        parse_response::<()>("ping", &body, &[], false).map(|(i, _)| i)
    }

    /// `getOpenSubsonicExtensions`. Returns an empty list on plain Subsonic servers.
    pub async fn get_open_subsonic_extensions(&self) -> Result<Vec<Extension>> {
        match self
            .call(
                "getOpenSubsonicExtensions",
                &[],
                &["openSubsonicExtensions"],
                false,
            )
            .await
        {
            Ok(v) => Ok(v),
            Err(Error::Api { .. }) | Err(Error::Http(_)) => Ok(Vec::new()),
            Err(e) => Err(e),
        }
    }

    /// Ping the server and detect OpenSubsonic extensions.
    pub async fn connect(&self) -> Result<ServerInfo> {
        let info = self.ping().await?;
        let ext = if info.open_subsonic {
            self.get_open_subsonic_extensions().await?
        } else {
            Vec::new()
        };
        if let Ok(mut e) = self.inner.extensions.write() {
            *e = ext;
        }
        Ok(info)
    }

    /// `getIndexes`: the folder based artist index.
    pub async fn get_indexes(&self, music_folder_id: Option<&str>) -> Result<Indexes> {
        let mut p: Params = Vec::new();
        if let Some(f) = music_folder_id {
            p.push(("musicFolderId", f.to_string()));
        }
        self.call("getIndexes", &p, &["indexes"], false).await
    }

    /// `getMusicDirectory`.
    pub async fn get_music_directory(&self, id: &str) -> Result<Directory> {
        self.call(
            "getMusicDirectory",
            &[("id", id.into())],
            &["directory"],
            true,
        )
        .await
    }

    /// `getArtists`: the ID3 based artist index.
    pub async fn get_artists(&self) -> Result<Artists> {
        self.call("getArtists", &[], &["artists"], false).await
    }

    /// `getArtist`: an artist with its albums.
    pub async fn get_artist(&self, id: &str) -> Result<Artist> {
        self.call("getArtist", &[("id", id.into())], &["artist"], true)
            .await
    }

    /// `getAlbum`: an album with its songs.
    pub async fn get_album(&self, id: &str) -> Result<Album> {
        self.call("getAlbum", &[("id", id.into())], &["album"], true)
            .await
    }

    /// `getSong`.
    pub async fn get_song(&self, id: &str) -> Result<Child> {
        self.call("getSong", &[("id", id.into())], &["song"], true)
            .await
    }

    /// `search3`.
    pub async fn search3(&self, query: &str, opts: SearchOptions) -> Result<SearchResult3> {
        let p: Params = vec![
            ("query", query.to_string()),
            ("artistCount", opts.artist_count.to_string()),
            ("artistOffset", opts.artist_offset.to_string()),
            ("albumCount", opts.album_count.to_string()),
            ("albumOffset", opts.album_offset.to_string()),
            ("songCount", opts.song_count.to_string()),
            ("songOffset", opts.song_offset.to_string()),
        ];
        self.call("search3", &p, &["searchResult3"], false).await
    }

    /// `getPlaylists`.
    pub async fn get_playlists(&self) -> Result<Vec<Playlist>> {
        self.call("getPlaylists", &[], &["playlists", "playlist"], false)
            .await
    }

    /// `getPlaylist`: a playlist with its entries.
    pub async fn get_playlist(&self, id: &str) -> Result<Playlist> {
        self.call("getPlaylist", &[("id", id.into())], &["playlist"], true)
            .await
    }

    /// `createPlaylist` with a name. Returns the playlist if the server sends it back (1.14+).
    pub async fn create_playlist(&self, name: &str, song_ids: &[Id]) -> Result<Option<Playlist>> {
        let mut p: Params = vec![("name", name.to_string())];
        p.extend(song_ids.iter().map(|id| ("songId", id.clone())));
        let pl: Option<Playlist> = self
            .call("createPlaylist", &p, &["playlist"], false)
            .await?;
        Ok(pl)
    }

    /// `createPlaylist` with `playlistId`: replace all songs of an existing playlist.
    pub async fn replace_playlist(&self, id: &str, song_ids: &[Id]) -> Result<()> {
        let mut p: Params = vec![("playlistId", id.to_string())];
        p.extend(song_ids.iter().map(|s| ("songId", s.clone())));
        self.call_unit("createPlaylist", &p).await
    }

    /// `updatePlaylist`.
    pub async fn update_playlist(&self, id: &str, update: &PlaylistUpdate) -> Result<()> {
        let mut p: Params = vec![("playlistId", id.to_string())];
        if let Some(n) = &update.name {
            p.push(("name", n.clone()));
        }
        if let Some(c) = &update.comment {
            p.push(("comment", c.clone()));
        }
        if let Some(pb) = update.public {
            p.push(("public", pb.to_string()));
        }
        p.extend(
            update
                .add_song_ids
                .iter()
                .map(|s| ("songIdToAdd", s.clone())),
        );
        p.extend(
            update
                .remove_indexes
                .iter()
                .map(|i| ("songIndexToRemove", i.to_string())),
        );
        self.call_unit("updatePlaylist", &p).await
    }

    /// `deletePlaylist`.
    pub async fn delete_playlist(&self, id: &str) -> Result<()> {
        self.call_unit("deletePlaylist", &[("id", id.into())]).await
    }

    /// `getStarred2`.
    pub async fn get_starred2(&self) -> Result<Starred2> {
        self.call("getStarred2", &[], &["starred2"], false).await
    }

    fn star_params(t: &StarTarget) -> Params {
        let mut p: Params = Vec::new();
        p.extend(t.ids.iter().map(|i| ("id", i.clone())));
        p.extend(t.album_ids.iter().map(|i| ("albumId", i.clone())));
        p.extend(t.artist_ids.iter().map(|i| ("artistId", i.clone())));
        p
    }

    /// `star`.
    pub async fn star(&self, target: &StarTarget) -> Result<()> {
        self.call_unit("star", &Self::star_params(target)).await
    }

    /// `unstar`.
    pub async fn unstar(&self, target: &StarTarget) -> Result<()> {
        self.call_unit("unstar", &Self::star_params(target)).await
    }

    /// `setRating` (0 removes the rating).
    pub async fn set_rating(&self, id: &str, rating: u8) -> Result<()> {
        self.call_unit(
            "setRating",
            &[("id", id.into()), ("rating", rating.min(5).to_string())],
        )
        .await
    }

    /// `getRandomSongs`.
    pub async fn get_random_songs(&self, size: u32, genre: Option<&str>) -> Result<Vec<Child>> {
        let mut p: Params = vec![("size", size.to_string())];
        if let Some(g) = genre {
            p.push(("genre", g.to_string()));
        }
        self.call("getRandomSongs", &p, &["randomSongs", "song"], false)
            .await
    }

    /// `getSimilarSongs` (works with song, album and artist ids).
    pub async fn get_similar_songs(&self, id: &str, count: u32) -> Result<Vec<Child>> {
        self.call(
            "getSimilarSongs",
            &[("id", id.into()), ("count", count.to_string())],
            &["similarSongs", "song"],
            false,
        )
        .await
    }

    /// `getSimilarSongs2` (artist ids, ID3).
    pub async fn get_similar_songs2(&self, artist_id: &str, count: u32) -> Result<Vec<Child>> {
        self.call(
            "getSimilarSongs2",
            &[("id", artist_id.into()), ("count", count.to_string())],
            &["similarSongs2", "song"],
            false,
        )
        .await
    }

    /// `getTopSongs` by artist name.
    pub async fn get_top_songs(&self, artist: &str, count: u32) -> Result<Vec<Child>> {
        self.call(
            "getTopSongs",
            &[("artist", artist.into()), ("count", count.to_string())],
            &["topSongs", "song"],
            false,
        )
        .await
    }

    /// `scrobble`. `submission = false` sends a "now playing" notification.
    pub async fn scrobble(&self, id: &str, submission: bool, time_ms: Option<u64>) -> Result<()> {
        let mut p: Params = vec![("id", id.into()), ("submission", submission.to_string())];
        if let Some(t) = time_ms {
            p.push(("time", t.to_string()));
        }
        self.call_unit("scrobble", &p).await
    }

    /// `getPlayQueue`. `None` if the user has no saved queue.
    pub async fn get_play_queue(&self) -> Result<Option<PlayQueue>> {
        match self
            .call::<Option<PlayQueue>>("getPlayQueue", &[], &["playQueue"], false)
            .await
        {
            Err(e) if e.is_not_found() => Ok(None),
            other => other,
        }
    }

    /// `savePlayQueue`. Servers reject empty queues, so this is a no-op for `ids.is_empty()`.
    pub async fn save_play_queue(
        &self,
        ids: &[Id],
        current: Option<&str>,
        position_ms: Option<u64>,
    ) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let mut p: Params = ids.iter().map(|i| ("id", i.clone())).collect();
        if let Some(c) = current {
            p.push(("current", c.to_string()));
        }
        if let Some(pos) = position_ms {
            p.push(("position", pos.to_string()));
        }
        self.call_unit("savePlayQueue", &p).await
    }

    /// `startScan`.
    pub async fn start_scan(&self) -> Result<ScanStatus> {
        self.call("startScan", &[], &["scanStatus"], false).await
    }

    /// `getScanStatus`.
    pub async fn get_scan_status(&self) -> Result<ScanStatus> {
        self.call("getScanStatus", &[], &["scanStatus"], false)
            .await
    }

    /// `getAlbumList2`.
    pub async fn get_album_list2(
        &self,
        list: AlbumListType,
        size: u32,
        offset: u32,
    ) -> Result<Vec<Album>> {
        let mut p: Params = vec![
            ("type", list.as_str().into()),
            ("size", size.to_string()),
            ("offset", offset.to_string()),
        ];
        if let AlbumListType::ByYear { from, to } = list {
            p.push(("fromYear", from.to_string()));
            p.push(("toYear", to.to_string()));
        }
        self.call("getAlbumList2", &p, &["albumList2", "album"], false)
            .await
    }

    /// `getAlbumList2` with `type=byGenre`.
    pub async fn get_albums_by_genre(
        &self,
        genre: &str,
        size: u32,
        offset: u32,
    ) -> Result<Vec<Album>> {
        self.call(
            "getAlbumList2",
            &[
                ("type", "byGenre".into()),
                ("genre", genre.into()),
                ("size", size.to_string()),
                ("offset", offset.to_string()),
            ],
            &["albumList2", "album"],
            false,
        )
        .await
    }

    /// `getBookmarks`.
    pub async fn get_bookmarks(&self) -> Result<Vec<Bookmark>> {
        self.call("getBookmarks", &[], &["bookmarks", "bookmark"], false)
            .await
    }

    /// `createBookmark`: save `position_ms` in song `id` (replaces an existing bookmark).
    pub async fn create_bookmark(
        &self,
        id: &str,
        position_ms: u64,
        comment: Option<&str>,
    ) -> Result<()> {
        let mut p: Params = vec![("id", id.into()), ("position", position_ms.to_string())];
        if let Some(c) = comment {
            p.push(("comment", c.into()));
        }
        self.call_unit("createBookmark", &p).await
    }

    /// `deleteBookmark`.
    pub async fn delete_bookmark(&self, id: &str) -> Result<()> {
        self.call_unit("deleteBookmark", &[("id", id.into())]).await
    }

    /// `getInternetRadioStations`.
    pub async fn get_internet_radio_stations(&self) -> Result<Vec<InternetRadioStation>> {
        self.call(
            "getInternetRadioStations",
            &[],
            &["internetRadioStations", "internetRadioStation"],
            false,
        )
        .await
    }

    /// `createInternetRadioStation` (admins only on most servers).
    pub async fn create_internet_radio_station(
        &self,
        name: &str,
        stream_url: &str,
        home_page_url: Option<&str>,
    ) -> Result<()> {
        let mut p: Params = vec![("name", name.into()), ("streamUrl", stream_url.into())];
        if let Some(h) = home_page_url {
            p.push(("homepageUrl", h.into()));
        }
        self.call_unit("createInternetRadioStation", &p).await
    }

    /// `deleteInternetRadioStation`.
    pub async fn delete_internet_radio_station(&self, id: &str) -> Result<()> {
        self.call_unit("deleteInternetRadioStation", &[("id", id.into())])
            .await
    }

    /// `getShares`.
    pub async fn get_shares(&self) -> Result<Vec<Share>> {
        self.call("getShares", &[], &["shares", "share"], false)
            .await
    }

    /// `createShare` for songs, albums or playlists. `expires_ms` is a Unix time in ms.
    pub async fn create_share(
        &self,
        ids: &[&str],
        description: Option<&str>,
        expires_ms: Option<u64>,
    ) -> Result<Share> {
        let mut p: Params = ids.iter().map(|i| ("id", (*i).to_string())).collect();
        if let Some(d) = description {
            p.push(("description", d.into()));
        }
        if let Some(e) = expires_ms {
            p.push(("expires", e.to_string()));
        }
        let shares: Vec<Share> = self
            .call("createShare", &p, &["shares", "share"], true)
            .await?;
        shares
            .into_iter()
            .next()
            .ok_or_else(|| Error::MissingPayload {
                endpoint: "createShare".into(),
                key: "share".into(),
            })
    }

    /// `deleteShare`.
    pub async fn delete_share(&self, id: &str) -> Result<()> {
        self.call_unit("deleteShare", &[("id", id.into())]).await
    }

    /// `getArtistInfo2`.
    pub async fn get_artist_info2(&self, id: &str, count: u32) -> Result<ArtistInfo> {
        self.call(
            "getArtistInfo2",
            &[("id", id.into()), ("count", count.to_string())],
            &["artistInfo2"],
            false,
        )
        .await
    }

    /// `getLyrics` (plain text, looked up by artist and title).
    pub async fn get_lyrics(&self, artist: &str, title: &str) -> Result<Lyrics> {
        self.call(
            "getLyrics",
            &[("artist", artist.into()), ("title", title.into())],
            &["lyrics"],
            false,
        )
        .await
    }

    /// `getGenres`.
    pub async fn get_genres(&self) -> Result<Vec<Genre>> {
        self.call("getGenres", &[], &["genres", "genre"], false)
            .await
    }

    /// `getSongsByGenre`.
    pub async fn get_songs_by_genre(
        &self,
        genre: &str,
        count: u32,
        offset: u32,
    ) -> Result<Vec<Child>> {
        self.call(
            "getSongsByGenre",
            &[
                ("genre", genre.into()),
                ("count", count.to_string()),
                ("offset", offset.to_string()),
            ],
            &["songsByGenre", "song"],
            false,
        )
        .await
    }

    /// `getLyricsBySongId` (OpenSubsonic `songLyrics`).
    pub async fn get_lyrics_by_song_id(&self, id: &str) -> Result<Vec<StructuredLyrics>> {
        self.call(
            "getLyricsBySongId",
            &[("id", id.into())],
            &["lyricsList", "structuredLyrics"],
            false,
        )
        .await
    }

    /// URL that streams a song, including authentication.
    pub fn stream_url(&self, id: &str, opts: &StreamOptions) -> Url {
        let mut p: Params = vec![("id", id.into())];
        if let Some(b) = opts.max_bit_rate.filter(|b| *b > 0) {
            p.push(("maxBitRate", b.to_string()));
        }
        if let Some(f) = &opts.format {
            p.push(("format", f.clone()));
        }
        if let Some(t) = opts.time_offset.filter(|t| *t > 0) {
            p.push(("timeOffset", t.to_string()));
        }
        if opts.estimate_content_length {
            p.push(("estimateContentLength", "true".into()));
        }
        self.url("stream", &p)
    }

    /// URL that downloads the original file.
    pub fn download_url(&self, id: &str) -> Url {
        self.url("download", &[("id", id.into())])
    }

    /// URL of a cover image, optionally scaled to `size` pixels.
    pub fn cover_art_url(&self, id: &str, size: Option<u32>) -> Url {
        let mut p: Params = vec![("id", id.into())];
        if let Some(s) = size {
            p.push(("size", s.to_string()));
        }
        self.url("getCoverArt", &p)
    }

    /// Download a cover image.
    pub async fn get_cover_art(&self, id: &str, size: Option<u32>) -> Result<Vec<u8>> {
        let resp = self
            .inner
            .http
            .get(self.cover_art_url(id, size))
            .send()
            .await?
            .error_for_status()?;
        let is_json = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.contains("json"));
        let bytes = resp.bytes().await?.to_vec();
        if is_json {
            // Errors are reported as a JSON document instead of an image.
            parse_response::<()>("getCoverArt", &bytes, &[], false)?;
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_url_is_normalised() {
        let c = Client::new(
            "https://user:pw@music.example.com/sub?x=1",
            Auth::ApiKey { key: "k".into() },
        )
        .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(c.base_url().as_str(), "https://music.example.com/sub/");
        let u = c.stream_url("42", &StreamOptions::default());
        assert!(
            u.as_str()
                .starts_with("https://music.example.com/sub/rest/stream?")
        );
        assert!(u.query_pairs().any(|(k, v)| k == "apiKey" && v == "k"));
        assert!(u.query_pairs().any(|(k, v)| k == "id" && v == "42"));
    }

    #[test]
    fn parse_error_response() {
        let body = br#"{"subsonic-response":{"status":"failed","version":"1.16.1","error":{"code":40,"message":"Wrong username or password"}}}"#;
        let err = parse_response::<()>("ping", body, &[], false).expect_err("api error");
        assert!(err.is_auth_error(), "{err}");
    }

    #[test]
    fn parse_missing_list_is_empty() {
        let body = br#"{"subsonic-response":{"status":"ok","version":"1.16.1","playlists":{}}}"#;
        let (_, v): (_, Vec<Playlist>) =
            parse_response("getPlaylists", body, &["playlists", "playlist"], false)
                .unwrap_or_else(|e| panic!("{e}"));
        assert!(v.is_empty());
    }

    #[test]
    fn parse_numeric_ids() {
        let body = br#"{"subsonic-response":{"status":"ok","version":"1.16.1","directory":{"id":12,"name":"X","child":[{"id":13,"parent":12,"isDir":false,"title":"T","coverArt":7}]}}}"#;
        let (_, d): (_, Directory) =
            parse_response("getMusicDirectory", body, &["directory"], true)
                .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(d.id, "12");
        assert_eq!(d.child[0].parent.as_deref(), Some("12"));
        assert_eq!(d.child[0].cover_art.as_deref(), Some("7"));
    }

    #[test]
    fn parse_null_list_is_empty() {
        let body = br#"{"subsonic-response":{"status":"ok","version":"1.16.1","lyricsList":{"structuredLyrics":null}}}"#;
        let (_, v): (_, Vec<StructuredLyrics>) = parse_response(
            "getLyricsBySongId",
            body,
            &["lyricsList", "structuredLyrics"],
            false,
        )
        .unwrap_or_else(|e| panic!("{e}"));
        assert!(v.is_empty());
    }

    #[test]
    fn parse_required_missing_is_error() {
        let body = br#"{"subsonic-response":{"status":"ok","version":"1.16.1"}}"#;
        let r = parse_response::<Album>("getAlbum", body, &["album"], true);
        assert!(matches!(r, Err(Error::MissingPayload { .. })));
    }
}
