// SPDX-License-Identifier: GPL-3.0-only
//! Streams tracks from the Subsonic server into the engine.

use std::sync::Arc;
use std::time::Duration;

use infrasonic_engine::source::{StreamWriter, stream_pair};
use infrasonic_engine::{Error as EngineError, MediaSource, SourceProvider, Track};
use infrasonic_subsonic::{Client, StreamOptions};
use parking_lot::RwLock;

/// Opens tracks via the Subsonic `stream` endpoint. Downloads run on a Tokio runtime and feed
/// a [`stream_pair`], so the engine can start decoding while the file is still arriving.
#[derive(Debug, Clone)]
pub struct HttpSourceProvider {
    client: Client,
    http: reqwest::Client,
    handle: tokio::runtime::Handle,
    options: Arc<RwLock<StreamOptions>>,
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
        }
    }

    /// Change bit rate / format for tracks opened from now on.
    pub fn set_options(&self, options: StreamOptions) {
        *self.options.write() = options;
    }
}

impl SourceProvider for HttpSourceProvider {
    fn open(&self, track: &Track) -> Result<Box<dyn MediaSource>, EngineError> {
        let url = self
            .client
            .stream_url(&track.id, &self.options.read().clone());
        let (writer, reader) = stream_pair(None);
        let http = self.http.clone();
        self.handle.spawn(download(http, url, writer));
        Ok(Box::new(reader))
    }
}

async fn download(http: reqwest::Client, url: url::Url, writer: StreamWriter) {
    let resp = match http
        .get(url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
    {
        Ok(r) => r,
        Err(e) => {
            writer.fail(format!("stream request failed: {}", e.without_url()));
            return;
        }
    };
    let is_json = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("json") || v.contains("xml"));
    if is_json {
        // Subsonic reports errors (e.g. "not found") as a document instead of audio.
        let body = resp.bytes().await.unwrap_or_default();
        let msg = match infrasonic_subsonic::parse_response::<()>("stream", &body, &[], false) {
            Err(e) => e.to_string(),
            Ok(_) => "server sent a document instead of audio".to_string(),
        };
        writer.fail(msg);
        return;
    }
    if let Some(len) = resp.content_length() {
        writer.set_len(len);
    }
    let mut resp = resp;
    loop {
        match resp.chunk().await {
            Ok(Some(bytes)) => {
                if !writer.push(&bytes) {
                    tracing::debug!("stream reader gone, stopping download");
                    return;
                }
            }
            Ok(None) => {
                writer.finish();
                return;
            }
            Err(e) => {
                writer.fail(format!("download interrupted: {}", e.without_url()));
                return;
            }
        }
    }
}
