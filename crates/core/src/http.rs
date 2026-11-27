// SPDX-License-Identifier: GPL-3.0-only
//! Streams tracks from the Subsonic server into the engine.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use infrasonic_engine::source::{StreamWriter, stream_pair};
use infrasonic_engine::{Error as EngineError, MediaSource, SourceProvider, Track};
use infrasonic_subsonic::{Client, StreamOptions};
use parking_lot::RwLock;

use crate::cache::{AudioCache, CacheWriter};

/// Opens tracks via the Subsonic `stream` endpoint. Downloads run on a Tokio runtime and feed
/// a [`stream_pair`], so the engine can start decoding while the file is still arriving.
#[derive(Debug, Clone)]
pub struct HttpSourceProvider {
    client: Client,
    http: reqwest::Client,
    handle: tokio::runtime::Handle,
    options: Arc<RwLock<StreamOptions>>,
    /// Options for pinned songs; `None` uses `options`.
    pin_options: Arc<RwLock<Option<StreamOptions>>>,
    cache: Arc<RwLock<Option<Arc<AudioCache>>>>,
    /// Bumped for each prefetch chain; an older chain stops when it sees a newer one.
    chain: Arc<AtomicU64>,
}

/// Cache variant name for stream options: the original file or a transcoding.
pub fn variant(o: &StreamOptions) -> String {
    match o.max_bit_rate.filter(|b| *b > 0) {
        Some(b) => format!("{}-{b}", o.format.as_deref().unwrap_or("default")),
        None => "raw".into(),
    }
}

impl HttpSourceProvider {
    /// Create a provider that spawns downloads on `handle`.
    pub fn new(client: Client, handle: tokio::runtime::Handle, options: StreamOptions) -> Self {
        let http = client.stream_http().clone();
        Self {
            client,
            http,
            handle,
            options: Arc::new(RwLock::new(options)),
            pin_options: Arc::new(RwLock::new(None)),
            cache: Arc::new(RwLock::new(None)),
            chain: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Use a disk cache (and pinned songs) for tracks opened from now on.
    pub fn set_cache(&self, cache: Option<Arc<AudioCache>>) {
        *self.cache.write() = cache;
    }

    pub fn cache(&self) -> Option<Arc<AudioCache>> {
        self.cache.read().clone()
    }

    /// Download songs into the cache without playing them, one after the other, once
    /// `playing` is cached and no other song is downloading. Prefetching thus never competes
    /// with playback for the connection, which other players may share. `ids` is asked for
    /// the songs at that point; a later call replaces the chain; cached songs are skipped.
    pub fn prefetch(&self, playing: &str, ids: impl FnOnce() -> Vec<String> + Send + 'static) {
        let Some(cache) = self.cache() else {
            return;
        };
        let generation = self.chain.fetch_add(1, Ordering::SeqCst) + 1;
        let this = self.clone();
        let playing = playing.to_string();
        self.handle.spawn(async move {
            let current = || this.chain.load(Ordering::SeqCst) == generation;
            let idle = |wait_for: Option<&str>| {
                let v = variant(&this.options.read());
                wait_for.is_none_or(|p| cache.contains(p, &v)) && cache.downloads() == 0
            };
            let mut wait_for = Some(playing.as_str());
            let mut ids = Some(ids);
            let mut list = Vec::new();
            loop {
                while !idle(wait_for) {
                    if !current() {
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
                if !current() {
                    return;
                }
                wait_for = None;
                if let Some(ids) = ids.take() {
                    list = ids();
                    list.reverse();
                }
                let Some(id) = list.pop() else {
                    return;
                };
                let opts = this.options.read().clone();
                let v = variant(&opts);
                if cache.contains(&id, &v) {
                    continue;
                }
                let Some(w) = cache.begin(&id, &v, false) else {
                    continue;
                };
                let url = this.client.stream_url(&id, &opts);
                download_file(this.http.clone(), url, w, None).await;
            }
        });
    }

    /// Download a song into the pinned store. Resolves when the file is in place.
    pub async fn pin(&self, id: &str) -> Result<(), String> {
        let cache = self.cache().ok_or("no song store configured")?;
        if cache.lookup(id, "pinned").is_some() {
            return Ok(());
        }
        let Some(w) = cache.begin(id, "pinned", true) else {
            return Err("already downloading".into());
        };
        let opts = self
            .pin_options
            .read()
            .clone()
            .unwrap_or_else(|| self.options.read().clone());
        let url = self.client.stream_url(id, &opts);
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.handle
            .spawn(download_file(self.http.clone(), url, w, Some(tx)));
        rx.await
            .unwrap_or_else(|_| Err("download cancelled".into()))
    }

    /// Bit rate / format for songs pinned from now on (`None`: same as streaming).
    pub fn set_pin_options(&self, options: Option<StreamOptions>) {
        *self.pin_options.write() = options;
    }

    /// Change bit rate / format for tracks opened from now on.
    pub fn set_options(&self, options: StreamOptions) {
        *self.options.write() = options;
    }
}

impl SourceProvider for HttpSourceProvider {
    fn open(&self, track: &Track) -> Result<Box<dyn MediaSource>, EngineError> {
        let opts = self.options.read().clone();
        let v = variant(&opts);
        let cache = self.cache();
        if let Some(c) = &cache
            && let Some(path) = c.lookup(&track.id, &v)
        {
            match std::fs::File::open(&path) {
                Ok(f) => {
                    tracing::debug!("playing {} from {}", track.id, path.display());
                    return Ok(Box::new(f));
                }
                Err(e) => tracing::warn!("cannot open {}: {e}", path.display()),
            }
        }
        let url = self.client.stream_url(&track.id, &opts);
        let (writer, reader) = stream_pair(None);
        let http = self.http.clone();
        let tee = cache.and_then(|c| c.begin(&track.id, &v, false));
        self.handle.spawn(download(http, url, writer, tee));
        Ok(Box::new(reader))
    }
}

/// How long a download keeps trying while no data arrives (mobile links drop and come back).
const GIVE_UP_AFTER: Duration = Duration::from_secs(120);
const FIRST_RETRY: Duration = Duration::from_millis(500);
const MAX_RETRY: Duration = Duration::from_secs(10);

/// Why a request did not deliver audio.
enum Failure {
    /// Worth trying again: the network, a timeout or the server for a moment.
    Again(String),
    /// The server refused (wrong song, no permission, an error document).
    Final(String),
}

/// Start a stream request from byte `offset` on. Returns the response and how many bytes of
/// it to drop because the server ignored the range and starts from the beginning; `None` when
/// there is nothing after `offset`. Subsonic reports errors as a JSON or XML document instead
/// of audio.
async fn request(
    http: &reqwest::Client,
    url: &url::Url,
    offset: u64,
) -> Result<Option<(reqwest::Response, u64)>, Failure> {
    let mut req = http.get(url.clone());
    if offset > 0 {
        req = req.header(reqwest::header::RANGE, format!("bytes={offset}-"));
    }
    let resp = req
        .send()
        .await
        .map_err(|e| Failure::Again(format!("stream request failed: {}", e.without_url())))?;
    let status = resp.status();
    if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE && offset > 0 {
        return Ok(None);
    }
    if !status.is_success() {
        let msg = format!("stream request failed: HTTP {status}");
        let again = status.is_server_error()
            || status == reqwest::StatusCode::REQUEST_TIMEOUT
            || status == reqwest::StatusCode::TOO_MANY_REQUESTS;
        return Err(if again {
            Failure::Again(msg)
        } else {
            Failure::Final(msg)
        });
    }
    let is_doc = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("json") || v.contains("xml"));
    if is_doc {
        let body = resp.bytes().await.unwrap_or_default();
        return Err(Failure::Final(
            match infrasonic_subsonic::parse_response::<()>("stream", &body, &[], false) {
                Err(e) => e.to_string(),
                Ok(_) => "server sent a document instead of audio".to_string(),
            },
        ));
    }
    let skip = if status == reqwest::StatusCode::PARTIAL_CONTENT {
        0
    } else {
        offset
    };
    Ok(Some((resp, skip)))
}

/// How a download ended.
enum Fetched {
    /// All of it arrived; the length the server announced first, if any.
    Complete(Option<u64>),
    /// `on_data` or `cancelled` asked to stop.
    Stopped,
}

/// Download `url`, going on with a range request where the connection broke. `on_len` hears
/// the total length once it is known, `on_data` gets every byte once and in order and returns
/// false to stop. Gives up after [`GIVE_UP_AFTER`] without progress or on a refusal.
async fn fetch(
    http: &reqwest::Client,
    url: &url::Url,
    mut on_len: impl FnMut(u64),
    mut on_data: impl FnMut(&[u8]) -> bool,
    cancelled: impl Fn() -> bool,
) -> Result<Fetched, String> {
    let mut received = 0u64;
    let mut total = None;
    let mut delay = FIRST_RETRY;
    let mut progress = tokio::time::Instant::now();
    loop {
        let error = match request(http, url, received).await {
            Ok(None) => return Ok(Fetched::Complete(total)),
            Err(Failure::Final(e)) => return Err(e),
            Err(Failure::Again(e)) => e,
            Ok(Some((mut resp, mut skip))) => {
                if total.is_none()
                    && let Some(len) = resp.content_length()
                {
                    let len = len + received - skip;
                    total = Some(len);
                    on_len(len);
                }
                loop {
                    match resp.chunk().await {
                        Ok(Some(bytes)) => {
                            let drop = skip.min(bytes.len() as u64);
                            skip -= drop;
                            let bytes = &bytes[drop as usize..];
                            if bytes.is_empty() {
                                continue;
                            }
                            received += bytes.len() as u64;
                            progress = tokio::time::Instant::now();
                            delay = FIRST_RETRY;
                            if !on_data(bytes) {
                                return Ok(Fetched::Stopped);
                            }
                        }
                        Ok(None) => return Ok(Fetched::Complete(total)),
                        Err(e) => break format!("download interrupted: {}", e.without_url()),
                    }
                }
            }
        };
        if progress.elapsed() > GIVE_UP_AFTER {
            return Err(error);
        }
        tracing::debug!("{error}; trying again in {delay:?} from byte {received}");
        tokio::time::sleep(delay).await;
        if cancelled() {
            return Ok(Fetched::Stopped);
        }
        delay = (delay * 2).min(MAX_RETRY);
    }
}

/// Download straight into the cache or the pinned store.
async fn download_file(
    http: reqwest::Client,
    url: url::Url,
    mut file: CacheWriter,
    done: Option<tokio::sync::oneshot::Sender<Result<(), String>>>,
) {
    let fetched = fetch(
        &http,
        &url,
        |_| {},
        |bytes| {
            file.write(bytes);
            true
        },
        || false,
    )
    .await;
    let result = match fetched {
        Ok(Fetched::Complete(len)) => file
            .commit(len)
            .map(|_| ())
            .ok_or_else(|| "could not store the song".to_string()),
        Ok(Fetched::Stopped) => Err("download cancelled".to_string()),
        Err(e) => Err(e),
    };
    if let Err(e) = &result {
        tracing::debug!("background download failed: {e}");
    }
    if let Some(tx) = done {
        let _ = tx.send(result);
    }
}

async fn download(
    http: reqwest::Client,
    url: url::Url,
    writer: StreamWriter,
    mut tee: Option<CacheWriter>,
) {
    let fetched = fetch(
        &http,
        &url,
        |len| writer.set_len(len),
        |bytes| {
            if let Some(t) = &mut tee {
                t.write(bytes);
            }
            writer.push(bytes)
        },
        || writer.is_cancelled(),
    )
    .await;
    match fetched {
        Ok(Fetched::Complete(len)) => {
            writer.finish();
            if let Some(t) = tee {
                t.commit(len);
            }
        }
        Ok(Fetched::Stopped) => tracing::debug!("stream reader gone, stopping download"),
        Err(e) => writer.fail(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// A server for `body` whose first answer breaks off after `cut` bytes. Later answers
    /// honor a range request if `ranges`, else send everything again. Records the ranges asked.
    async fn flaky_server(
        body: Vec<u8>,
        cut: usize,
        ranges: bool,
    ) -> (url::Url, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let asked = Arc::new(Mutex::new(Vec::new()));
        let log = asked.clone();
        tokio::spawn(async move {
            let mut first = true;
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                let mut req = vec![0u8; 4096];
                let n = sock.read(&mut req).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&req[..n]).to_lowercase();
                let from = req
                    .lines()
                    .find_map(|l| l.strip_prefix("range: bytes="))
                    .and_then(|r| r.trim_end_matches('-').trim().parse::<usize>().ok());
                log.lock()
                    .unwrap()
                    .push(from.map(|f| f.to_string()).unwrap_or_default());
                let (status, part) = match from {
                    Some(f) if ranges => ("206 Partial Content", &body[f..]),
                    _ => ("200 OK", &body[..]),
                };
                let head = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: audio/flac\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    part.len()
                );
                let _ = sock.write_all(head.as_bytes()).await;
                let send = if first { &part[..cut] } else { part };
                first = false;
                let _ = sock.write_all(send).await;
                let _ = sock.shutdown().await;
            }
        });
        (
            url::Url::parse(&format!("http://{addr}/stream")).unwrap(),
            asked,
        )
    }

    async fn get(url: &url::Url) -> (Result<Fetched, String>, Vec<u8>, Option<u64>) {
        infrasonic_subsonic::ensure_crypto_provider();
        let http = reqwest::Client::new();
        let mut got = Vec::new();
        let mut len = None;
        let r = fetch(
            &http,
            url,
            |l| len = Some(l),
            |b| {
                got.extend_from_slice(b);
                true
            },
            || false,
        )
        .await;
        (r, got, len)
    }

    fn body() -> Vec<u8> {
        (0..200_000u32).map(|i| (i % 251) as u8).collect()
    }

    #[tokio::test]
    async fn broken_download_goes_on_with_a_range() {
        let (url, asked) = flaky_server(body(), 70_000, true).await;
        let (r, got, len) = get(&url).await;
        assert!(matches!(r, Ok(Fetched::Complete(Some(200_000)))));
        assert_eq!(len, Some(200_000));
        assert!(got == body(), "got {} bytes, not the same", got.len());
        let asked = asked.lock().unwrap().clone();
        assert_eq!(asked.len(), 2);
        assert_eq!(asked[0], "");
        assert!(asked[1].parse::<usize>().unwrap() > 0);
    }

    #[tokio::test]
    async fn server_without_ranges_is_read_again_from_the_start() {
        let (url, _) = flaky_server(body(), 70_000, false).await;
        let (r, got, _) = get(&url).await;
        assert!(matches!(r, Ok(Fetched::Complete(_))));
        assert!(got == body(), "got {} bytes, not the same", got.len());
    }

    #[tokio::test]
    async fn refusal_is_not_tried_again() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::any())
            .respond_with(wiremock::ResponseTemplate::new(404))
            .expect(1)
            .mount(&server)
            .await;
        let url = url::Url::parse(&server.uri()).unwrap();
        let (r, _, _) = get(&url).await;
        assert!(matches!(r, Err(e) if e.contains("404")));
    }

    #[tokio::test]
    async fn server_error_is_tried_again() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::any())
            .respond_with(wiremock::ResponseTemplate::new(503))
            .up_to_n_times(2)
            .mount(&server)
            .await;
        wiremock::Mock::given(wiremock::matchers::any())
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .insert_header("content-type", "audio/mpeg")
                    .set_body_bytes(vec![1u8; 1000]),
            )
            .mount(&server)
            .await;
        let url = url::Url::parse(&server.uri()).unwrap();
        let (r, got, _) = get(&url).await;
        assert!(matches!(r, Ok(Fetched::Complete(Some(1000)))));
        assert_eq!(got.len(), 1000);
    }
}
