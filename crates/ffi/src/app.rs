// SPDX-License-Identifier: GPL-3.0-only
//! The [`Infrasonic`] object: one connected server and the engine that plays from it.

use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;

use infrasonic_core::cache::AudioCache;
use infrasonic_core::{Session, SessionEvent};
use infrasonic_engine::Command;
use infrasonic_engine::output::{CpalOutputFactory, NullOutputFactory, OutputFactory, OutputSpec};
use infrasonic_subsonic::{self as ss, Client, SearchOptions, StarTarget};
use parking_lot::{Mutex, RwLock};
use tokio::sync::mpsc;

use crate::types::*;

/// Receives playback events. Called on a background thread; keep it short.
#[uniffi::export(with_foreign)]
pub trait PlayerListener: Send + Sync {
    fn on_event(&self, event: PlayerEvent);
}

/// Playback commands.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum PlayerCommand {
    Play,
    Pause,
    TogglePause,
    Stop,
    Next,
    /// Restart the song, or go to the previous one near its start.
    Previous,
    /// Absolute position in seconds.
    Seek {
        seconds: f64,
    },
    SeekBy {
        seconds: f64,
    },
    PlayIndex {
        index: u32,
    },
    Remove {
        index: u32,
    },
    Move {
        from: u32,
        to: u32,
    },
    Clear,
    /// Shuffle the queue; the current song moves to the front.
    Shuffle,
    SetRepeat {
        repeat: Repeat,
    },
    /// 0–100, on top of the system volume.
    SetVolume {
        volume: u8,
    },
}

impl From<PlayerCommand> for Command {
    fn from(c: PlayerCommand) -> Self {
        match c {
            PlayerCommand::Play => Command::Play,
            PlayerCommand::Pause => Command::Pause,
            PlayerCommand::TogglePause => Command::TogglePause,
            PlayerCommand::Stop => Command::Stop,
            PlayerCommand::Next => Command::Next,
            PlayerCommand::Previous => Command::Previous,
            PlayerCommand::Seek { seconds } => Command::Seek(seconds.max(0.0)),
            PlayerCommand::SeekBy { seconds } => Command::SeekBy(seconds),
            PlayerCommand::PlayIndex { index } => Command::PlayIndex(index as usize),
            PlayerCommand::Remove { index } => Command::Remove(index as usize),
            PlayerCommand::Move { from, to } => Command::Move {
                from: from as usize,
                to: to as usize,
            },
            PlayerCommand::Clear => Command::Clear,
            PlayerCommand::Shuffle => Command::Shuffle,
            PlayerCommand::SetRepeat { repeat } => Command::SetRepeat(repeat.into()),
            PlayerCommand::SetVolume { volume } => Command::SetVolume(volume.min(100)),
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Output {
    Device,
    /// Discard audio, `speed` times faster than real time (tests).
    Null(f64),
}

impl Output {
    fn factory(self) -> Box<dyn OutputFactory> {
        match self {
            Output::Device => Box::new(CpalOutputFactory::default()),
            Output::Null(speed) => Box::new(NullOutputFactory::new(
                OutputSpec {
                    sample_rate: 48_000,
                    channels: 2,
                },
                speed,
            )),
        }
    }
}

struct Active {
    profile: ServerProfile,
    session: Arc<Session>,
}

type Listener = Arc<RwLock<Option<Arc<dyn PlayerListener>>>>;

/// The app's connection to infrasonic: server, library and player.
#[derive(uniffi::Object)]
pub struct Infrasonic {
    rt: tokio::runtime::Runtime,
    data_dir: PathBuf,
    cache_dir: PathBuf,
    output: Output,
    active: RwLock<Option<Active>>,
    listener: Listener,
    settings: Mutex<PlaybackSettings>,
}

impl std::fmt::Debug for Infrasonic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Infrasonic")
            .field("data_dir", &self.data_dir)
            .field("cache_dir", &self.cache_dir)
            .field("output", &self.output)
            .finish_non_exhaustive()
    }
}

fn join_error(e: tokio::task::JoinError) -> FfiError {
    FfiError::Other {
        reason: format!("task failed: {e}"),
    }
}

const MIB: u64 = 1024 * 1024;
const SETTINGS_FILE: &str = "settings.json";
const PROFILES_FILE: &str = "profiles.json";

fn read_json<T: serde::de::DeserializeOwned>(path: &std::path::Path) -> Option<T> {
    let bytes = std::fs::read(path).ok()?;
    match serde_json::from_slice(&bytes) {
        Ok(v) => Some(v),
        Err(e) => {
            tracing::warn!("ignoring {}: {e}", path.display());
            None
        }
    }
}

/// Write a file atomically (temporary file + rename), creating its directory.
fn write_json<T: serde::Serialize>(path: &std::path::Path, value: &T) -> Result<()> {
    let io = |e: std::io::Error| FfiError::Other {
        reason: format!("cannot write {}: {e}", path.display()),
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(io)?;
    }
    let tmp = path.with_extension("json.part");
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| FfiError::Other {
        reason: e.to_string(),
    })?;
    std::fs::write(&tmp, bytes).map_err(io)?;
    std::fs::rename(&tmp, path).map_err(io)
}

impl Infrasonic {
    fn build(data_dir: String, cache_dir: String, output: Output) -> Result<Arc<Self>> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("infrasonic-rt")
            .enable_all()
            .build()
            .map_err(|e| FfiError::Other {
                reason: format!("cannot start runtime: {e}"),
            })?;
        let data_dir: PathBuf = data_dir.into();
        let settings = read_json(&data_dir.join(SETTINGS_FILE)).unwrap_or_default();
        Ok(Arc::new(Self {
            rt,
            data_dir,
            cache_dir: cache_dir.into(),
            output,
            active: RwLock::new(None),
            listener: Arc::new(RwLock::new(None)),
            settings: Mutex::new(settings),
        }))
    }

    fn queue_file(&self) -> Option<PathBuf> {
        let key = self.active.read().as_ref()?.profile.key();
        Some(self.data_dir.join("queue").join(format!("{key}.json")))
    }

    /// Run a future on the runtime (reqwest needs Tokio; Kotlin polls the result).
    async fn run<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: Future<Output = Result<T>> + Send + 'static,
    {
        self.rt.spawn(f).await.map_err(join_error)?
    }

    fn session(&self) -> Result<Arc<Session>> {
        self.active
            .read()
            .as_ref()
            .map(|a| a.session.clone())
            .ok_or(FfiError::NotConnected)
    }

    fn client(&self) -> Result<Client> {
        Ok(self.session()?.client().clone())
    }

    /// Run `f` with a client on the runtime.
    async fn with_client<T, F, Fut>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(Client) -> Fut,
        Fut: Future<Output = Result<T>> + Send + 'static,
    {
        let c = self.client()?;
        self.run(f(c)).await
    }

    fn cache(&self) -> Option<Arc<AudioCache>> {
        self.active.read().as_ref().and_then(|a| a.session.cache())
    }

    /// Start a session for `profile` with an already connected (or offline) client.
    fn start(&self, profile: ServerProfile, client: Client, info: ss::ServerInfo) -> ServerStatus {
        let _guard = self.rt.enter();
        let settings = self.settings.lock().clone();
        let status = ServerStatus::from_info(&info);
        let (session, events) = Session::with_client(
            client,
            info,
            &profile.name,
            settings.to_config(),
            self.output.factory(),
        );
        session.set_cache(Some(Arc::new(AudioCache::new(
            &self.cache_dir,
            &self.data_dir,
            &profile.key(),
            settings.cache_mb * MIB,
        ))));
        session.set_pin_bit_rate(
            settings
                .pin_quality
                .as_ref()
                .map(|q| (q.max_bit_rate, q.format.clone())),
        );
        forward_events(events, self.listener.clone());
        let old = self.active.write().replace(Active {
            profile,
            session: Arc::new(session),
        });
        drop(old);
        status
    }
}

/// Deliver session events to the listener from a thread of their own, so that a slow
/// listener never holds up the runtime. Ends when the session is gone.
fn forward_events(mut events: mpsc::UnboundedReceiver<SessionEvent>, listener: Listener) {
    let _ = std::thread::Builder::new()
        .name("infrasonic-listener".into())
        .spawn(move || {
            while let Some(ev) = events.blocking_recv() {
                let l = listener.read().clone();
                if let Some(l) = l {
                    l.on_event(ev.into());
                }
            }
        });
}

#[uniffi::export]
impl Infrasonic {
    /// Play through the device's audio output. `data_dir` keeps pinned songs, `cache_dir`
    /// the song cache.
    #[uniffi::constructor]
    pub fn new(data_dir: String, cache_dir: String) -> Result<Arc<Self>> {
        Self::build(data_dir, cache_dir, Output::Device)
    }

    /// Discard the audio, `speed` times faster than real time (for tests).
    #[uniffi::constructor]
    pub fn with_null_output(data_dir: String, cache_dir: String, speed: f64) -> Result<Arc<Self>> {
        Self::build(data_dir, cache_dir, Output::Null(speed.max(0.01)))
    }

    pub fn set_listener(&self, listener: Option<Arc<dyn PlayerListener>>) {
        *self.listener.write() = listener;
    }

    pub fn settings(&self) -> PlaybackSettings {
        self.settings.lock().clone()
    }

    /// Change and save settings. Quality, ReplayGain and the cache size apply right away;
    /// scrobbling and preloading from the next connection on.
    pub fn set_settings(&self, settings: PlaybackSettings) {
        let old = std::mem::replace(&mut *self.settings.lock(), settings.clone());
        if let Err(e) = write_json(&self.data_dir.join(SETTINGS_FILE), &settings) {
            tracing::warn!("{e}");
        }
        let Ok(session) = self.session() else {
            return;
        };
        if old.quality != settings.quality {
            session.set_max_bit_rate(
                settings.quality.max_bit_rate,
                settings.quality.format.clone(),
            );
        }
        if old.pin_quality != settings.pin_quality {
            session.set_pin_bit_rate(
                settings
                    .pin_quality
                    .as_ref()
                    .map(|q| (q.max_bit_rate, q.format.clone())),
            );
        }
        if old.replay_gain != settings.replay_gain
            || old.replay_gain_preamp != settings.replay_gain_preamp
            || old.replay_gain_fallback != settings.replay_gain_fallback
            || old.prevent_clipping != settings.prevent_clipping
        {
            let cfg = settings.to_config();
            session.send(Command::SetReplayGain(cfg.replay_gain_config()));
        }
        if old.cache_mb != settings.cache_mb
            && let Some(active) = self.active.read().as_ref()
        {
            let cache = AudioCache::new(
                &self.cache_dir,
                &self.data_dir,
                &active.profile.key(),
                settings.cache_mb * MIB,
            );
            cache.evict();
            active.session.set_cache(Some(Arc::new(cache)));
        }
    }

    /// The saved servers.
    pub fn profiles(&self) -> Vec<ServerProfile> {
        read_json::<StoredProfiles>(&self.data_dir.join(PROFILES_FILE))
            .unwrap_or_default()
            .list
    }

    /// Id of the server to connect to on start.
    pub fn active_profile_id(&self) -> Option<String> {
        read_json::<StoredProfiles>(&self.data_dir.join(PROFILES_FILE))
            .unwrap_or_default()
            .active
    }

    /// Save the servers and the one in use.
    pub fn save_profiles(&self, list: Vec<ServerProfile>, active: Option<String>) -> Result<()> {
        write_json(
            &self.data_dir.join(PROFILES_FILE),
            &StoredProfiles { active, list },
        )
    }

    /// Check a server without connecting the player.
    pub async fn test_connection(&self, profile: ServerProfile) -> Result<ServerStatus> {
        let client = profile.client()?;
        self.run(async move { Ok(ServerStatus::from_info(&client.connect().await?)) })
            .await
    }

    /// Connect to a server; replaces the current session (the queue starts empty).
    pub async fn connect(&self, profile: ServerProfile) -> Result<ServerStatus> {
        let client = profile.client()?;
        let c = client.clone();
        let info = self.run(async move { Ok(c.connect().await?) }).await?;
        Ok(self.start(profile, client, info))
    }

    /// Use a server without reaching it, for pinned songs. Library calls still try the server.
    pub fn connect_offline(&self, profile: ServerProfile) -> Result<ServerStatus> {
        let client = profile.client()?;
        Ok(self.start(profile, client, ss::ServerInfo::default()))
    }

    /// Stop playback and close the session.
    pub fn disconnect(&self) {
        let old = self.active.write().take();
        if let Some(a) = old {
            a.session.send(Command::Stop);
        }
    }

    /// Id of the connected server profile.
    pub fn connected_profile(&self) -> Option<String> {
        self.active.read().as_ref().map(|a| a.profile.id.clone())
    }

    /// Whether the session reached the server when it started.
    pub fn is_online(&self) -> bool {
        self.session().map(|s| s.is_online()).unwrap_or(false)
    }

    // --- Playback ---

    /// Replace the queue and start playing at `start`.
    pub fn play(&self, songs: Vec<Song>, start: u32) -> Result<()> {
        self.session()?
            .play_songs(&children(&songs), start as usize);
        Ok(())
    }

    /// Replace the queue without starting playback (e.g. to restore it).
    pub fn set_queue(&self, songs: Vec<Song>, start: u32, position: f64) -> Result<()> {
        let tracks: Vec<_> = children(&songs)
            .iter()
            .map(infrasonic_core::track_from_child)
            .collect();
        self.session()?.send(Command::SetQueue {
            start: (start as usize).min(tracks.len().saturating_sub(1)),
            tracks,
            position,
            play: false,
        });
        Ok(())
    }

    pub fn enqueue(&self, songs: Vec<Song>) -> Result<()> {
        self.session()?.enqueue(&children(&songs));
        Ok(())
    }

    /// Insert after the current song.
    pub fn enqueue_next(&self, songs: Vec<Song>) -> Result<()> {
        self.session()?.enqueue_next(&children(&songs));
        Ok(())
    }

    pub fn command(&self, command: PlayerCommand) -> Result<()> {
        self.session()?.send(command.into());
        Ok(())
    }

    pub fn state(&self) -> Option<PlayerState> {
        self.session()
            .ok()
            .map(|s| PlayerState::from(&s.snapshot()))
    }

    pub fn queue(&self) -> Vec<QueueItem> {
        self.session()
            .map(|s| s.snapshot().queue.iter().map(QueueItem::from).collect())
            .unwrap_or_default()
    }

    /// Save the queue and position on the device (for resuming playback later).
    pub fn save_local_queue(&self) -> Result<()> {
        let s = self.session()?;
        let Some(path) = self.queue_file() else {
            return Err(FfiError::NotConnected);
        };
        let snap = s.snapshot();
        let stored = StoredQueue {
            songs: snap
                .queue
                .iter()
                .map(|e| ss::Child::from(&Song::from(&e.track)))
                .collect(),
            index: snap.current_index.unwrap_or(0) as u32,
            position: snap.position,
        };
        write_json(&path, &stored)
    }

    /// The queue last saved on the device for the connected server.
    pub fn local_queue(&self) -> Option<SavedQueue> {
        let q: StoredQueue = read_json(&self.queue_file()?)?;
        (!q.songs.is_empty()).then(|| SavedQueue {
            songs: songs(&q.songs),
            index: q.index,
            position: q.position,
        })
    }

    /// Save the queue and position on the server.
    pub async fn save_server_queue(&self) -> Result<()> {
        let s = self.session()?;
        self.run(async move { Ok(s.save_play_queue().await?) })
            .await
    }

    /// Restore the queue saved on the server (paused). Returns the number of songs.
    pub async fn load_server_queue(&self) -> Result<u32> {
        let s = self.session()?;
        self.run(async move { Ok(s.load_play_queue().await? as u32) })
            .await
    }

    /// Save the queue as a new playlist, or replace the songs of `overwrite_id`.
    pub async fn save_queue_as_playlist(
        &self,
        name: String,
        overwrite_id: Option<String>,
    ) -> Result<()> {
        let s = self.session()?;
        self.run(async move {
            Ok(s.save_queue_as_playlist(&name, overwrite_id.as_deref())
                .await?)
        })
        .await
    }

    // --- Library ---

    pub async fn artists(&self) -> Result<Vec<ArtistGroup>> {
        self.with_client(|c| async move {
            Ok(c.get_artists()
                .await?
                .index
                .iter()
                .map(|i| ArtistGroup {
                    name: i.name.clone(),
                    artists: i.artist.iter().map(Artist::from).collect(),
                })
                .collect())
        })
        .await
    }

    /// An artist with its albums.
    pub async fn artist(&self, id: String) -> Result<Artist> {
        self.with_client(|c| async move { Ok(Artist::from(&c.get_artist(&id).await?)) })
            .await
    }

    pub async fn artist_info(&self, id: String) -> Result<ArtistInfo> {
        self.with_client(|c| async move {
            let i = c.get_artist_info2(&id, 10).await?;
            Ok(ArtistInfo {
                biography: i.biography.filter(|b| !b.trim().is_empty()),
                image_url: i
                    .large_image_url
                    .or(i.medium_image_url)
                    .filter(|u| !u.is_empty()),
                last_fm_url: i.last_fm_url,
                similar: i.similar_artist.iter().map(Artist::from).collect(),
            })
        })
        .await
    }

    /// All songs of an artist, album by album.
    pub async fn artist_songs(&self, id: String) -> Result<Vec<Song>> {
        let s = self.session()?;
        self.run(async move { Ok(songs(&s.artist_songs(&id).await?)) })
            .await
    }

    /// Most played songs of an artist (by name, from last.fm via the server).
    pub async fn top_songs(&self, artist: String, count: u32) -> Result<Vec<Song>> {
        self.with_client(|c| async move { Ok(songs(&c.get_top_songs(&artist, count).await?)) })
            .await
    }

    /// Songs similar to an artist ("artist radio").
    pub async fn artist_radio(&self, artist_id: String, count: u32) -> Result<Vec<Song>> {
        self.with_client(
            |c| async move { Ok(songs(&c.get_similar_songs2(&artist_id, count).await?)) },
        )
        .await
    }

    /// Songs similar to a song ("song radio").
    pub async fn song_radio(&self, song_id: String, count: u32) -> Result<Vec<Song>> {
        self.with_client(|c| async move { Ok(songs(&c.get_similar_songs(&song_id, count).await?)) })
            .await
    }

    /// An album with its songs.
    pub async fn album(&self, id: String) -> Result<Album> {
        self.with_client(|c| async move { Ok(Album::from(&c.get_album(&id).await?)) })
            .await
    }

    pub async fn album_list(
        &self,
        kind: AlbumListKind,
        size: u32,
        offset: u32,
    ) -> Result<Vec<Album>> {
        use ss::AlbumListType as T;
        self.with_client(|c| async move {
            let list = match kind {
                AlbumListKind::ByGenre { genre } => {
                    c.get_albums_by_genre(&genre, size, offset).await?
                }
                k => {
                    let t = match k {
                        AlbumListKind::Random => T::Random,
                        AlbumListKind::Newest => T::Newest,
                        AlbumListKind::Highest => T::Highest,
                        AlbumListKind::Frequent => T::Frequent,
                        AlbumListKind::Recent => T::Recent,
                        AlbumListKind::ByName => T::AlphabeticalByName,
                        AlbumListKind::ByArtist => T::AlphabeticalByArtist,
                        AlbumListKind::Starred => T::Starred,
                        AlbumListKind::ByYear { from, to } => T::ByYear { from, to },
                        AlbumListKind::ByGenre { .. } => unreachable!("handled above"),
                    };
                    c.get_album_list2(t, size, offset).await?
                }
            };
            Ok(list.iter().map(Album::from).collect())
        })
        .await
    }

    pub async fn song(&self, id: String) -> Result<Song> {
        self.with_client(|c| async move { Ok(Song::from(&c.get_song(&id).await?)) })
            .await
    }

    pub async fn random_songs(&self, size: u32, genre: Option<String>) -> Result<Vec<Song>> {
        self.with_client(|c| async move {
            Ok(songs(&c.get_random_songs(size, genre.as_deref()).await?))
        })
        .await
    }

    pub async fn search(&self, query: String, count: u32, offset: u32) -> Result<SearchResult> {
        self.with_client(|c| async move {
            let r = c
                .search3(
                    &query,
                    SearchOptions {
                        artist_count: count,
                        artist_offset: offset,
                        album_count: count,
                        album_offset: offset,
                        song_count: count,
                        song_offset: offset,
                    },
                )
                .await?;
            Ok(SearchResult {
                artists: r.artist.iter().map(Artist::from).collect(),
                albums: r.album.iter().map(Album::from).collect(),
                songs: songs(&r.song),
            })
        })
        .await
    }

    pub async fn starred(&self) -> Result<SearchResult> {
        self.with_client(|c| async move {
            let r = c.get_starred2().await?;
            Ok(SearchResult {
                artists: r.artist.iter().map(Artist::from).collect(),
                albums: r.album.iter().map(Album::from).collect(),
                songs: songs(&r.song),
            })
        })
        .await
    }

    pub async fn genres(&self) -> Result<Vec<Genre>> {
        self.with_client(|c| async move {
            let mut g: Vec<Genre> = c
                .get_genres()
                .await?
                .into_iter()
                .map(|g| Genre {
                    name: g.value,
                    song_count: g.song_count,
                    album_count: g.album_count,
                })
                .collect();
            g.sort_by_key(|g| g.name.to_lowercase());
            Ok(g)
        })
        .await
    }

    pub async fn songs_by_genre(
        &self,
        genre: String,
        count: u32,
        offset: u32,
    ) -> Result<Vec<Song>> {
        self.with_client(|c| async move {
            Ok(songs(&c.get_songs_by_genre(&genre, count, offset).await?))
        })
        .await
    }

    /// A folder of the file view; `None` lists the top level.
    pub async fn folder(&self, id: Option<String>) -> Result<Folder> {
        self.with_client(|c| async move {
            Ok(match id {
                None => {
                    let i = c.get_indexes(None).await?;
                    Folder {
                        id: String::new(),
                        name: String::new(),
                        parent: None,
                        folders: i
                            .index
                            .iter()
                            .flat_map(|x| &x.artist)
                            .map(|a| FolderEntry {
                                id: a.id.clone(),
                                name: a.name.clone(),
                                cover_art: a.cover_art.clone(),
                            })
                            .collect(),
                        songs: songs(&i.child),
                    }
                }
                Some(id) => {
                    let d = c.get_music_directory(&id).await?;
                    Folder {
                        id: d.id,
                        name: d.name,
                        parent: d.parent,
                        folders: d
                            .child
                            .iter()
                            .filter(|c| c.is_dir)
                            .map(|c| FolderEntry {
                                id: c.id.clone(),
                                name: c.title.clone(),
                                cover_art: c.cover_art.clone(),
                            })
                            .collect(),
                        songs: songs(&d.child),
                    }
                }
            })
        })
        .await
    }

    /// All songs below a folder.
    pub async fn folder_songs(&self, id: String) -> Result<Vec<Song>> {
        let s = self.session()?;
        self.run(async move { Ok(songs(&s.directory_songs(&id).await?)) })
            .await
    }

    pub async fn set_starred(&self, kind: ItemKind, id: String, starred: bool) -> Result<()> {
        let s = self.session()?;
        let target = match kind {
            ItemKind::Song => StarTarget::id(id.clone()),
            ItemKind::Album => StarTarget::album(id.clone()),
            ItemKind::Artist => StarTarget::artist(id.clone()),
        };
        let s2 = s.clone();
        self.run(async move { Ok(s2.set_starred(&target, starred).await?) })
            .await?;
        if kind == ItemKind::Song {
            s.mark_track_starred(&id, starred);
        }
        Ok(())
    }

    /// Rate 1–5, or 0 to remove the rating.
    pub async fn set_rating(&self, id: String, rating: u8) -> Result<()> {
        self.with_client(|c| async move { Ok(c.set_rating(&id, rating.min(5)).await?) })
            .await
    }

    /// Lyrics of a song: synchronised from OpenSubsonic servers where available, otherwise
    /// plain text looked up by artist and title.
    pub async fn lyrics(&self, song: Song) -> Result<Option<Lyrics>> {
        self.with_client(|c| async move {
            if c.supports("songLyrics") {
                let list = c.get_lyrics_by_song_id(&song.id).await?;
                let best = list
                    .iter()
                    .find(|l| l.synced && !l.line.is_empty())
                    .or_else(|| list.iter().find(|l| !l.line.is_empty()));
                if let Some(l) = best {
                    let offset = l.offset.unwrap_or(0);
                    return Ok(Some(Lyrics {
                        synced: l.synced,
                        lang: Some(l.lang.clone()).filter(|s| !s.is_empty() && s != "xxx"),
                        lines: l
                            .line
                            .iter()
                            .map(|x| LyricsLine {
                                start_ms: x.start.filter(|_| l.synced).map(|s| s + offset),
                                text: x.value.clone(),
                            })
                            .collect(),
                    }));
                }
            }
            if song.artist.is_empty() || song.title.is_empty() {
                return Ok(None);
            }
            let l = c.get_lyrics(&song.artist, &song.title).await?;
            let text = l.value.trim();
            Ok((!text.is_empty()).then(|| Lyrics {
                synced: false,
                lang: None,
                lines: text
                    .lines()
                    .map(|t| LyricsLine {
                        start_ms: None,
                        text: t.trim_end().to_string(),
                    })
                    .collect(),
            }))
        })
        .await
    }

    // --- Playlists ---

    pub async fn playlists(&self) -> Result<Vec<Playlist>> {
        self.with_client(|c| async move {
            Ok(c.get_playlists()
                .await?
                .iter()
                .map(Playlist::from)
                .collect())
        })
        .await
    }

    /// A playlist with its songs.
    pub async fn playlist(&self, id: String) -> Result<Playlist> {
        self.with_client(|c| async move { Ok(Playlist::from(&c.get_playlist(&id).await?)) })
            .await
    }

    pub async fn create_playlist(&self, name: String, song_ids: Vec<String>) -> Result<()> {
        self.with_client(|c| async move {
            c.create_playlist(&name, &song_ids).await?;
            Ok(())
        })
        .await
    }

    pub async fn add_to_playlist(&self, id: String, song_ids: Vec<String>) -> Result<()> {
        self.with_client(|c| async move {
            Ok(c.update_playlist(
                &id,
                &ss::PlaylistUpdate {
                    add_song_ids: song_ids,
                    ..ss::PlaylistUpdate::default()
                },
            )
            .await?)
        })
        .await
    }

    /// Remove the songs at these positions.
    pub async fn remove_from_playlist(&self, id: String, indexes: Vec<u32>) -> Result<()> {
        self.with_client(|c| async move {
            Ok(c.update_playlist(
                &id,
                &ss::PlaylistUpdate {
                    remove_indexes: indexes.into_iter().map(|i| i as usize).collect(),
                    ..ss::PlaylistUpdate::default()
                },
            )
            .await?)
        })
        .await
    }

    pub async fn rename_playlist(
        &self,
        id: String,
        name: String,
        comment: Option<String>,
    ) -> Result<()> {
        self.with_client(|c| async move {
            Ok(c.update_playlist(
                &id,
                &ss::PlaylistUpdate {
                    name: Some(name),
                    comment,
                    ..ss::PlaylistUpdate::default()
                },
            )
            .await?)
        })
        .await
    }

    pub async fn delete_playlist(&self, id: String) -> Result<()> {
        self.with_client(|c| async move { Ok(c.delete_playlist(&id).await?) })
            .await
    }

    // --- Bookmarks, shares, radio ---

    pub async fn bookmarks(&self) -> Result<Vec<Bookmark>> {
        self.with_client(|c| async move {
            Ok(c.get_bookmarks()
                .await?
                .iter()
                .map(|b| Bookmark {
                    song: Song::from(&b.entry),
                    position_ms: b.position,
                    comment: b.comment.clone(),
                    changed: b.changed.clone(),
                })
                .collect())
        })
        .await
    }

    pub async fn create_bookmark(
        &self,
        song_id: String,
        position_ms: u64,
        comment: Option<String>,
    ) -> Result<()> {
        self.with_client(|c| async move {
            Ok(c.create_bookmark(&song_id, position_ms, comment.as_deref())
                .await?)
        })
        .await
    }

    pub async fn delete_bookmark(&self, song_id: String) -> Result<()> {
        self.with_client(|c| async move { Ok(c.delete_bookmark(&song_id).await?) })
            .await
    }

    pub async fn shares(&self) -> Result<Vec<Share>> {
        self.with_client(
            |c| async move { Ok(c.get_shares().await?.iter().map(Share::from).collect()) },
        )
        .await
    }

    /// Share songs, an album or a playlist. `expires_ms` is a Unix time in milliseconds.
    pub async fn create_share(
        &self,
        ids: Vec<String>,
        description: Option<String>,
        expires_ms: Option<u64>,
    ) -> Result<Share> {
        self.with_client(|c| async move {
            let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
            Ok(Share::from(
                &c.create_share(&ids, description.as_deref(), expires_ms)
                    .await?,
            ))
        })
        .await
    }

    pub async fn delete_share(&self, id: String) -> Result<()> {
        self.with_client(|c| async move { Ok(c.delete_share(&id).await?) })
            .await
    }

    pub async fn radio_stations(&self) -> Result<Vec<RadioStation>> {
        self.with_client(|c| async move {
            Ok(c.get_internet_radio_stations()
                .await?
                .into_iter()
                .map(|r| RadioStation {
                    id: r.id,
                    name: r.name,
                    stream_url: r.stream_url,
                    home_page_url: r.home_page_url,
                })
                .collect())
        })
        .await
    }

    // --- Podcasts ---

    /// Podcast channels, with episodes if `include_episodes`.
    pub async fn podcasts(&self, include_episodes: bool) -> Result<Vec<PodcastChannel>> {
        self.with_client(|c| async move {
            Ok(c.get_podcasts(include_episodes, None)
                .await?
                .iter()
                .map(PodcastChannel::from)
                .collect())
        })
        .await
    }

    /// One channel with its episodes.
    pub async fn podcast(&self, id: String) -> Result<PodcastChannel> {
        self.with_client(|c| async move {
            c.get_podcasts(true, Some(&id))
                .await?
                .first()
                .map(PodcastChannel::from)
                .ok_or(FfiError::Server {
                    code: 70,
                    reason: "podcast not found".into(),
                })
        })
        .await
    }

    pub async fn newest_episodes(&self, count: u32) -> Result<Vec<PodcastEpisode>> {
        self.with_client(|c| async move {
            Ok(c.get_newest_podcasts(count)
                .await?
                .iter()
                .map(PodcastEpisode::from)
                .collect())
        })
        .await
    }

    pub async fn add_podcast(&self, url: String) -> Result<()> {
        self.with_client(|c| async move { Ok(c.create_podcast_channel(&url).await?) })
            .await
    }

    pub async fn delete_podcast(&self, id: String) -> Result<()> {
        self.with_client(|c| async move { Ok(c.delete_podcast_channel(&id).await?) })
            .await
    }

    /// Ask the server to download an episode.
    pub async fn download_episode(&self, id: String) -> Result<()> {
        self.with_client(|c| async move { Ok(c.download_podcast_episode(&id).await?) })
            .await
    }

    pub async fn refresh_podcasts(&self) -> Result<()> {
        self.with_client(|c| async move { Ok(c.refresh_podcasts().await?) })
            .await
    }

    // --- Jukebox ---

    /// Control playback on the server's own audio output.
    pub async fn jukebox(&self, command: JukeboxCommand) -> Result<JukeboxState> {
        self.with_client(
            |c| async move { Ok(JukeboxState::from(&c.jukebox(&command.into()).await?)) },
        )
        .await
    }

    // --- Server ---

    /// Start a library scan; returns whether a scan is running.
    pub async fn start_scan(&self) -> Result<bool> {
        self.with_client(|c| async move { Ok(c.start_scan().await?.scanning) })
            .await
    }

    pub async fn scan_running(&self) -> Result<bool> {
        self.with_client(|c| async move { Ok(c.get_scan_status().await?.scanning) })
            .await
    }

    /// Cover art image bytes; `size` in pixels (the server scales).
    pub async fn cover_art(&self, id: String, size: Option<u32>) -> Result<Vec<u8>> {
        self.with_client(|c| async move { Ok(c.get_cover_art(&id, size).await?) })
            .await
    }

    /// URL of a song's file including login data, for "open with" or sharing outside the app.
    pub fn download_url(&self, id: String) -> Result<String> {
        Ok(self.client()?.download_url(&id).to_string())
    }

    // --- Offline ---

    /// Download songs for offline use in the background; each arrives as [`PlayerEvent::Pinned`].
    pub fn pin(&self, songs: Vec<Song>) -> Result<()> {
        let s = self.session()?;
        let _guard = self.rt.enter();
        s.pin(children(&songs));
        Ok(())
    }

    /// Download songs for offline use and wait (for background jobs). Returns how many
    /// failed.
    pub async fn pin_and_wait(&self, songs: Vec<Song>) -> Result<u32> {
        let s = self.session()?;
        let c = children(&songs);
        self.run(async move { Ok(s.pin_and_wait(c).await as u32) })
            .await
    }

    pub fn unpin(&self, ids: Vec<String>) -> Result<()> {
        self.session()?.unpin(&ids);
        Ok(())
    }

    /// Pinned songs of the connected server, in the order they were pinned.
    pub fn pinned(&self) -> Vec<Song> {
        self.cache().map(|c| songs(&c.pinned())).unwrap_or_default()
    }

    pub fn is_pinned(&self, id: String) -> bool {
        self.cache().is_some_and(|c| c.is_pinned(&id))
    }

    pub fn cache_usage(&self) -> CacheUsage {
        let Some(c) = self.cache() else {
            return CacheUsage {
                cached_files: 0,
                cached_bytes: 0,
                pinned_files: 0,
                pinned_bytes: 0,
            };
        };
        let (u, p) = (c.usage(), c.pinned_usage());
        CacheUsage {
            cached_files: u.files,
            cached_bytes: u.bytes,
            pinned_files: p.files,
            pinned_bytes: p.bytes,
        }
    }

    /// Delete the cached (not pinned) songs.
    pub fn clear_cache(&self) -> Result<()> {
        if let Some(c) = self.cache() {
            c.clear().map_err(|e| FfiError::Other {
                reason: e.to_string(),
            })?;
        }
        Ok(())
    }
}
