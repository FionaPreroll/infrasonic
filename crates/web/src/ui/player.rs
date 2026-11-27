// SPDX-License-Identifier: GPL-3.0-only
//! Playback for the page. Two backends produce the same [`Snapshot`]:
//!
//! - the engine in a Web Worker with an AudioWorklet: gapless, exact ReplayGain;
//! - an `<audio>` element: the browser decodes, for browsers without AudioWorklet, for
//!   iOS (which stops WebAudio in the background) and for internet radio streams.

use std::collections::HashMap;
use std::sync::Arc;

use infrasonic_engine::{
    Command, Event, PlayState, Queue, QueueEntry, ReplayGainConfig, Snapshot, Track, volume_to_gain,
};
use infrasonic_subsonic::{Child, Client, StreamOptions};
use js_sys::{Array, Object, Reflect};
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    AudioContext, AudioWorkletNode, AudioWorkletNodeOptions, HtmlAudioElement, MessageChannel,
    MessageEvent, Worker, WorkerOptions, WorkerType,
};

use crate::protocol::{FromWorker, ToWorker, parse};

/// Which backend plays songs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    Engine,
    Element,
}

impl BackendKind {
    /// Whether the browser can run the engine: AudioWorklet needs a secure context (HTTPS or
    /// localhost), so a player opened over plain HTTP in a home network cannot.
    pub fn engine_available() -> bool {
        let Some(w) = web_sys::window() else {
            return false;
        };
        // Some browsers define `AudioWorkletNode` even where `audioWorklet` is missing.
        w.is_secure_context()
            && Reflect::get(&w, &"AudioWorkletNode".into()).is_ok_and(|v| !v.is_undefined())
    }

    /// The engine where the browser can run it well; the `<audio>` element on iOS.
    pub fn auto() -> Self {
        let ua = web_sys::window()
            .and_then(|w| w.navigator().user_agent().ok())
            .unwrap_or_default();
        let ios = ua.contains("iPhone") || ua.contains("iPad");
        if Self::engine_available() && !ios {
            Self::Engine
        } else {
            Self::Element
        }
    }
}

/// Settings that shape what the backends request and play.
#[derive(Debug, Clone, PartialEq)]
pub struct PlaybackSettings {
    pub max_bit_rate: u32,
    pub format: Option<String>,
    pub replay_gain: ReplayGainConfig,
}

/// The playback state shown by the page; cheap to copy into closures.
#[derive(Debug, Clone, Copy)]
pub struct Player {
    pub snapshot: RwSignal<Snapshot>,
    pub queue: RwSignal<Vec<QueueEntry>>,
    /// Errors and notes for the status line.
    pub message: RwSignal<Option<String>>,
    /// Which backend is active.
    pub active: RwSignal<Option<BackendKind>>,
    /// Songs downloaded by the worker / still loading.
    pub cache: RwSignal<(usize, usize)>,
    pub preferred: RwSignal<BackendKind>,
    pub settings: RwSignal<PlaybackSettings>,
    client: StoredValue<Option<Client>, LocalStorage>,
    engine: StoredValue<Option<EngineBackend>, LocalStorage>,
    element: StoredValue<Option<ElementBackend>, LocalStorage>,
}

struct EngineBackend {
    worker: Worker,
    ctx: AudioContext,
    ready: bool,
    pending: Vec<ToWorker>,
    _node: Option<AudioWorkletNode>,
    _on_message: Closure<dyn FnMut(MessageEvent)>,
}

struct ElementBackend {
    audio: HtmlAudioElement,
    queue: Queue,
    urls: HashMap<String, String>,
    volume: u8,
    state: PlayState,
    _listeners: Vec<Closure<dyn FnMut()>>,
}

/// A Subsonic song as an engine track; `format` is the transcoding target, if any.
pub fn track_from_child(c: &Child, format: Option<&str>) -> Track {
    let rg = c.replay_gain.as_ref().and_then(|r| {
        let rg = infrasonic_engine::ReplayGain {
            track_gain: r.track_gain.or(r.fallback_gain),
            track_peak: r.track_peak.filter(|p| *p > 0.0),
            album_gain: r.album_gain.or(r.fallback_gain),
            album_peak: r.album_peak.filter(|p| *p > 0.0),
        };
        (!rg.is_empty()).then_some(rg)
    });
    Track {
        id: c.id.clone(),
        title: c.title.clone(),
        artist: c.artist_name().to_string(),
        album: c.album.clone().unwrap_or_default(),
        duration: c.duration.map(f64::from),
        suffix: format
            .map(str::to_string)
            .or_else(|| c.transcoded_suffix.clone())
            .or_else(|| c.suffix.clone()),
        content_type: c.content_type.clone(),
        cover_art: c.cover_art.clone(),
        track_number: c.track,
        disc_number: c.disc_number,
        year: c.year,
        genre: c.genre.clone(),
        bit_rate: c.bit_rate,
        album_id: c.album_id.clone(),
        artist_id: c.artist_id.clone(),
        starred: c.is_starred(),
        rating: c.user_rating.filter(|r| *r > 0),
        replay_gain: rg,
    }
}

fn js_err(e: JsValue) -> String {
    e.as_string()
        .or_else(|| {
            Reflect::get(&e, &"message".into())
                .ok()
                .and_then(|m| m.as_string())
        })
        .unwrap_or_else(|| format!("{e:?}"))
}

impl Player {
    pub fn new(preferred: BackendKind, settings: PlaybackSettings) -> Self {
        Self {
            snapshot: RwSignal::new(Snapshot {
                volume: 80,
                ..Snapshot::default()
            }),
            queue: RwSignal::new(Vec::new()),
            message: RwSignal::new(None),
            active: RwSignal::new(None),
            cache: RwSignal::new((0, 0)),
            preferred: RwSignal::new(preferred),
            settings: RwSignal::new(settings),
            client: StoredValue::new_local(None),
            engine: StoredValue::new_local(None),
            element: StoredValue::new_local(None),
        }
    }

    pub fn set_client(&self, c: Option<Client>) {
        self.client.set_value(c);
    }

    fn url(&self, id: &str) -> Option<String> {
        let s = self.settings.get_untracked();
        self.client.with_value(|c| {
            c.as_ref().map(|c| {
                c.stream_url(
                    id,
                    &StreamOptions {
                        max_bit_rate: (s.max_bit_rate > 0).then_some(s.max_bit_rate),
                        format: s.format.clone(),
                        ..StreamOptions::default()
                    },
                )
                .to_string()
            })
        })
    }

    fn tracks(&self, songs: &[Child]) -> (Vec<Track>, Vec<(String, String)>) {
        let format = self.settings.get_untracked().format;
        let tracks: Vec<Track> = songs
            .iter()
            .map(|c| track_from_child(c, format.as_deref()))
            .collect();
        let urls = tracks
            .iter()
            .filter_map(|t| self.url(&t.id).map(|u| (t.id.clone(), u)))
            .collect();
        (tracks, urls)
    }

    /// Replace the queue with `songs` and play from `start`.
    pub fn play_songs(&self, songs: &[Child], start: usize) {
        self.play_tracks(songs, start, 0.0, true);
    }

    /// Replace the queue; `play` false only restores it (e.g. the server queue).
    pub fn play_tracks(&self, songs: &[Child], start: usize, position: f64, play: bool) {
        if songs.is_empty() {
            return;
        }
        let (tracks, urls) = self.tracks(songs);
        self.switch_to(self.preferred.get_untracked());
        self.send_urls(urls);
        self.send(Command::SetQueue {
            tracks,
            start,
            position,
            play,
        });
    }

    pub fn enqueue(&self, songs: &[Child]) {
        self.add(songs, false);
    }

    pub fn play_next(&self, songs: &[Child]) {
        self.add(songs, true);
    }

    fn add(&self, songs: &[Child], next: bool) {
        if songs.is_empty() {
            return;
        }
        if self.active.get_untracked().is_none() || self.queue.with_untracked(|q| q.is_empty()) {
            self.play_tracks(songs, 0, 0.0, true);
            return;
        }
        let (tracks, urls) = self.tracks(songs);
        self.send_urls(urls);
        self.send(if next {
            Command::InsertNext(tracks)
        } else {
            Command::Append(tracks)
        });
    }

    /// Play an internet radio stream (always through the `<audio>` element).
    pub fn play_radio(&self, id: &str, name: &str, url: &str) {
        let mut t = Track::new(format!("radio:{id}"), name);
        t.artist = "Radio".into();
        self.switch_to(BackendKind::Element);
        self.send_urls(vec![(t.id.clone(), url.to_string())]);
        self.send(Command::SetQueue {
            tracks: vec![t],
            start: 0,
            position: 0.0,
            play: true,
        });
    }

    /// Send a command to the active backend.
    pub fn send(&self, cmd: Command) {
        match self.active.get_untracked() {
            Some(BackendKind::Engine) => self.engine_send(ToWorker::Command { cmd }),
            Some(BackendKind::Element) => self.element_command(cmd),
            None => {
                // Nothing played yet; remember volume and repeat for later.
                self.snapshot.update(|s| match cmd {
                    Command::SetVolume(v) => s.volume = v.min(100),
                    Command::SetRepeat(r) => s.repeat = r,
                    _ => {}
                });
            }
        }
    }

    fn send_urls(&self, urls: Vec<(String, String)>) {
        match self.active.get_untracked() {
            Some(BackendKind::Engine) => self.engine_send(ToWorker::Urls { urls }),
            Some(BackendKind::Element) => self.element.update_value(|e| {
                if let Some(e) = e {
                    e.urls.extend(urls);
                }
            }),
            None => {}
        }
    }

    pub fn set_replay_gain(&self, rg: ReplayGainConfig) {
        self.settings.update(|s| s.replay_gain = rg);
        if self.active.get_untracked() == Some(BackendKind::Engine) {
            self.engine_send(ToWorker::Command {
                cmd: Command::SetReplayGain(rg),
            });
        }
    }

    /// Make `kind` the active backend, stopping the other one.
    fn switch_to(&self, kind: BackendKind) {
        let kind = if kind == BackendKind::Engine && !BackendKind::engine_available() {
            BackendKind::Element
        } else {
            kind
        };
        let current = self.active.get_untracked();
        if current == Some(kind) {
            return;
        }
        match current {
            Some(BackendKind::Engine) => self.engine_send(ToWorker::Command {
                cmd: Command::Clear,
            }),
            Some(BackendKind::Element) => self.element_command(Command::Clear),
            None => {}
        }
        let volume = self.snapshot.get_untracked().volume;
        let repeat = self.snapshot.get_untracked().repeat;
        match kind {
            BackendKind::Engine => self.start_engine(volume),
            BackendKind::Element => self.start_element(volume),
        }
        self.active.set(Some(kind));
        self.send(Command::SetRepeat(repeat));
    }

    // --- engine backend ---------------------------------------------------------------

    fn engine_send(&self, m: ToWorker) {
        self.engine.update_value(|e| {
            let Some(e) = e else {
                return;
            };
            if e.ready {
                let _ = e.worker.post_message(&JsValue::from_str(&m.to_json()));
            } else {
                e.pending.push(m);
            }
            // The context starts suspended until a user gesture; every command is one.
            let _ = e.ctx.resume();
        });
    }

    fn start_engine(&self, volume: u8) {
        if self.engine.with_value(|e| e.is_some()) {
            return;
        }
        // Created inside the click handler, so the browser allows audio.
        let ctx = match AudioContext::new() {
            Ok(c) => c,
            Err(e) => {
                self.message.set(Some(format!("audio: {}", js_err(e))));
                return;
            }
        };
        let opts = WorkerOptions::new();
        opts.set_type(WorkerType::Module);
        let worker = match Worker::new_with_options("worker.js", &opts) {
            Ok(w) => w,
            Err(e) => {
                self.message.set(Some(format!("worker: {}", js_err(e))));
                return;
            }
        };
        let me = *self;
        let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |e: MessageEvent| {
            if let Some(text) = e.data().as_string()
                && let Some(m) = parse::<FromWorker>(&text)
            {
                me.on_worker(m);
            }
        });
        worker.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        self.engine.set_value(Some(EngineBackend {
            worker,
            ctx: ctx.clone(),
            ready: false,
            pending: Vec::new(),
            _node: None,
            _on_message: on_message,
        }));
        let settings = self.settings.get_untracked();
        wasm_bindgen_futures::spawn_local(async move {
            if let Err(e) = me.connect_worklet(ctx, volume, settings.replay_gain).await {
                me.message.set(Some(format!("audio: {e}")));
            }
        });
    }

    async fn connect_worklet(
        &self,
        ctx: AudioContext,
        volume: u8,
        replay_gain: ReplayGainConfig,
    ) -> Result<(), String> {
        let worklet = ctx.audio_worklet().map_err(js_err)?;
        JsFuture::from(worklet.add_module("audio-worklet.js").map_err(js_err)?)
            .await
            .map_err(js_err)?;
        let opts = AudioWorkletNodeOptions::new();
        opts.set_number_of_inputs(0);
        opts.set_output_channel_count(&Array::of1(&2.into()));
        let node =
            AudioWorkletNode::new_with_options(&ctx, "infrasonic-output", &opts).map_err(js_err)?;
        node.connect_with_audio_node(&ctx.destination())
            .map_err(js_err)?;
        let channel = MessageChannel::new().map_err(js_err)?;
        let to_worklet = Object::new();
        let _ = Reflect::set(&to_worklet, &"port".into(), &channel.port1());
        node.port()
            .map_err(js_err)?
            .post_message_with_transferable(&to_worklet, &Array::of1(&channel.port1()))
            .map_err(js_err)?;
        let init = ToWorker::Init {
            sample_rate: ctx.sample_rate() as u32,
            volume,
            replay_gain,
        };
        let msg = Object::new();
        let _ = Reflect::set(&msg, &"msg".into(), &init.to_json().into());
        let _ = Reflect::set(&msg, &"port".into(), &channel.port2());
        self.engine.update_value(|e| {
            if let Some(e) = e {
                let _ = e
                    .worker
                    .post_message_with_transfer(&msg, &Array::of1(&channel.port2()));
                for m in e.pending.drain(..) {
                    let _ = e.worker.post_message(&JsValue::from_str(&m.to_json()));
                }
                e.ready = true;
                e._node = Some(node);
            }
        });
        Ok(())
    }

    fn on_worker(&self, m: FromWorker) {
        if self.active.get_untracked() != Some(BackendKind::Engine) {
            return;
        }
        match m {
            FromWorker::Status {
                snapshot,
                cached,
                loading,
            } => {
                self.snapshot.set(snapshot);
                self.cache.set((cached, loading));
            }
            FromWorker::Queue { entries } => self.queue.set(entries),
            FromWorker::Event { event } => self.on_event(event),
            FromWorker::Error { text } => self.message.set(Some(text)),
        }
    }

    fn on_event(&self, e: Event) {
        match e {
            Event::Error(text) => self.message.set(Some(text)),
            Event::Position { position, duration } => self.snapshot.update(|s| {
                s.position = position;
                s.duration = duration;
            }),
            Event::StateChanged(st) => self.snapshot.update(|s| s.state = st),
            Event::VolumeChanged(v) => self.snapshot.update(|s| s.volume = v),
            Event::RepeatChanged(r) => self.snapshot.update(|s| s.repeat = r),
            Event::TrackChanged(entry) => self.snapshot.update(|s| {
                if s.current.as_ref().map(|c| c.uid) != entry.as_ref().map(|c| c.uid) {
                    s.position = 0.0;
                    s.duration = entry.as_ref().and_then(|e| e.track.duration);
                }
                s.current = entry;
            }),
            Event::QueueChanged | Event::QueueEnded => {}
        }
    }

    // --- <audio> element backend ------------------------------------------------------

    fn start_element(&self, volume: u8) {
        if self.element.with_value(|e| e.is_some()) {
            return;
        }
        let audio = match HtmlAudioElement::new() {
            Ok(a) => a,
            Err(e) => {
                self.message.set(Some(format!("audio: {}", js_err(e))));
                return;
            }
        };
        audio.set_preload("auto");
        let me = *self;
        let mut listeners = Vec::new();
        for (name, kind) in [
            ("timeupdate", ElementEvent::Time),
            ("durationchange", ElementEvent::Time),
            ("playing", ElementEvent::Playing),
            ("pause", ElementEvent::Paused),
            ("waiting", ElementEvent::Waiting),
            ("ended", ElementEvent::Ended),
            ("error", ElementEvent::Failed),
        ] {
            let cb = Closure::<dyn FnMut()>::new(move || me.element_event(kind));
            let _ = audio.add_event_listener_with_callback(name, cb.as_ref().unchecked_ref());
            listeners.push(cb);
        }
        audio.set_volume(f64::from(volume_to_gain(volume)));
        self.element.set_value(Some(ElementBackend {
            audio,
            queue: Queue::new(),
            urls: HashMap::new(),
            volume,
            state: PlayState::Stopped,
            _listeners: listeners,
        }));
    }

    fn element_event(&self, kind: ElementEvent) {
        let mut advance = false;
        self.element.update_value(|e| {
            let Some(e) = e else {
                return;
            };
            match kind {
                ElementEvent::Time => {}
                ElementEvent::Playing => e.state = PlayState::Playing,
                ElementEvent::Paused => {
                    if e.state == PlayState::Playing {
                        e.state = PlayState::Paused;
                    }
                }
                ElementEvent::Waiting => e.state = PlayState::Buffering,
                ElementEvent::Ended => advance = true,
                ElementEvent::Failed => {
                    advance = true;
                    self.message.set(Some(format!(
                        "cannot play {}",
                        e.queue
                            .current()
                            .map(|c| c.track.title.as_str())
                            .unwrap_or("")
                    )));
                }
            }
        });
        if advance {
            self.element_advance(true);
        }
        self.element_publish(false);
    }

    /// Go to the next entry (automatically at the end of a song, or by the user).
    fn element_advance(&self, auto: bool) {
        self.element.update_value(|e| {
            let Some(e) = e else {
                return;
            };
            match e.queue.next_index(e.queue.current_index(), auto) {
                Some(n) => {
                    e.queue.set_current(Some(n));
                    element_load(e, 0.0, true);
                }
                None => {
                    e.audio.remove_attribute("src").ok();
                    e.audio.load();
                    e.state = PlayState::Stopped;
                }
            }
        });
    }

    fn element_command(&self, cmd: Command) {
        let mut queue_changed = false;
        let is_next = matches!(cmd, Command::Next);
        self.element.update_value(|e| {
            let Some(e) = e else {
                return;
            };
            let cur_uid = e.queue.current().map(|c| c.uid);
            match cmd {
                Command::SetQueue {
                    tracks,
                    start,
                    position,
                    play,
                } => {
                    e.queue.set(tracks, Some(start));
                    queue_changed = true;
                    element_load(e, position, play);
                }
                Command::Append(t) => {
                    e.queue.append(t);
                    queue_changed = true;
                }
                Command::InsertNext(t) => {
                    e.queue.insert_next(t);
                    queue_changed = true;
                }
                Command::Remove(i) => {
                    e.queue.remove(i);
                    queue_changed = true;
                    if e.queue.current().map(|c| c.uid) != cur_uid {
                        element_load(e, 0.0, e.state == PlayState::Playing);
                    }
                }
                Command::Move { from, to } => {
                    e.queue.move_entry(from, to);
                    queue_changed = true;
                }
                Command::Clear => {
                    e.queue.clear();
                    queue_changed = true;
                    let _ = e.audio.pause();
                    e.audio.remove_attribute("src").ok();
                    e.state = PlayState::Stopped;
                }
                Command::Shuffle => {
                    e.queue.shuffle();
                    queue_changed = true;
                }
                Command::SetRepeat(r) => e.queue.set_repeat(r),
                Command::PlayIndex(i) => {
                    if e.queue.set_current(Some(i)) || e.queue.current_index() == Some(i) {
                        element_load(e, 0.0, true);
                    }
                }
                Command::Play => {
                    if e.audio.src().is_empty() {
                        if e.queue.current_index().is_none() && !e.queue.is_empty() {
                            e.queue.set_current(Some(0));
                        }
                        element_load(e, 0.0, true);
                    } else {
                        let _ = e.audio.play();
                    }
                }
                Command::Pause => {
                    let _ = e.audio.pause();
                }
                Command::TogglePause => {
                    if e.audio.paused() {
                        if e.audio.src().is_empty() {
                            element_load(e, 0.0, true);
                        } else {
                            let _ = e.audio.play();
                        }
                    } else {
                        let _ = e.audio.pause();
                    }
                }
                Command::Stop => {
                    let _ = e.audio.pause();
                    e.audio.set_current_time(0.0);
                    e.state = PlayState::Stopped;
                }
                Command::Seek(t) => e.audio.set_current_time(t.max(0.0)),
                Command::SeekBy(d) => e
                    .audio
                    .set_current_time((e.audio.current_time() + d).max(0.0)),
                Command::SetVolume(v) => {
                    e.volume = v.min(100);
                    e.audio.set_volume(f64::from(volume_to_gain(e.volume)));
                }
                Command::Previous => {
                    if e.audio.current_time() > 3.0 {
                        e.audio.set_current_time(0.0);
                    } else if let Some(p) = e.queue.prev_index(e.queue.current_index()) {
                        e.queue.set_current(Some(p));
                        element_load(e, 0.0, true);
                    }
                }
                Command::UpdateTrack(t) => {
                    queue_changed |= e.queue.update_tracks(&t);
                }
                Command::Next | Command::SetReplayGain(_) | Command::Shutdown => {}
            }
        });
        if is_next {
            self.element_advance(false);
        }
        self.element_publish(queue_changed);
    }

    fn element_publish(&self, queue_changed: bool) {
        let mut snap = None;
        let mut entries = None;
        self.element.with_value(|e| {
            let Some(e) = e else {
                return;
            };
            let a = &e.audio;
            let d = a.duration();
            snap = Some(Snapshot {
                state: e.state,
                current: e.queue.current().cloned(),
                current_index: e.queue.current_index(),
                position: a.current_time(),
                duration: (d.is_finite() && d > 0.0)
                    .then_some(d)
                    .or_else(|| e.queue.current().and_then(|c| c.track.duration)),
                volume: e.volume,
                repeat: e.queue.repeat(),
                queue: Arc::default(),
                underruns: 0,
                replay_gain_db: None,
            });
            if queue_changed {
                entries = Some(e.queue.entries().to_vec());
            }
        });
        if let Some(s) = snap {
            self.snapshot.set(s);
        }
        if let Some(q) = entries {
            self.queue.set(q);
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum ElementEvent {
    Time,
    Playing,
    Paused,
    Waiting,
    Ended,
    Failed,
}

/// Load the current queue entry into the element.
fn element_load(e: &mut ElementBackend, position: f64, play: bool) {
    let Some(cur) = e.queue.current() else {
        return;
    };
    let Some(url) = e.urls.get(&cur.track.id) else {
        return;
    };
    e.audio.set_src(url);
    if position > 0.0 {
        e.audio.set_current_time(position);
    }
    e.state = PlayState::Buffering;
    if play {
        let _ = e.audio.play();
    } else {
        e.state = PlayState::Paused;
    }
}
