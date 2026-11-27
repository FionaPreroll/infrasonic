// SPDX-License-Identifier: GPL-3.0-only
//! A connected session: Subsonic client + playback engine + scrobbling + queue sync.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use infrasonic_engine::output::OutputFactory;
use infrasonic_engine::{Command, Engine, EngineConfig, Event, Snapshot, SnapshotReader, Track};
use infrasonic_subsonic::{Child, Client, ServerInfo, StarTarget, StreamOptions};
use tokio::sync::mpsc;

use crate::cache::AudioCache;
use crate::config::{PlaybackConfig, Profile, resolve_auth};
use crate::http::HttpSourceProvider;
use crate::scrobble::{ScrobbleAction, ScrobbleTracker};
use crate::{CoreError, track_from_child};

/// Events for frontends.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionEvent {
    /// Forwarded engine event.
    Engine(Event),
    /// A play was submitted to the server.
    Scrobbled { id: String },
    /// A song was downloaded for offline use.
    Pinned { id: String },
    /// A background operation failed.
    Error(String),
}

/// Limits for recursive directory expansion.
const MAX_DEPTH: usize = 8;
const MAX_SONGS: usize = 5000;

/// A connected server plus the engine that plays from it.
#[derive(Debug)]
pub struct Session {
    client: Client,
    info: ServerInfo,
    engine: Engine,
    provider: HttpSourceProvider,
    playback: PlaybackConfig,
    profile_name: String,
    events: mpsc::UnboundedSender<SessionEvent>,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn stream_options(p: &PlaybackConfig) -> StreamOptions {
    if p.max_bit_rate > 0 {
        StreamOptions {
            max_bit_rate: Some(p.max_bit_rate),
            format: p.format.clone(),
            estimate_content_length: true,
            ..StreamOptions::default()
        }
    } else {
        StreamOptions {
            format: Some("raw".into()),
            ..StreamOptions::default()
        }
    }
}

impl Session {
    /// Connect to the server of `profile` and start an engine with the given output.
    /// Must be called from within a Tokio runtime.
    pub async fn connect(
        profile_name: &str,
        profile: &Profile,
        playback: PlaybackConfig,
        output: Box<dyn OutputFactory>,
    ) -> Result<(Self, mpsc::UnboundedReceiver<SessionEvent>), CoreError> {
        let auth = resolve_auth(profile_name, profile)?;
        let client = Client::builder(&profile.url, auth)
            .client_name("infrasonic")
            .build()?;
        let info = client.connect().await?;
        Ok(Self::with_client(
            client,
            info,
            profile_name,
            playback,
            output,
        ))
    }

    /// A session that does not talk to the server on start, for playing pinned songs while
    /// the server is unreachable. Requests made later still go to the server.
    pub fn offline(
        profile_name: &str,
        profile: &Profile,
        playback: PlaybackConfig,
        output: Box<dyn OutputFactory>,
    ) -> Result<(Self, mpsc::UnboundedReceiver<SessionEvent>), CoreError> {
        let auth = resolve_auth(profile_name, profile)?;
        let client = Client::builder(&profile.url, auth)
            .client_name("infrasonic")
            .build()?;
        Ok(Self::with_client(
            client,
            ServerInfo::default(),
            profile_name,
            playback,
            output,
        ))
    }

    /// Build a session around an already connected client.
    pub fn with_client(
        client: Client,
        info: ServerInfo,
        profile_name: &str,
        playback: PlaybackConfig,
        output: Box<dyn OutputFactory>,
    ) -> (Self, mpsc::UnboundedReceiver<SessionEvent>) {
        let handle = tokio::runtime::Handle::current();
        let provider =
            HttpSourceProvider::new(client.clone(), handle.clone(), stream_options(&playback));
        let engine = Engine::new(
            EngineConfig {
                volume: playback.volume,
                replay_gain: playback.replay_gain_config(),
                ..EngineConfig::default()
            },
            Arc::new(provider.clone()),
            output,
        );
        let (tx, rx) = mpsc::unbounded_channel();
        spawn_event_pump(Pump {
            events: engine.events().clone(),
            tx: tx.clone(),
            client: client.clone(),
            handle,
            scrobble: playback.scrobble,
            provider: provider.clone(),
            snapshot: engine.snapshot_reader(),
            preload: playback.preload_songs as usize,
        });
        (
            Self {
                client,
                info,
                engine,
                provider,
                playback,
                profile_name: profile_name.to_string(),
                events: tx,
            },
            rx,
        )
    }

    pub fn client(&self) -> &Client {
        &self.client
    }
    pub fn server_info(&self) -> &ServerInfo {
        &self.info
    }
    pub fn profile_name(&self) -> &str {
        &self.profile_name
    }
    pub fn playback_config(&self) -> &PlaybackConfig {
        &self.playback
    }
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Send a command to the engine.
    pub fn send(&self, cmd: Command) {
        self.engine.send(cmd);
    }

    /// Current engine state.
    pub fn snapshot(&self) -> Snapshot {
        self.engine.snapshot()
    }

    /// Use a song cache and pinned songs (see [`AudioCache`]).
    pub fn set_cache(&self, cache: Option<Arc<AudioCache>>) {
        self.provider.set_cache(cache);
    }

    pub fn cache(&self) -> Option<Arc<AudioCache>> {
        self.provider.cache()
    }

    /// Whether the server answered on connect (false for [`Session::offline`]).
    pub fn is_online(&self) -> bool {
        !self.info.version.is_empty()
    }

    /// Download songs for offline use, one after the other, in the background. Each
    /// finished song is reported as [`SessionEvent::Pinned`].
    pub fn pin(&self, songs: Vec<Child>) {
        let provider = self.provider.clone();
        let tx = self.events.clone();
        tokio::runtime::Handle::current().spawn(async move {
            pin_songs(&provider, &tx, songs).await;
        });
    }

    /// Download songs for offline use and wait until all are done. Returns the number of
    /// songs that could not be downloaded; events are sent as with [`Session::pin`].
    pub async fn pin_and_wait(&self, songs: Vec<Child>) -> usize {
        pin_songs(&self.provider, &self.events, songs).await
    }

    /// Remove pinned songs.
    pub fn unpin(&self, ids: &[String]) {
        if let Some(c) = self.cache() {
            for id in ids {
                c.unpin(id);
            }
        }
    }

    /// Change ReplayGain settings.
    pub fn set_replay_gain(&self, setting: crate::config::ReplayGainSetting) {
        let mut p = self.playback.clone();
        p.replay_gain = setting;
        self.send(Command::SetReplayGain(p.replay_gain_config()));
    }

    /// Change streaming quality for tracks opened from now on.
    pub fn set_max_bit_rate(&self, kbps: u32, format: Option<String>) {
        let mut p = self.playback.clone();
        p.max_bit_rate = kbps;
        p.format = format;
        self.provider.set_options(stream_options(&p));
    }

    /// Change the quality of songs pinned from now on; `None` pins like streaming.
    pub fn set_pin_bit_rate(&self, setting: Option<(u32, Option<String>)>) {
        self.provider.set_pin_options(setting.map(|(kbps, format)| {
            let mut p = self.playback.clone();
            p.max_bit_rate = kbps;
            p.format = format;
            stream_options(&p)
        }));
    }

    /// Replace the queue with `songs` and start playing at `start`.
    pub fn play_songs(&self, songs: &[Child], start: usize) {
        let tracks: Vec<Track> = songs
            .iter()
            .filter(|c| !c.is_dir)
            .map(track_from_child)
            .collect();
        if tracks.is_empty() {
            return;
        }
        self.send(Command::SetQueue {
            tracks,
            start: start.min(songs.len().saturating_sub(1)),
            position: 0.0,
            play: true,
        });
    }

    /// Append songs to the queue.
    pub fn enqueue(&self, songs: &[Child]) {
        let tracks: Vec<Track> = songs
            .iter()
            .filter(|c| !c.is_dir)
            .map(track_from_child)
            .collect();
        if !tracks.is_empty() {
            self.send(Command::Append(tracks));
        }
    }

    /// Insert songs after the current one.
    pub fn enqueue_next(&self, songs: &[Child]) {
        let tracks: Vec<Track> = songs
            .iter()
            .filter(|c| !c.is_dir)
            .map(track_from_child)
            .collect();
        if !tracks.is_empty() {
            self.send(Command::InsertNext(tracks));
        }
    }

    /// All songs below a directory, depth first, in server order.
    pub async fn directory_songs(&self, id: &str) -> Result<Vec<Child>, CoreError> {
        let mut out = Vec::new();
        let mut stack = vec![(id.to_string(), 0usize)];
        let mut dirs_seen = std::collections::HashSet::new();
        while let Some((dir, depth)) = stack.pop() {
            if !dirs_seen.insert(dir.clone()) || depth > MAX_DEPTH {
                continue;
            }
            let d = self.client.get_music_directory(&dir).await?;
            let mut subdirs = Vec::new();
            for c in d.child {
                if c.is_dir {
                    subdirs.push((c.id, depth + 1));
                } else {
                    out.push(c);
                    if out.len() >= MAX_SONGS {
                        return Ok(out);
                    }
                }
            }
            // Reverse so that popping keeps the server order.
            stack.extend(subdirs.into_iter().rev());
        }
        Ok(out)
    }

    /// All songs of an artist (ID3), album by album.
    pub async fn artist_songs(&self, artist_id: &str) -> Result<Vec<Child>, CoreError> {
        let artist = self.client.get_artist(artist_id).await?;
        let mut out = Vec::new();
        for album in artist.album {
            let a = self.client.get_album(&album.id).await?;
            out.extend(a.song);
            if out.len() >= MAX_SONGS {
                break;
            }
        }
        Ok(out)
    }

    /// Star or unstar an item. Returns the new state.
    pub async fn set_starred(&self, target: &StarTarget, starred: bool) -> Result<bool, CoreError> {
        if starred {
            self.client.star(target).await?;
        } else {
            self.client.unstar(target).await?;
        }
        Ok(starred)
    }

    /// Update the starred flag of queue entries for a song.
    pub fn mark_track_starred(&self, id: &str, starred: bool) {
        let snap = self.snapshot();
        if let Some(e) = snap.queue.iter().find(|e| e.track.id == id) {
            let mut t = e.track.clone();
            t.starred = starred;
            self.send(Command::UpdateTrack(t));
        }
    }

    /// Save the queue and position to the server (`savePlayQueue`).
    pub async fn save_play_queue(&self) -> Result<(), CoreError> {
        let snap = self.snapshot();
        let ids: Vec<String> = snap.queue.iter().map(|e| e.track.id.clone()).collect();
        let current = snap.current.as_ref().map(|e| e.track.id.clone());
        let pos = (snap.position * 1000.0) as u64;
        self.client
            .save_play_queue(&ids, current.as_deref(), Some(pos))
            .await?;
        Ok(())
    }

    /// Restore the queue saved on the server, paused at the saved position.
    /// Returns the number of restored songs.
    pub async fn load_play_queue(&self) -> Result<usize, CoreError> {
        let Some(pq) = self.client.get_play_queue().await? else {
            return Ok(0);
        };
        let tracks: Vec<Track> = pq.entry.iter().map(track_from_child).collect();
        if tracks.is_empty() {
            return Ok(0);
        }
        let start = pq
            .current
            .as_ref()
            .and_then(|c| tracks.iter().position(|t| &t.id == c))
            .unwrap_or(0);
        let n = tracks.len();
        self.send(Command::SetQueue {
            tracks,
            start,
            position: pq.position.unwrap_or(0) as f64 / 1000.0,
            play: false,
        });
        Ok(n)
    }

    /// Save the queue to a playlist, creating it or replacing an existing one.
    pub async fn save_queue_as_playlist(
        &self,
        name: &str,
        overwrite_id: Option<&str>,
    ) -> Result<(), CoreError> {
        let snap = self.snapshot();
        let ids: Vec<String> = snap.queue.iter().map(|e| e.track.id.clone()).collect();
        match overwrite_id {
            Some(id) => self.client.replace_playlist(id, &ids).await?,
            None => {
                self.client.create_playlist(name, &ids).await?;
            }
        }
        Ok(())
    }

    /// Save the queue if configured and stop playback. The engine keeps running until the
    /// session is dropped.
    pub async fn finish(&self) -> Result<(), CoreError> {
        let result = if self.playback.save_queue_on_exit && !self.snapshot().queue.is_empty() {
            self.save_play_queue().await
        } else {
            Ok(())
        };
        self.engine.send(Command::Stop);
        result
    }

    /// Shut down: optionally save the queue, then stop the engine.
    pub async fn shutdown(self) -> Result<(), CoreError> {
        let result = self.finish().await;
        drop(self.engine);
        result
    }
}

/// Download songs into the pinned store; returns the number of failures.
async fn pin_songs(
    provider: &HttpSourceProvider,
    tx: &mpsc::UnboundedSender<SessionEvent>,
    songs: Vec<Child>,
) -> usize {
    let mut failed = 0;
    for song in songs.into_iter().filter(|s| !s.is_dir) {
        let Some(cache) = provider.cache() else {
            return failed + 1;
        };
        if cache.is_pinned(&song.id) {
            continue;
        }
        match provider.pin(&song.id).await {
            Ok(()) => {
                cache.note_pin(&song);
                let _ = tx.send(SessionEvent::Pinned { id: song.id });
            }
            Err(e) => {
                failed += 1;
                let _ = tx.send(SessionEvent::Error(format!(
                    "could not download {}: {e}",
                    song.title
                )));
            }
        }
    }
    failed
}

struct Pump {
    events: crossbeam_channel::Receiver<Event>,
    tx: mpsc::UnboundedSender<SessionEvent>,
    client: Client,
    handle: tokio::runtime::Handle,
    scrobble: bool,
    provider: HttpSourceProvider,
    snapshot: SnapshotReader,
    preload: usize,
}

/// Download the songs after the next one into the cache (the engine opens the next one
/// itself, and that download is cached too).
fn prefetch(p: &Pump, uid: u64) {
    if p.preload == 0 || p.provider.cache().is_none_or(|c| !c.enabled()) {
        return;
    }
    let snap = p.snapshot.get();
    let Some(i) = snap.queue.iter().position(|e| e.uid == uid) else {
        return;
    };
    for e in snap.queue.iter().skip(i + 2).take(p.preload) {
        p.provider.prefetch(&e.track.id);
    }
}

fn spawn_event_pump(p: Pump) {
    let _ = std::thread::Builder::new()
        .name("infrasonic-events".into())
        .spawn(move || {
            let Pump {
                events,
                tx,
                client,
                handle,
                scrobble,
                ..
            } = &p;
            let (events, tx, client, handle, scrobble) = (
                events.clone(),
                tx.clone(),
                client.clone(),
                handle.clone(),
                *scrobble,
            );
            let mut tracker = ScrobbleTracker::new();
            while let Ok(ev) = events.recv() {
                if let Event::TrackChanged(Some(e)) = &ev {
                    prefetch(&p, e.uid);
                }
                let action = match &ev {
                    Event::TrackChanged(entry) => tracker.track_changed(
                        entry.as_ref().map(|e| e.track.id.as_str()),
                        entry.as_ref().and_then(|e| e.track.duration),
                        now_ms(),
                    ),
                    Event::Position { position, .. } => tracker.position(*position),
                    _ => None,
                };
                if scrobble && let Some(action) = action {
                    let client = client.clone();
                    let tx = tx.clone();
                    handle.spawn(async move {
                        let (id, res, submitted) = match action {
                            ScrobbleAction::NowPlaying { id } => {
                                let r = client.scrobble(&id, false, None).await;
                                (id, r, false)
                            }
                            ScrobbleAction::Submit { id, time_ms } => {
                                let r = client.scrobble(&id, true, Some(time_ms)).await;
                                (id, r, true)
                            }
                        };
                        match res {
                            Ok(()) if submitted => {
                                let _ = tx.send(SessionEvent::Scrobbled { id });
                            }
                            Ok(()) => {}
                            Err(e) => {
                                let _ =
                                    tx.send(SessionEvent::Error(format!("scrobble failed: {e}")));
                            }
                        }
                    });
                }
                if tx.send(SessionEvent::Engine(ev)).is_err() {
                    break;
                }
            }
        });
}
