// SPDX-License-Identifier: GPL-3.0-only
//! The engine thread: queue handling, decoding, gapless transitions and position tracking.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, select};
use parking_lot::RwLock;
use rtrb::Producer;

use crate::convert::{RateConverter, map_channels};
use crate::decode::TrackDecoder;
use crate::output::{AudioOutput, OutputFactory, OutputShared, OutputSpec};
use crate::queue::{Queue, QueueEntry, RepeatMode};
use crate::{Error, SourceProvider, Track, volume_to_gain};

/// Commands understood by the engine.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Replace the queue. Starts at `start` / `position` seconds; stays paused unless `play`.
    SetQueue {
        tracks: Vec<Track>,
        start: usize,
        position: f64,
        play: bool,
    },
    Append(Vec<Track>),
    InsertNext(Vec<Track>),
    Remove(usize),
    Move { from: usize, to: usize },
    Clear,
    /// Shuffle the queue; the current track moves to the front.
    Shuffle,
    SetRepeat(RepeatMode),
    /// Play the entry at this index.
    PlayIndex(usize),
    Play,
    Pause,
    TogglePause,
    Stop,
    Next,
    Previous,
    /// Seek to an absolute position in seconds.
    Seek(f64),
    /// Seek relative to the current position.
    SeekBy(f64),
    /// Volume 0–100.
    SetVolume(u8),
    /// Update the metadata of all entries with this track id (e.g. after starring).
    UpdateTrack(Track),
    Shutdown,
}

/// Playback state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum PlayState {
    #[default]
    Stopped,
    /// Waiting for data of the track that should play.
    Buffering,
    Playing,
    Paused,
}

/// Notifications from the engine.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    StateChanged(PlayState),
    /// The audible track changed (`None` when playback stopped).
    TrackChanged(Option<QueueEntry>),
    QueueChanged,
    /// Sent about twice per second while playing, and after seeks.
    Position {
        position: f64,
        duration: Option<f64>,
    },
    VolumeChanged(u8),
    RepeatChanged(RepeatMode),
    /// Playback reached the end of the queue.
    QueueEnded,
    /// A recoverable problem, e.g. a track that could not be decoded.
    Error(String),
}

/// A consistent view of the engine state.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Snapshot {
    pub state: PlayState,
    pub current: Option<QueueEntry>,
    pub current_index: Option<usize>,
    pub position: f64,
    pub duration: Option<f64>,
    pub volume: u8,
    pub repeat: RepeatMode,
    pub queue: Arc<Vec<QueueEntry>>,
    pub underruns: u64,
}

/// Engine settings.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Audio buffered between decoder and output.
    pub buffer: Duration,
    /// Open the next track while the current one plays (needed for gapless playback).
    pub preload_next: bool,
    pub volume: u8,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            buffer: Duration::from_millis(750),
            preload_next: true,
            volume: 100,
        }
    }
}

/// Handle to the engine thread.
#[derive(Debug)]
pub struct Engine {
    tx: Sender<Command>,
    events: Receiver<Event>,
    snapshot: Arc<RwLock<Snapshot>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Engine {
    /// Start the engine thread. The output is opened on first playback.
    pub fn new(
        config: EngineConfig,
        provider: Arc<dyn SourceProvider>,
        output: Box<dyn OutputFactory>,
    ) -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        let (ev_tx, ev_rx) = crossbeam_channel::unbounded();
        let snapshot = Arc::new(RwLock::new(Snapshot {
            volume: config.volume.min(100),
            ..Snapshot::default()
        }));
        let snap = snapshot.clone();
        let thread = std::thread::Builder::new()
            .name("infrasonic-engine".into())
            .spawn(move || Worker::new(config, provider, output, rx, ev_tx, snap).run())
            .ok();
        Self {
            tx,
            events: ev_rx,
            snapshot,
            thread,
        }
    }

    /// Queue a command. Never blocks.
    pub fn send(&self, cmd: Command) {
        let _ = self.tx.send(cmd);
    }

    /// A sender that can be moved to other threads.
    pub fn sender(&self) -> Sender<Command> {
        self.tx.clone()
    }

    /// Engine events. Each event is delivered to exactly one receiver.
    pub fn events(&self) -> &Receiver<Event> {
        &self.events
    }

    /// Current state.
    pub fn snapshot(&self) -> Snapshot {
        self.snapshot.read().clone()
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Shutdown);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Purpose {
    /// The track the user asked for; flushes and starts at the given position.
    Play,
    /// The track after the active one, for a gapless transition.
    Next,
}

struct Loaded {
    generation: u64,
    uid: u64,
    purpose: Purpose,
    start: f64,
    result: Result<TrackDecoder, Error>,
}

struct Active {
    uid: u64,
    dec: TrackDecoder,
    conv: Option<RateConverter>,
    conv_key: (u32, usize),
    pending: Vec<f32>,
    pending_pos: usize,
    scratch: Vec<f32>,
    done: bool,
}

#[derive(Debug, Clone, Copy)]
struct Segment {
    start_frame: u64,
    uid: u64,
    offset: f64,
    duration: Option<f64>,
}

struct Worker {
    config: EngineConfig,
    provider: Arc<dyn SourceProvider>,
    factory: Box<dyn OutputFactory>,
    rx: Receiver<Command>,
    events: Sender<Event>,
    snapshot: Arc<RwLock<Snapshot>>,
    loaded_tx: Sender<Loaded>,
    loaded_rx: Receiver<Loaded>,

    queue: Queue,
    queue_version: u64,
    published_queue_version: u64,
    shared: Arc<OutputShared>,
    output: Option<Box<dyn AudioOutput>>,
    producer: Option<Producer<f32>>,
    spec: OutputSpec,

    state: PlayState,
    want_pause: bool,
    generation: u64,
    active: Option<Active>,
    /// Decoder for the next track, opened in advance.
    preloaded: Option<(u64, TrackDecoder)>,
    /// uid of a preload that is in flight.
    preloading: Option<u64>,
    /// After the active track ended: uid we are waiting for.
    waiting_next: Option<u64>,
    end_of_queue: bool,
    segments: VecDeque<Segment>,
    frames_written: u64,
    current_uid: Option<u64>,
    position: f64,
    duration: Option<f64>,
    volume: u8,
    last_position_event: Instant,
    failures_in_row: usize,
}

impl Worker {
    fn new(
        config: EngineConfig,
        provider: Arc<dyn SourceProvider>,
        factory: Box<dyn OutputFactory>,
        rx: Receiver<Command>,
        events: Sender<Event>,
        snapshot: Arc<RwLock<Snapshot>>,
    ) -> Self {
        let (loaded_tx, loaded_rx) = crossbeam_channel::unbounded();
        let shared = Arc::new(OutputShared::default());
        let volume = config.volume.min(100);
        shared.set_gain(volume_to_gain(volume));
        Self {
            config,
            provider,
            factory,
            rx,
            events,
            snapshot,
            loaded_tx,
            loaded_rx,
            queue: Queue::new(),
            queue_version: 1,
            published_queue_version: 0,
            shared,
            output: None,
            producer: None,
            spec: OutputSpec {
                sample_rate: 44_100,
                channels: 2,
            },
            state: PlayState::Stopped,
            want_pause: false,
            generation: 0,
            active: None,
            preloaded: None,
            preloading: None,
            waiting_next: None,
            end_of_queue: false,
            segments: VecDeque::new(),
            frames_written: 0,
            current_uid: None,
            position: 0.0,
            duration: None,
            volume,
            last_position_event: Instant::now(),
            failures_in_row: 0,
        }
    }

    fn emit(&self, e: Event) {
        let _ = self.events.send(e);
    }

    fn set_state(&mut self, s: PlayState) {
        if self.state != s {
            self.state = s;
            self.shared
                .set_expect_audio(matches!(s, PlayState::Playing));
            self.emit(Event::StateChanged(s));
        }
    }

    fn run(mut self) {
        loop {
            let busy = self.needs_fill();
            let timeout = if busy {
                Duration::from_millis(0)
            } else {
                Duration::from_millis(10)
            };
            let rx = self.rx.clone();
            let loaded_rx = self.loaded_rx.clone();
            select! {
                recv(rx) -> cmd => match cmd {
                    Ok(Command::Shutdown) | Err(_) => break,
                    Ok(c) => self.handle(c),
                },
                recv(loaded_rx) -> l => if let Ok(l) = l { self.handle_loaded(l) },
                default(timeout) => {}
            }
            // Handle bursts of commands before doing audio work.
            while let Ok(c) = self.rx.try_recv() {
                if c == Command::Shutdown {
                    self.stop_output();
                    return;
                }
                self.handle(c);
            }
            self.fill();
            self.update_position();
            self.publish();
        }
        self.stop_output();
    }

    fn stop_output(&mut self) {
        self.producer = None;
        self.output = None;
    }

    fn touch_queue(&mut self) {
        self.queue_version += 1;
        self.emit(Event::QueueChanged);
    }

    fn handle(&mut self, cmd: Command) {
        match cmd {
            Command::SetQueue {
                tracks,
                start,
                position,
                play,
            } => {
                let empty = tracks.is_empty();
                self.queue.set(tracks, Some(start));
                self.touch_queue();
                if empty {
                    self.stop();
                } else {
                    let idx = start.min(self.queue.len() - 1);
                    self.play_index(idx, position, !play);
                }
            }
            Command::Append(t) => {
                self.queue.append(t);
                self.after_queue_edit();
            }
            Command::InsertNext(t) => {
                self.queue.insert_next(t);
                self.after_queue_edit();
            }
            Command::Remove(i) => {
                let removed_current = self.queue.current_index() == Some(i);
                if self.queue.remove(i).is_some() {
                    if removed_current && self.state != PlayState::Stopped {
                        if i < self.queue.len() {
                            self.play_index(i, 0.0, self.want_pause);
                        } else {
                            self.stop();
                        }
                    }
                    self.after_queue_edit();
                }
            }
            Command::Move { from, to } => {
                if self.queue.move_entry(from, to) {
                    self.after_queue_edit();
                }
            }
            Command::Clear => {
                self.queue.clear();
                self.stop();
                self.touch_queue();
            }
            Command::Shuffle => {
                self.queue.shuffle();
                self.after_queue_edit();
            }
            Command::SetRepeat(r) => {
                self.queue.set_repeat(r);
                self.emit(Event::RepeatChanged(r));
                self.after_queue_edit();
            }
            Command::PlayIndex(i) => {
                if i < self.queue.len() {
                    self.play_index(i, 0.0, false);
                }
            }
            Command::Play => self.play(),
            Command::Pause => self.pause(true),
            Command::TogglePause => match self.state {
                PlayState::Playing => self.pause(true),
                PlayState::Buffering => self.pause(!self.want_pause),
                PlayState::Paused | PlayState::Stopped => self.play(),
            },
            Command::Stop => self.stop(),
            Command::Next => {
                if let Some(n) = self.queue.next_index(self.queue.current_index(), false) {
                    self.play_index(n, 0.0, self.want_pause);
                } else {
                    self.stop();
                }
            }
            Command::Previous => {
                if self.position > 3.0 {
                    self.seek(0.0);
                } else if let Some(p) = self.queue.prev_index(self.queue.current_index()) {
                    self.play_index(p, 0.0, self.want_pause);
                }
            }
            Command::Seek(t) => self.seek(t),
            Command::SeekBy(d) => self.seek(self.position + d),
            Command::SetVolume(v) => {
                self.volume = v.min(100);
                self.shared.set_gain(volume_to_gain(self.volume));
                self.emit(Event::VolumeChanged(self.volume));
            }
            Command::UpdateTrack(track) => {
                if self.queue.update_tracks(&track) {
                    self.touch_queue();
                }
            }
            Command::Shutdown => {}
        }
    }

    /// After a queue edit the preloaded "next" track may be wrong.
    fn after_queue_edit(&mut self) {
        self.touch_queue();
        let expected = self.expected_next_uid();
        if let Some((uid, _)) = &self.preloaded
            && Some(*uid) != expected
        {
            self.preloaded = None;
        }
        if self.preloading.is_some() && self.preloading != expected {
            self.preloading = None;
        }
        if self.waiting_next.is_some() && self.waiting_next != expected {
            self.waiting_next = None;
            if let Some(next) = expected {
                self.start_load_next(next);
            }
        }
        if self.end_of_queue && self.active.is_none() && self.waiting_next.is_none() {
            // Something was appended after the last track was written: continue with it.
            if let Some(next) = expected {
                self.end_of_queue = false;
                self.start_load_next(next);
            }
        }
        if self.preloaded.is_none() && self.preloading.is_none() {
            self.preload();
        }
    }

    /// uid of the entry that should follow the decoder's current track.
    fn expected_next_uid(&self) -> Option<u64> {
        let from_uid = self
            .active
            .as_ref()
            .map(|a| a.uid)
            .or(self.segments.back().map(|s| s.uid))?;
        let idx = self.queue.index_of(from_uid)?;
        let n = self.queue.next_index(Some(idx), true)?;
        self.queue.get(n).map(|e| e.uid)
    }

    fn ensure_output(&mut self) -> bool {
        if self.shared.has_failed() {
            self.stop_output();
            self.shared = Arc::new(OutputShared::default());
            self.shared.set_gain(volume_to_gain(self.volume));
        }
        if self.producer.is_some() {
            return true;
        }
        match self.factory.open(self.shared.clone(), self.config.buffer) {
            Ok((out, producer)) => {
                self.spec = out.spec();
                self.output = Some(out);
                self.producer = Some(producer);
                true
            }
            Err(e) => {
                self.emit(Event::Error(e.to_string()));
                false
            }
        }
    }

    fn flush(&mut self) {
        if self.producer.is_some() {
            self.shared.request_flush();
            let deadline = Instant::now() + Duration::from_millis(300);
            while self.shared.flush_pending() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        self.frames_written = 0;
        self.segments.clear();
    }

    fn reset_pipeline(&mut self) {
        self.generation += 1;
        self.flush();
        self.active = None;
        self.preloaded = None;
        self.preloading = None;
        self.waiting_next = None;
        self.end_of_queue = false;
    }

    fn play_index(&mut self, index: usize, position: f64, paused: bool) {
        let Some(entry) = self.queue.get(index).cloned() else {
            return;
        };
        if !self.ensure_output() {
            self.stop();
            return;
        }
        self.reset_pipeline();
        self.queue.set_current(Some(index));
        self.want_pause = paused;
        self.shared.set_paused(paused);
        if self.current_uid != Some(entry.uid) {
            self.current_uid = Some(entry.uid);
            self.emit(Event::TrackChanged(Some(entry.clone())));
        }
        self.position = position.max(0.0);
        self.duration = entry.track.duration;
        self.set_state(PlayState::Buffering);
        self.spawn_open(entry, Purpose::Play, position.max(0.0));
    }

    fn spawn_open(&self, entry: QueueEntry, purpose: Purpose, start: f64) {
        let provider = self.provider.clone();
        let tx = self.loaded_tx.clone();
        let generation = self.generation;
        let spawned = std::thread::Builder::new()
            .name("infrasonic-loader".into())
            .spawn(move || {
                let result = provider.open(&entry.track).and_then(|src| {
                    let mut dec = TrackDecoder::open(src, entry.track.format_hint())?;
                    if start > 0.0 {
                        dec.seek(start)?;
                    }
                    Ok(dec)
                });
                let _ = tx.send(Loaded {
                    generation,
                    uid: entry.uid,
                    purpose,
                    start,
                    result,
                });
            });
        if let Err(e) = spawned {
            self.emit(Event::Error(format!("cannot start loader: {e}")));
        }
    }

    fn spawn_seek(&self, uid: u64, mut dec: TrackDecoder, to: f64) {
        let tx = self.loaded_tx.clone();
        let generation = self.generation;
        let _ = std::thread::Builder::new()
            .name("infrasonic-seek".into())
            .spawn(move || {
                let result = dec.seek(to).map(|_| dec);
                let _ = tx.send(Loaded {
                    generation,
                    uid,
                    purpose: Purpose::Play,
                    start: to,
                    result,
                });
            });
    }

    fn handle_loaded(&mut self, l: Loaded) {
        if l.generation != self.generation {
            return;
        }
        match l.purpose {
            Purpose::Play => {
                if self.current_uid != Some(l.uid) {
                    return;
                }
                match l.result {
                    Ok(dec) => {
                        self.failures_in_row = 0;
                        if self.duration.is_none() {
                            self.duration = dec.duration();
                        }
                        self.activate(l.uid, dec, l.start);
                        let s = if self.want_pause {
                            PlayState::Paused
                        } else {
                            PlayState::Playing
                        };
                        self.set_state(s);
                        self.emit_position();
                        self.preload();
                    }
                    Err(e) => self.track_failed(l.uid, e),
                }
            }
            Purpose::Next => {
                let waiting = self.waiting_next == Some(l.uid);
                if !waiting && self.preloading != Some(l.uid) {
                    return;
                }
                match l.result {
                    Ok(dec) => {
                        if waiting {
                            self.waiting_next = None;
                            self.activate(l.uid, dec, 0.0);
                            self.preload();
                        } else {
                            self.preloading = None;
                            self.preloaded = Some((l.uid, dec));
                        }
                    }
                    Err(e) => {
                        self.preloading = None;
                        if waiting {
                            self.waiting_next = None;
                            self.emit(Event::Error(e.to_string()));
                            // Skip the broken track once it would have started.
                            if let Some(idx) = self.queue.index_of(l.uid)
                                && let Some(n) = self.queue.next_index(Some(idx), false)
                                && let Some(e) = self.queue.get(n)
                                && e.uid != l.uid
                            {
                                self.waiting_next = Some(e.uid);
                                self.spawn_open(e.clone(), Purpose::Next, 0.0);
                            } else {
                                self.end_of_queue = true;
                            }
                        }
                    }
                }
            }
        }
    }

    fn track_failed(&mut self, uid: u64, e: Error) {
        self.emit(Event::Error(e.to_string()));
        self.failures_in_row += 1;
        let idx = self.queue.index_of(uid);
        let next = self.queue.next_index(idx, false);
        match next {
            Some(n) if self.failures_in_row < self.queue.len() && Some(n) != idx => {
                self.play_index(n, 0.0, self.want_pause);
            }
            _ => {
                self.failures_in_row = 0;
                self.stop();
            }
        }
    }

    fn activate(&mut self, uid: u64, dec: TrackDecoder, offset: f64) {
        let duration = self
            .queue
            .index_of(uid)
            .and_then(|i| self.queue.get(i))
            .and_then(|e| e.track.duration)
            .or(dec.duration());
        self.segments.push_back(Segment {
            start_frame: self.frames_written,
            uid,
            offset,
            duration,
        });
        self.active = Some(Active {
            uid,
            conv: None,
            conv_key: (0, 0),
            dec,
            pending: Vec::new(),
            pending_pos: 0,
            scratch: Vec::new(),
            done: false,
        });
    }

    fn preload(&mut self) {
        if !self.config.preload_next || self.preloaded.is_some() || self.preloading.is_some() {
            return;
        }
        let Some(next) = self.expected_next_uid() else {
            return;
        };
        if let Some(e) = self.queue.index_of(next).and_then(|i| self.queue.get(i)).cloned() {
            self.preloading = Some(next);
            self.spawn_open(e, Purpose::Next, 0.0);
        }
    }

    fn start_load_next(&mut self, uid: u64) {
        if let Some(e) = self.queue.index_of(uid).and_then(|i| self.queue.get(i)).cloned() {
            self.waiting_next = Some(uid);
            self.spawn_open(e, Purpose::Next, 0.0);
        }
    }

    fn play(&mut self) {
        match self.state {
            PlayState::Stopped => {
                let idx = self.queue.current_index().or(if self.queue.is_empty() {
                    None
                } else {
                    Some(0)
                });
                if let Some(i) = idx {
                    self.play_index(i, 0.0, false);
                }
            }
            PlayState::Paused => {
                self.want_pause = false;
                self.shared.set_paused(false);
                self.set_state(PlayState::Playing);
            }
            PlayState::Buffering => {
                self.want_pause = false;
                self.shared.set_paused(false);
            }
            PlayState::Playing => {}
        }
    }

    fn pause(&mut self, p: bool) {
        self.want_pause = p;
        self.shared.set_paused(p);
        match self.state {
            PlayState::Playing if p => self.set_state(PlayState::Paused),
            PlayState::Paused if !p => self.set_state(PlayState::Playing),
            _ => {}
        }
    }

    fn stop(&mut self) {
        self.reset_pipeline();
        self.want_pause = false;
        self.shared.set_paused(false);
        self.position = 0.0;
        self.set_state(PlayState::Stopped);
        self.emit_position();
    }

    fn seek(&mut self, to: f64) {
        let Some(uid) = self.current_uid else {
            return;
        };
        if self.state == PlayState::Stopped {
            return;
        }
        let mut to = to.max(0.0);
        if let Some(d) = self.duration {
            if to >= d {
                // Seeking past the end skips to the next track.
                self.handle(Command::Next);
                return;
            }
            to = to.min(d);
        }
        let active_is_current =
            self.active.as_ref().is_some_and(|a| a.uid == uid) && self.segments.len() <= 1;
        let dec = if active_is_current {
            self.active.take().map(|a| a.dec)
        } else {
            None
        };
        self.reset_pipeline();
        self.position = to;
        self.set_state(PlayState::Buffering);
        self.emit_position();
        match dec {
            Some(d) => self.spawn_seek(uid, d, to),
            None => {
                if let Some(e) = self.queue.index_of(uid).and_then(|i| self.queue.get(i)).cloned()
                {
                    self.spawn_open(e, Purpose::Play, to);
                }
            }
        }
    }

    fn needs_fill(&self) -> bool {
        let Some(p) = &self.producer else {
            return false;
        };
        let Some(a) = &self.active else {
            return false;
        };
        let ch = self.spec.channels.max(1);
        (a.pending_pos < a.pending.len() || !a.done) && p.slots() >= 1024 * ch
    }

    fn fill(&mut self) {
        let ch = self.spec.channels.max(1);
        let rate = self.spec.sample_rate;
        // Bound the work per loop iteration so commands stay responsive.
        for _ in 0..32 {
            let Some(producer) = self.producer.as_mut() else {
                return;
            };
            let Some(a) = self.active.as_mut() else {
                return;
            };
            if a.pending_pos < a.pending.len() {
                let free = producer.slots() / ch * ch;
                let n = free.min(a.pending.len() - a.pending_pos);
                if n == 0 {
                    return;
                }
                if let Ok(chunk) = producer.write_chunk_uninit(n) {
                    chunk.fill_from_iter(a.pending[a.pending_pos..a.pending_pos + n].iter().copied());
                }
                a.pending_pos += n;
                self.frames_written += (n / ch) as u64;
                continue;
            }
            a.pending.clear();
            a.pending_pos = 0;
            if a.done {
                self.track_written();
                continue;
            }
            match a.dec.next_chunk() {
                Ok(Some(chunk)) => {
                    let key = (chunk.rate, chunk.channels);
                    if a.conv_key != key {
                        a.conv_key = key;
                        a.conv = RateConverter::new(chunk.rate, rate, ch);
                    }
                    a.scratch.clear();
                    map_channels(chunk.samples, chunk.channels, ch, &mut a.scratch);
                    match &mut a.conv {
                        Some(c) => c.process(&a.scratch, &mut a.pending),
                        None => std::mem::swap(&mut a.pending, &mut a.scratch),
                    }
                }
                Ok(None) => {
                    if let Some(c) = &mut a.conv {
                        c.finish(&mut a.pending);
                    }
                    a.done = true;
                }
                Err(e) => {
                    let msg = e.to_string();
                    a.done = true;
                    self.emit(Event::Error(msg));
                }
            }
        }
    }

    /// The active decoder delivered all its samples: continue with the next track.
    fn track_written(&mut self) {
        let Some(a) = self.active.take() else {
            return;
        };
        let next = self
            .queue
            .index_of(a.uid)
            .and_then(|i| self.queue.next_index(Some(i), true))
            .and_then(|n| self.queue.get(n))
            .map(|e| e.uid);
        let Some(next) = next else {
            self.end_of_queue = true;
            return;
        };
        match self.preloaded.take() {
            Some((uid, dec)) if uid == next => {
                self.activate(uid, dec, 0.0);
                self.preload();
            }
            _ => {
                if self.preloading == Some(next) {
                    self.preloading = None;
                    self.waiting_next = Some(next);
                } else {
                    self.start_load_next(next);
                }
            }
        }
    }

    fn update_position(&mut self) {
        if self.producer.is_some() && self.shared.has_failed() {
            self.emit(Event::Error("audio device lost".into()));
            self.stop();
            self.stop_output();
            return;
        }
        if self.shared.flush_pending() {
            return;
        }
        let played = self.shared.played_frames();
        let mut transitioned = false;
        while self.segments.len() > 1 && self.segments[1].start_frame <= played {
            self.segments.pop_front();
            transitioned = true;
        }
        let rate = f64::from(self.spec.sample_rate.max(1));
        if let Some(seg) = self.segments.front().copied() {
            if seg.start_frame <= played {
                if transitioned || self.current_uid != Some(seg.uid) {
                    self.current_uid = Some(seg.uid);
                    let idx = self.queue.index_of(seg.uid);
                    self.queue.set_current(idx);
                    self.duration = seg.duration;
                    let entry = idx.and_then(|i| self.queue.get(i)).cloned();
                    self.emit(Event::TrackChanged(entry));
                    self.emit_position();
                }
                self.position = seg.offset + (played - seg.start_frame) as f64 / rate;
                if let Some(d) = seg.duration {
                    self.position = self.position.min(d);
                }
            }
        }
        if self.end_of_queue
            && self.active.is_none()
            && self.waiting_next.is_none()
            && played >= self.frames_written
            && self.state != PlayState::Stopped
        {
            self.end_of_queue = false;
            self.reset_pipeline();
            self.position = 0.0;
            self.set_state(PlayState::Stopped);
            self.current_uid = None;
            self.queue.set_current(None);
            self.emit(Event::TrackChanged(None));
            self.emit(Event::QueueEnded);
        }
        if self.state == PlayState::Playing
            && self.last_position_event.elapsed() >= Duration::from_millis(500)
        {
            self.emit_position();
        }
    }

    fn emit_position(&mut self) {
        self.last_position_event = Instant::now();
        self.emit(Event::Position {
            position: self.position,
            duration: self.duration,
        });
    }

    fn publish(&mut self) {
        let mut s = self.snapshot.write();
        s.state = self.state;
        s.current_index = self.queue.current_index();
        s.current = self.queue.current().cloned();
        s.position = self.position;
        s.duration = self.duration;
        s.volume = self.volume;
        s.repeat = self.queue.repeat();
        s.underruns = self.shared.underruns();
        if self.published_queue_version != self.queue_version {
            s.queue = Arc::new(self.queue.entries().to_vec());
            self.published_queue_version = self.queue_version;
        }
    }
}
