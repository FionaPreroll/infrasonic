// SPDX-License-Identifier: GPL-3.0-only
//! Audio on disk: a size-limited cache of streamed songs and pinned songs for offline use.
//!
//! Cached files live in `<cache dir>/audio/<server>/`, named after the song id and the
//! streaming variant (original or a transcoded bit rate). The least recently played files
//! are deleted when the cache grows beyond its limit. Pinned songs live in
//! `<data dir>/pinned/<server>/` together with an `index.json` of their metadata, and are
//! never evicted.

use std::collections::HashSet;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use infrasonic_subsonic::Child;
use parking_lot::Mutex;

/// Stable short key for a server and user, used as directory name.
pub fn server_key(url: &str, username: &str) -> String {
    // FNV-1a, enough to keep servers apart; not a security feature.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in url
        .trim_end_matches('/')
        .bytes()
        .chain([0])
        .chain(username.bytes())
    {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// File name for a song id: safe characters are kept, anything else is hex encoded.
fn safe_name(id: &str) -> String {
    if !id.is_empty()
        && id.len() <= 100
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        id.to_string()
    } else {
        let mut s = String::from("x");
        for b in id.bytes().take(100) {
            s.push_str(&format!("{b:02x}"));
        }
        s
    }
}

/// Disk usage of the cache.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub files: u64,
    pub bytes: u64,
}

/// Cache and pin store for one server.
#[derive(Debug)]
pub struct AudioCache {
    dir: PathBuf,
    pinned_dir: PathBuf,
    max_bytes: u64,
    inflight: Mutex<HashSet<String>>,
    pins: Mutex<Vec<Child>>,
}

impl AudioCache {
    /// `max_bytes = 0` disables caching of streamed songs; pinning still works.
    pub fn new(cache_root: &Path, data_root: &Path, server: &str, max_bytes: u64) -> Self {
        let pinned_dir = data_root.join("pinned").join(server);
        let pins = std::fs::read(pinned_dir.join("index.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<Vec<Child>>(&b).ok())
            .unwrap_or_default();
        Self {
            dir: cache_root.join("audio").join(server),
            pinned_dir,
            max_bytes,
            inflight: Mutex::new(HashSet::new()),
            pins: Mutex::new(pins),
        }
    }

    /// Whether streamed songs are cached.
    pub fn enabled(&self) -> bool {
        self.max_bytes > 0
    }

    fn cache_path(&self, id: &str, variant: &str) -> PathBuf {
        self.dir
            .join(format!("{}.{}", safe_name(id), safe_name(variant)))
    }

    fn pin_path(&self, id: &str) -> PathBuf {
        self.pinned_dir.join(safe_name(id))
    }

    /// A local file for the song: pinned first, then cached in this variant.
    pub fn lookup(&self, id: &str, variant: &str) -> Option<PathBuf> {
        let pinned = self.pin_path(id);
        if pinned.is_file() {
            return Some(pinned);
        }
        if !self.enabled() {
            return None;
        }
        let p = self.cache_path(id, variant);
        if p.is_file() {
            // Mark as recently used.
            if let Ok(f) = std::fs::File::options().append(true).open(&p) {
                let _ = f.set_modified(SystemTime::now());
            }
            return Some(p);
        }
        None
    }

    /// Claim a download, so that the same song is not fetched twice at once.
    /// Returns `None` if it is already being downloaded.
    pub fn begin(self: &Arc<Self>, id: &str, variant: &str, pin: bool) -> Option<CacheWriter> {
        if !pin && !self.enabled() {
            return None;
        }
        let key = format!("{}:{variant}:{pin}", safe_name(id));
        if !self.inflight.lock().insert(key.clone()) {
            return None;
        }
        let (dir, target) = if pin {
            (self.pinned_dir.clone(), self.pin_path(id))
        } else {
            (self.dir.clone(), self.cache_path(id, variant))
        };
        let tmp = dir.join(format!(".{}.{}.part", safe_name(id), std::process::id()));
        let file = std::fs::create_dir_all(&dir)
            .and_then(|()| std::fs::File::create(&tmp))
            .map_err(|e| tracing::warn!("cache: cannot create {}: {e}", tmp.display()))
            .ok();
        let Some(file) = file else {
            self.inflight.lock().remove(&key);
            return None;
        };
        Some(CacheWriter {
            cache: self.clone(),
            key,
            tmp,
            target,
            file: Some(std::io::BufWriter::new(file)),
            written: 0,
            pin,
        })
    }

    /// Delete the least recently used files until the cache fits its limit.
    pub fn evict(&self) {
        let Ok(rd) = std::fs::read_dir(&self.dir) else {
            return;
        };
        let mut files: Vec<(SystemTime, u64, PathBuf)> = rd
            .filter_map(Result::ok)
            .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
            .filter_map(|e| {
                let m = e.metadata().ok()?;
                m.is_file().then(|| {
                    (
                        m.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                        m.len(),
                        e.path(),
                    )
                })
            })
            .collect();
        let mut total: u64 = files.iter().map(|f| f.1).sum();
        if total <= self.max_bytes {
            return;
        }
        files.sort();
        for (_, len, path) in files {
            if total <= self.max_bytes {
                break;
            }
            if std::fs::remove_file(&path).is_ok() {
                total = total.saturating_sub(len);
            }
        }
    }

    /// Size of the cache (not counting pinned songs).
    pub fn usage(&self) -> Usage {
        let mut u = Usage::default();
        if let Ok(rd) = std::fs::read_dir(&self.dir) {
            for m in rd.filter_map(Result::ok).filter_map(|e| e.metadata().ok()) {
                if m.is_file() {
                    u.files += 1;
                    u.bytes += m.len();
                }
            }
        }
        u
    }

    /// Size of the pinned songs.
    pub fn pinned_usage(&self) -> Usage {
        let mut u = Usage::default();
        if let Ok(rd) = std::fs::read_dir(&self.pinned_dir) {
            for e in rd.filter_map(Result::ok) {
                let name = e.file_name();
                if name.to_string_lossy().starts_with('.') || name == "index.json" {
                    continue;
                }
                if let Ok(m) = e.metadata()
                    && m.is_file()
                {
                    u.files += 1;
                    u.bytes += m.len();
                }
            }
        }
        u
    }

    /// Delete all cached (not pinned) songs.
    pub fn clear(&self) -> std::io::Result<()> {
        match std::fs::remove_dir_all(&self.dir) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    /// Pinned songs, in the order they were pinned.
    pub fn pinned(&self) -> Vec<Child> {
        self.pins.lock().clone()
    }

    pub fn is_pinned(&self, id: &str) -> bool {
        self.pins.lock().iter().any(|c| c.id == id)
    }

    fn save_pins(&self, pins: &[Child]) {
        let res = std::fs::create_dir_all(&self.pinned_dir).and_then(|()| {
            let tmp = self.pinned_dir.join(".index.json.part");
            std::fs::write(&tmp, serde_json::to_vec_pretty(pins).unwrap_or_default())?;
            std::fs::rename(&tmp, self.pinned_dir.join("index.json"))
        });
        if let Err(e) = res {
            tracing::warn!("cannot save pinned songs: {e}");
        }
    }

    /// Record a pinned song whose file is in place.
    fn add_pin(&self, song: &Child) {
        let mut pins = self.pins.lock();
        if !pins.iter().any(|c| c.id == song.id) {
            pins.push(song.clone());
            self.save_pins(&pins);
        }
    }

    /// Remember the metadata of a song that is about to be pinned.
    pub fn note_pin(&self, song: &Child) {
        if self.pin_path(&song.id).is_file() {
            self.add_pin(song);
        }
    }

    /// Remove a pinned song and its file.
    pub fn unpin(&self, id: &str) {
        let mut pins = self.pins.lock();
        pins.retain(|c| c.id != id);
        self.save_pins(&pins);
        let _ = std::fs::remove_file(self.pin_path(id));
    }
}

/// A file being downloaded into the cache. Dropped without [`CacheWriter::commit`], the
/// partial file is deleted.
#[derive(Debug)]
pub struct CacheWriter {
    cache: Arc<AudioCache>,
    key: String,
    tmp: PathBuf,
    target: PathBuf,
    file: Option<std::io::BufWriter<std::fs::File>>,
    written: u64,
    pin: bool,
}

impl CacheWriter {
    /// Append bytes. On a write error the download continues without caching.
    pub fn write(&mut self, bytes: &[u8]) {
        if let Some(f) = &mut self.file {
            if let Err(e) = f.write_all(bytes) {
                tracing::warn!("cache write failed: {e}");
                self.file = None;
            } else {
                self.written += bytes.len() as u64;
            }
        }
    }

    /// Move the complete file into place. `expected` is the Content-Length, if known.
    pub fn commit(mut self, expected: Option<u64>) -> Option<PathBuf> {
        let f = self.file.take()?;
        if expected.is_some_and(|e| e != self.written) || self.written == 0 {
            return None;
        }
        let ok = f
            .into_inner()
            .map_err(|e| e.into_error())
            .and_then(|f| f.sync_all())
            .and_then(|()| std::fs::rename(&self.tmp, &self.target));
        match ok {
            Ok(()) => {
                if !self.pin {
                    self.cache.evict();
                }
                Some(self.target.clone())
            }
            Err(e) => {
                tracing::warn!("cache: cannot store {}: {e}", self.target.display());
                None
            }
        }
    }
}

impl Drop for CacheWriter {
    fn drop(&mut self) {
        self.file = None;
        let _ = std::fs::remove_file(&self.tmp);
        self.cache.inflight.lock().remove(&self.key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(id: &str) -> Child {
        Child {
            id: id.into(),
            title: format!("Song {id}"),
            ..Child::default()
        }
    }

    #[test]
    fn stores_looks_up_and_evicts() {
        let dir = tempfile::tempdir().expect("tempdir");
        let c = Arc::new(AudioCache::new(
            &dir.path().join("c"),
            &dir.path().join("d"),
            "srv",
            250,
        ));
        assert_eq!(c.lookup("a", "raw"), None);
        for id in ["a", "b", "c"] {
            let mut w = c.begin(id, "raw", false).expect("writer");
            // A second download of the same song is refused while the first runs.
            assert!(c.begin(id, "raw", false).is_none());
            w.write(&[1u8; 100]);
            assert!(w.commit(Some(100)).is_some());
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        // 300 bytes > 250: the oldest file went.
        assert_eq!(c.lookup("a", "raw"), None);
        assert!(c.lookup("b", "raw").is_some());
        assert!(c.lookup("c", "raw").is_some());
        assert_eq!(c.lookup("c", "mp3-128"), None);
        assert_eq!(
            c.usage(),
            Usage {
                files: 2,
                bytes: 200
            }
        );
        c.clear().expect("clear");
        assert_eq!(c.usage(), Usage::default());
    }

    #[test]
    fn incomplete_downloads_are_discarded() {
        let dir = tempfile::tempdir().expect("tempdir");
        let c = Arc::new(AudioCache::new(dir.path(), dir.path(), "srv", 1 << 20));
        let mut w = c.begin("x", "raw", false).expect("writer");
        w.write(b"half");
        assert!(w.commit(Some(8)).is_none());
        assert_eq!(c.lookup("x", "raw"), None);
        let mut w = c.begin("x", "raw", false).expect("writer again");
        w.write(b"data");
        drop(w);
        assert_eq!(c.lookup("x", "raw"), None);
        assert_eq!(c.usage().files, 0, "no partial files left");
    }

    #[test]
    fn pins_survive_and_win() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (cr, dr) = (dir.path().join("c"), dir.path().join("d"));
        {
            // Caching disabled: pinning still works.
            let c = Arc::new(AudioCache::new(&cr, &dr, "srv", 0));
            assert!(c.begin("p/1", "raw", false).is_none());
            let mut w = c.begin("p/1", "raw", true).expect("pin writer");
            w.write(b"pinned");
            assert!(w.commit(None).is_some());
            c.note_pin(&song("p/1"));
            assert!(c.is_pinned("p/1"));
        }
        let c = AudioCache::new(&cr, &dr, "srv", 0);
        assert_eq!(c.pinned(), vec![song("p/1")]);
        let p = c.lookup("p/1", "mp3-128").expect("pinned file");
        assert_eq!(std::fs::read(p).expect("read"), b"pinned");
        c.unpin("p/1");
        assert!(c.pinned().is_empty());
        assert_eq!(c.lookup("p/1", "raw"), None);
        // Other servers do not see it.
        assert_ne!(server_key("https://a", "me"), server_key("https://b", "me"));
        assert_eq!(
            server_key("https://a/", "me"),
            server_key("https://a", "me")
        );
    }
}
