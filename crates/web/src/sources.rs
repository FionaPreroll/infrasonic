// SPDX-License-Identifier: GPL-3.0-only
//! Song downloads for the engine in the worker. Pump mode must never block, so a song is
//! opened only when it is completely in memory; [`Sources::is_ready`] starts the download.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use infrasonic_engine::source::StreamReader;
use infrasonic_engine::{Error, MediaSource, SourceProvider, Track};
use parking_lot::Mutex;

/// Songs kept in memory besides the ones loading (current, next, a few recent).
const KEEP: usize = 6;

#[derive(Debug, Clone)]
enum State {
    Loading,
    Ready(Arc<Vec<u8>>),
    Failed(String),
}

#[derive(Debug, Default)]
struct Inner {
    urls: HashMap<String, String>,
    songs: HashMap<String, State>,
    /// Ready songs, oldest first.
    order: VecDeque<String>,
    /// Downloads to start; taken by the host, which owns the async runtime.
    requests: Vec<(String, String)>,
}

/// Download state shared by the engine (through [`SourceProvider`]) and the host.
#[derive(Debug, Default)]
pub struct Sources {
    inner: Mutex<Inner>,
}

impl Sources {
    pub fn set_urls(&self, urls: impl IntoIterator<Item = (String, String)>) {
        self.inner.lock().urls.extend(urls);
    }

    /// Ask for a download of `id` unless it is loaded or loading.
    pub fn want(&self, id: &str) {
        let mut g = self.inner.lock();
        if g.songs.contains_key(id) {
            return;
        }
        let Some(url) = g.urls.get(id).cloned() else {
            g.songs
                .insert(id.to_string(), State::Failed(format!("no URL for {id}")));
            return;
        };
        g.songs.insert(id.to_string(), State::Loading);
        g.requests.push((id.to_string(), url));
    }

    /// Downloads the host should start now.
    pub fn take_requests(&self) -> Vec<(String, String)> {
        std::mem::take(&mut self.inner.lock().requests)
    }

    pub fn finished(&self, id: &str, result: Result<Vec<u8>, String>) {
        let mut g = self.inner.lock();
        match result {
            Ok(bytes) => {
                g.songs
                    .insert(id.to_string(), State::Ready(Arc::new(bytes)));
                g.order.retain(|x| x != id);
                g.order.push_back(id.to_string());
                while g.order.len() > KEEP {
                    if let Some(old) = g.order.pop_front() {
                        g.songs.remove(&old);
                    }
                }
            }
            Err(e) => {
                g.songs.insert(id.to_string(), State::Failed(e));
            }
        }
    }

    /// Forget a failed download so that it can be tried again.
    pub fn retry(&self, id: &str) {
        let mut g = self.inner.lock();
        if matches!(g.songs.get(id), Some(State::Failed(_))) {
            g.songs.remove(id);
        }
    }

    /// (songs in memory, downloads running)
    pub fn counts(&self) -> (usize, usize) {
        let g = self.inner.lock();
        let loading = g
            .songs
            .values()
            .filter(|s| matches!(s, State::Loading))
            .count();
        (g.order.len(), loading)
    }
}

impl SourceProvider for Sources {
    fn open(&self, track: &Track) -> Result<Box<dyn MediaSource>, Error> {
        let state = self.inner.lock().songs.get(&track.id).cloned();
        match state {
            Some(State::Ready(bytes)) => Ok(Box::new(StreamReader::from_bytes(bytes.to_vec()))),
            Some(State::Failed(e)) => {
                // A later play of the same song tries again.
                self.retry(&track.id);
                Err(Error::Source(e))
            }
            _ => Err(Error::Source(format!("{} is not loaded", track.title))),
        }
    }

    fn is_ready(&self, track: &Track) -> bool {
        self.want(&track.id);
        !matches!(
            self.inner.lock().songs.get(&track.id),
            Some(State::Loading) | None
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downloads_once_and_evicts_old_songs() {
        let s = Sources::default();
        s.set_urls((0..10).map(|i| (format!("s{i}"), format!("u{i}"))));
        let t = Track::new("s0", "Song 0");
        assert!(!s.is_ready(&t));
        assert!(!s.is_ready(&t));
        assert_eq!(s.take_requests(), [("s0".to_string(), "u0".to_string())]);
        assert!(s.take_requests().is_empty(), "asked only once");
        s.finished("s0", Ok(vec![1, 2, 3]));
        assert!(s.is_ready(&t));
        for i in 1..10 {
            s.want(&format!("s{i}"));
            s.finished(&format!("s{i}"), Ok(vec![0]));
        }
        assert_eq!(s.counts(), (KEEP, 0));
        assert!(!s.is_ready(&t), "s0 was evicted and loads again");
    }

    #[test]
    fn failures_are_reported_and_retried() {
        let s = Sources::default();
        let t = Track::new("x", "X");
        assert!(s.is_ready(&t), "no URL: fails at once");
        assert!(s.open(&t).is_err());
        s.set_urls([("x".to_string(), "u".to_string())]);
        assert!(!s.is_ready(&t), "tried again after the failure");
        s.finished("x", Err("404".into()));
        assert!(s.is_ready(&t));
        let e = s.open(&t).err().map(|e| e.to_string()).unwrap_or_default();
        assert!(e.contains("404"), "{e}");
    }
}
