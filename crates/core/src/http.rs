// SPDX-License-Identifier: GPL-3.0-only
//! Streams tracks from the Subsonic server into the engine.

use std::sync::Arc;
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
    cache: Arc<RwLock<Option<Arc<AudioCache>>>>,
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
        infrasonic_subsonic::ensure_crypto_provider();
        // No total timeout: large files on slow links may take minutes.
        let builder = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(30))
            .user_agent(concat!("infrasonic/", env!("CARGO_PKG_VERSION")));
        let http = builder.build().unwrap_or_else(|_| client.http().clone());
        Self {
            client,
            http,
            handle,
            options: Arc::new(RwLock::new(options)),
            cache: Arc::new(RwLock::new(None)),
        }
    }

    /// Use a disk cache (and pinned songs) for tracks opened from now on.
    pub fn set_cache(&self, cache: Option<Arc<AudioCache>>) {
        *self.cache.write() = cache;
    }

    pub fn cache(&self) -> Option<Arc<AudioCache>> {
        self.cache.read().clone()
    }

    /// Download a song into the cache without playing it. Returns false if it is cached
    /// already, being downloaded, or caching is off.
    pub fn prefetch(&self, id: &str) -> bool {
        let Some(cache) = self.cache() else {
            return false;
        };
        let opts = self.options.read().clone();
        let v = variant(&opts);
        if cache.lookup(id, &v).is_some() {
            return false;
        }
        let Some(w) = cache.begin(id, &v, false) else {
            return false;
        };
        let url = self.client.stream_url(id, &opts);
        self.handle
            .spawn(download_file(self.http.clone(), url, w, None));
        true
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
        let url = self.client.stream_url(id, &self.options.read().clone());
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.handle
            .spawn(download_file(self.http.clone(), url, w, Some(tx)));
        rx.await
            .unwrap_or_else(|_| Err("download cancelled".into()))
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

/// Start a stream request. Subsonic reports errors as a JSON or XML document instead of audio.
async fn request(http: &reqwest::Client, url: url::Url) -> Result<reqwest::Response, String> {
    let resp = http
        .get(url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("stream request failed: {}", e.without_url()))?;
    let is_doc = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("json") || v.contains("xml"));
    if is_doc {
        let body = resp.bytes().await.unwrap_or_default();
        return Err(
            match infrasonic_subsonic::parse_response::<()>("stream", &body, &[], false) {
                Err(e) => e.to_string(),
                Ok(_) => "server sent a document instead of audio".to_string(),
            },
        );
    }
    Ok(resp)
}

/// Download straight into the cache or the pinned store.
async fn download_file(
    http: reqwest::Client,
    url: url::Url,
    mut file: CacheWriter,
    done: Option<tokio::sync::oneshot::Sender<Result<(), String>>>,
) {
    let result = async {
        let mut resp = request(&http, url).await?;
        let len = resp.content_length();
        while let Some(bytes) = resp
            .chunk()
            .await
            .map_err(|e| format!("download interrupted: {}", e.without_url()))?
        {
            file.write(&bytes);
        }
        file.commit(len)
            .map(|_| ())
            .ok_or_else(|| "could not store the song".to_string())
    }
    .await;
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
    let resp = match request(&http, url).await {
        Ok(r) => r,
        Err(e) => {
            writer.fail(e);
            return;
        }
    };
    let len = resp.content_length();
    if let Some(len) = len {
        writer.set_len(len);
    }
    let mut resp = resp;
    loop {
        match resp.chunk().await {
            Ok(Some(bytes)) => {
                if let Some(t) = &mut tee {
                    t.write(&bytes);
                }
                if !writer.push(&bytes) {
                    tracing::debug!("stream reader gone, stopping download");
                    return;
                }
            }
            Ok(None) => {
                writer.finish();
                if let Some(t) = tee {
                    t.commit(len);
                }
                return;
            }
            Err(e) => {
                writer.fail(format!("download interrupted: {}", e.without_url()));
                return;
            }
        }
    }
}
