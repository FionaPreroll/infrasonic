// SPDX-License-Identifier: GPL-3.0-only
//! Audio output backends.
//!
//! The engine thread writes interleaved `f32` samples into a lock-free ring buffer. The output
//! backend pulls from it in its real-time callback via [`render`], which also applies volume,
//! pause and flush requests and counts played frames for position tracking.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

use parking_lot::Mutex;
/// The engine's end of an output's ring buffer, for implementing [`OutputFactory`].
pub use rtrb::Producer;
use rtrb::{Consumer, RingBuffer};

use crate::Error;
use crate::eq::{EqProcessor, Equalizer};

/// Format of the output stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputSpec {
    pub sample_rate: u32,
    pub channels: usize,
}

/// State shared between the engine thread and the output callback.
#[derive(Debug)]
pub struct OutputShared {
    played_frames: AtomicU64,
    /// Frames taken from the ring but not heard yet (pull backends with their own queue).
    latency_frames: AtomicU64,
    paused: AtomicBool,
    flush: AtomicBool,
    gain_bits: AtomicU32,
    underruns: AtomicU64,
    failed: AtomicBool,
    expect_audio: AtomicBool,
    /// Equalizer filters; only the output callback touches them.
    eq: Mutex<Option<EqProcessor>>,
    /// New settings from the engine, which the callback takes up at its next buffer.
    eq_next: Mutex<Option<(Option<Equalizer>, OutputSpec)>>,
    eq_changed: AtomicBool,
    /// Frames left to fade in, so a jump into the middle of a song (seek, reopened device)
    /// does not click.
    fade_in: AtomicU32,
    /// The callback faded out for a pause and plays silence.
    silent: AtomicBool,
    /// The callback played audio on this stream.
    heard: AtomicBool,
}

/// Length of a fade in or out, in frames (about 5 ms).
const FADE_FRAMES: u32 = 256;

impl Default for OutputShared {
    fn default() -> Self {
        Self {
            played_frames: AtomicU64::new(0),
            latency_frames: AtomicU64::new(0),
            paused: AtomicBool::new(false),
            flush: AtomicBool::new(false),
            gain_bits: AtomicU32::new(1.0f32.to_bits()),
            underruns: AtomicU64::new(0),
            failed: AtomicBool::new(false),
            expect_audio: AtomicBool::new(false),
            eq: Mutex::new(None),
            eq_next: Mutex::new(None),
            eq_changed: AtomicBool::new(false),
            fade_in: AtomicU32::new(0),
            silent: AtomicBool::new(false),
            heard: AtomicBool::new(false),
        }
    }
}

impl OutputShared {
    /// Frames played since the last flush.
    pub fn played_frames(&self) -> u64 {
        self.played_frames
            .load(Ordering::Acquire)
            .saturating_sub(self.latency_frames.load(Ordering::Acquire))
    }
    pub fn set_paused(&self, p: bool) {
        self.paused.store(p, Ordering::Release);
    }
    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Acquire)
    }
    pub fn set_gain(&self, g: f32) {
        self.gain_bits.store(g.to_bits(), Ordering::Release);
    }
    pub fn gain(&self) -> f32 {
        f32::from_bits(self.gain_bits.load(Ordering::Acquire))
    }
    /// Ask the callback to drop all buffered audio and reset the frame counter.
    pub fn request_flush(&self) {
        self.flush.store(true, Ordering::Release);
    }
    pub fn flush_pending(&self) -> bool {
        self.flush.load(Ordering::Acquire)
    }
    /// Buffer underruns while audio was expected.
    pub fn underruns(&self) -> u64 {
        self.underruns.load(Ordering::Relaxed)
    }
    /// The backend reported a fatal error (device gone).
    pub fn mark_failed(&self) {
        self.failed.store(true, Ordering::Release);
    }
    pub fn has_failed(&self) -> bool {
        self.failed.load(Ordering::Acquire)
    }
    /// Whether the engine currently expects audio (used to count underruns).
    pub fn set_expect_audio(&self, v: bool) {
        self.expect_audio.store(v, Ordering::Release);
    }
    /// Use `eq` (or none) for an output with this spec; the callback fades over to it.
    pub fn set_equalizer(&self, eq: Option<&Equalizer>, spec: OutputSpec) {
        *self.eq_next.lock() = Some((eq.copied(), spec));
        self.eq_changed.store(true, Ordering::Release);
    }

    /// Run the equalizer over `out` (the callback's side of [`Self::set_equalizer`]). The
    /// locks are never contended for long: the engine only holds `eq_next` to store settings.
    fn equalize(&self, out: &mut [f32]) {
        let Some(mut eq) = self.eq.try_lock() else {
            return;
        };
        if self.eq_changed.load(Ordering::Acquire)
            && let Some(mut next) = self.eq_next.try_lock()
        {
            self.eq_changed.store(false, Ordering::Release);
            if let Some((settings, spec)) = next.take() {
                let fits = eq
                    .as_ref()
                    .is_some_and(|p| p.fits(spec.sample_rate, spec.channels));
                if !fits {
                    // A new output: start with the settings, or fade to them if this stream
                    // already played without an equalizer.
                    let start = if self.heard.load(Ordering::Relaxed) {
                        None
                    } else {
                        settings.as_ref()
                    };
                    *eq = Some(EqProcessor::new(start, spec.sample_rate, spec.channels));
                }
                if let Some(p) = eq.as_mut()
                    && (fits || self.heard.load(Ordering::Relaxed))
                {
                    p.set(settings.as_ref());
                }
            }
        }
        if let Some(p) = eq.as_mut() {
            p.process(out);
        }
    }

    /// Fade in the next audio, after a flush that jumped into the middle of a song.
    pub fn request_fade_in(&self) {
        self.fade_in.store(FADE_FRAMES, Ordering::Relaxed);
    }

    fn fade_in(&self, out: &mut [f32], channels: usize) {
        let left = self.fade_in.load(Ordering::Relaxed);
        if left == 0 {
            return;
        }
        let mut left = left;
        for frame in out.chunks_exact_mut(channels) {
            if left == 0 {
                break;
            }
            let g = 1.0 - left as f32 / FADE_FRAMES as f32;
            for s in frame {
                *s *= g;
            }
            left -= 1;
        }
        self.fade_in.store(left, Ordering::Relaxed);
    }
}

/// Fill `out` from the ring buffer. Called from the backend's audio callback. Returns how
/// many frames at the start of `out` carry audio; the rest is silence.
///
/// Nothing stops abruptly: a pause and a flush (seek, new queue) first fade out the next few
/// milliseconds of what is playing, and playback after a pause or a jump fades in.
pub fn render(
    consumer: &mut Consumer<f32>,
    shared: &OutputShared,
    out: &mut [f32],
    channels: usize,
) -> usize {
    let channels = channels.max(1);
    let paused = shared.paused.load(Ordering::Acquire);
    if shared.flush.load(Ordering::Acquire) {
        let faded = if paused || shared.silent.load(Ordering::Relaxed) {
            0
        } else {
            fade_out(consumer, shared, out, channels)
        };
        let n = consumer.slots();
        if let Ok(chunk) = consumer.read_chunk(n) {
            chunk.commit_all();
        }
        shared.played_frames.store(0, Ordering::Release);
        shared.flush.store(false, Ordering::Release);
        out[faded * channels..].fill(0.0);
        return faded;
    }
    if paused {
        let faded = if shared.silent.swap(true, Ordering::Relaxed) {
            0
        } else {
            let k = fade_out(consumer, shared, out, channels);
            shared.played_frames.fetch_add(k as u64, Ordering::AcqRel);
            k
        };
        out[faded * channels..].fill(0.0);
        return faded;
    }
    if shared.silent.swap(false, Ordering::Relaxed) {
        shared.request_fade_in();
    }
    let n = take(consumer, shared, out, channels);
    if n > 0 {
        shared.fade_in(&mut out[..n * channels], channels);
        shared.played_frames.fetch_add(n as u64, Ordering::AcqRel);
    }
    if n * channels < out.len() {
        out[n * channels..].fill(0.0);
        if shared.expect_audio.load(Ordering::Acquire) && n > 0 {
            shared.underruns.fetch_add(1, Ordering::Relaxed);
        }
    }
    n
}

/// Move up to `out.len()` samples (whole frames) from the ring into `out`, with volume and
/// equalizer. Returns the number of frames.
fn take(
    consumer: &mut Consumer<f32>,
    shared: &OutputShared,
    out: &mut [f32],
    channels: usize,
) -> usize {
    let avail = consumer.slots() / channels * channels;
    let n = avail.min(out.len() / channels * channels);
    if n == 0 {
        return 0;
    }
    let gain = shared.gain();
    if let Ok(chunk) = consumer.read_chunk(n) {
        let (a, b) = chunk.as_slices();
        for (o, s) in out.iter_mut().zip(a.iter().chain(b.iter())) {
            *o = *s * gain;
        }
        chunk.commit_all();
    }
    shared.equalize(&mut out[..n]);
    shared.heard.store(true, Ordering::Relaxed);
    n / channels
}

/// Play the next [`FADE_FRAMES`] (or what there is) fading to silence. Returns the frames.
fn fade_out(
    consumer: &mut Consumer<f32>,
    shared: &OutputShared,
    out: &mut [f32],
    channels: usize,
) -> usize {
    let len = out.len().min(FADE_FRAMES as usize * channels);
    let n = take(consumer, shared, &mut out[..len], channels);
    for (i, frame) in out[..n * channels].chunks_exact_mut(channels).enumerate() {
        let g = 1.0 - (i + 1) as f32 / FADE_FRAMES as f32;
        for s in frame {
            *s *= g;
        }
    }
    n
}

/// A running output stream. Dropping it stops playback.
pub trait AudioOutput: Send {
    fn spec(&self) -> OutputSpec;

    /// Called by the engine after it requested a flush. Backends whose audio is not pulled by
    /// a callback of their own carry the flush out here.
    fn service(&mut self) {}
}

/// Opens output streams.
pub trait OutputFactory: Send + Sync {
    /// Open a stream whose ring buffer holds `buffer` worth of audio.
    fn open(
        &self,
        shared: Arc<OutputShared>,
        buffer: Duration,
    ) -> Result<(Box<dyn AudioOutput>, Producer<f32>), Error>;
}

fn ring(spec: OutputSpec, buffer: Duration) -> (Producer<f32>, Consumer<f32>) {
    let frames = (f64::from(spec.sample_rate) * buffer.as_secs_f64()).ceil() as usize;
    RingBuffer::new(frames.max(1024) * spec.channels)
}

/// An output for hosts that fetch the audio themselves, such as a browser page feeding an
/// `AudioWorklet`. The engine writes into the ring buffer as usual; the host takes samples
/// with [`PullHandle::pull`] and reports how much of that it has not played yet.
#[derive(Debug, Clone)]
pub struct PullOutputFactory {
    pub spec: OutputSpec,
    handle: PullHandle,
}

impl PullOutputFactory {
    /// The factory and the handle the host keeps.
    pub fn new(spec: OutputSpec) -> (Self, PullHandle) {
        let handle = PullHandle::default();
        (
            Self {
                spec,
                handle: handle.clone(),
            },
            handle,
        )
    }
}

/// The host side of a [`PullOutputFactory`]. Cheap to clone.
#[derive(Debug, Clone, Default)]
pub struct PullHandle {
    inner: Arc<PullInner>,
}

#[derive(Default)]
struct PullInner {
    consumer: Mutex<Option<Consumer<f32>>>,
    shared: Mutex<Option<Arc<OutputShared>>>,
    flushed: AtomicBool,
}

impl std::fmt::Debug for PullInner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PullInner")
            .field("open", &self.consumer.lock().is_some())
            .finish()
    }
}

impl PullInner {
    /// Drop buffered audio if the engine asked for it.
    fn flush_if_requested(&self, consumer: &mut Consumer<f32>, shared: &OutputShared) {
        if shared.flush.load(Ordering::Acquire) {
            let n = consumer.slots();
            if let Ok(chunk) = consumer.read_chunk(n) {
                chunk.commit_all();
            }
            shared.played_frames.store(0, Ordering::Release);
            shared.latency_frames.store(0, Ordering::Release);
            shared.flush.store(false, Ordering::Release);
            self.flushed.store(true, Ordering::Release);
        }
    }
}

impl PullHandle {
    /// Take up to `out.len()` samples (whole frames, volume applied). Returns the number of
    /// frames written; 0 while paused or when nothing is buffered. Never pads with silence.
    pub fn pull(&self, out: &mut [f32]) -> usize {
        let shared = self.inner.shared.lock().clone();
        let mut guard = self.inner.consumer.lock();
        let (Some(shared), Some(consumer)) = (shared, guard.as_mut()) else {
            return 0;
        };
        self.inner.flush_if_requested(consumer, &shared);
        if shared.paused.load(Ordering::Acquire) {
            return 0;
        }
        let channels = self.channels();
        let n = (consumer.slots() / channels * channels).min(out.len() / channels * channels);
        if n == 0 {
            return 0;
        }
        let gain = shared.gain();
        if let Ok(chunk) = consumer.read_chunk(n) {
            let (a, b) = chunk.as_slices();
            for (o, s) in out.iter_mut().zip(a.iter().chain(b.iter())) {
                *o = *s * gain;
            }
            chunk.commit_all();
        }
        shared
            .played_frames
            .fetch_add((n / channels) as u64, Ordering::AcqRel);
        n / channels
    }

    fn channels(&self) -> usize {
        2
    }

    /// Frames the host has taken but not played yet; position tracking subtracts them.
    pub fn set_queued(&self, frames: u64) {
        if let Some(s) = self.inner.shared.lock().as_ref() {
            s.latency_frames.store(frames, Ordering::Release);
        }
    }

    /// The host ran out of audio while playing.
    pub fn note_underrun(&self) {
        if let Some(s) = self.inner.shared.lock().as_ref()
            && s.expect_audio.load(Ordering::Acquire)
        {
            s.underruns.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// True once after each flush: the host should drop the audio it still queues.
    pub fn take_flushed(&self) -> bool {
        self.inner.flushed.swap(false, Ordering::AcqRel)
    }

    /// Whether playback is paused, so the host can hold its own queue.
    pub fn is_paused(&self) -> bool {
        self.inner
            .shared
            .lock()
            .as_ref()
            .is_some_and(|s| s.is_paused())
    }

    /// Whether the engine has an output open.
    pub fn is_open(&self) -> bool {
        self.inner.consumer.lock().is_some()
    }
}

struct PullOutput {
    spec: OutputSpec,
    inner: Arc<PullInner>,
}

impl AudioOutput for PullOutput {
    fn spec(&self) -> OutputSpec {
        self.spec
    }

    fn service(&mut self) {
        let shared = self.inner.shared.lock().clone();
        if let (Some(shared), Some(c)) = (shared, self.inner.consumer.lock().as_mut()) {
            self.inner.flush_if_requested(c, &shared);
        }
    }
}

impl Drop for PullOutput {
    fn drop(&mut self) {
        *self.inner.consumer.lock() = None;
        *self.inner.shared.lock() = None;
    }
}

impl OutputFactory for PullOutputFactory {
    fn open(
        &self,
        shared: Arc<OutputShared>,
        buffer: Duration,
    ) -> Result<(Box<dyn AudioOutput>, Producer<f32>), Error> {
        if self.spec.channels != 2 {
            return Err(Error::Output("the pull output is stereo only".into()));
        }
        let (producer, consumer) = ring(self.spec, buffer);
        *self.handle.inner.consumer.lock() = Some(consumer);
        *self.handle.inner.shared.lock() = Some(shared);
        Ok((
            Box::new(PullOutput {
                spec: self.spec,
                inner: self.handle.inner.clone(),
            }),
            producer,
        ))
    }
}

/// An output that discards audio in (accelerated) real time. Useful for tests, soak runs and
/// headless use. Optionally records everything it plays.
#[derive(Debug, Clone)]
pub struct NullOutputFactory {
    pub spec: OutputSpec,
    /// Playback speed relative to real time (1.0 = real time).
    pub speed: f64,
    /// If set, played samples are appended here.
    pub capture: Option<Arc<Mutex<Vec<f32>>>>,
}

impl NullOutputFactory {
    pub fn new(spec: OutputSpec, speed: f64) -> Self {
        Self {
            spec,
            speed,
            capture: None,
        }
    }

    pub fn with_capture(mut self, capture: Arc<Mutex<Vec<f32>>>) -> Self {
        self.capture = Some(capture);
        self
    }
}

struct NullOutput {
    spec: OutputSpec,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl AudioOutput for NullOutput {
    fn spec(&self) -> OutputSpec {
        self.spec
    }
}

impl Drop for NullOutput {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl OutputFactory for NullOutputFactory {
    fn open(
        &self,
        shared: Arc<OutputShared>,
        buffer: Duration,
    ) -> Result<(Box<dyn AudioOutput>, Producer<f32>), Error> {
        let spec = self.spec;
        let (producer, mut consumer) = ring(spec, buffer);
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let speed = self.speed.max(0.01);
        let capture = self.capture.clone();
        let tick = Duration::from_millis(5);
        let thread = std::thread::Builder::new()
            .name("infrasonic-null-output".into())
            .spawn(move || {
                let frames_per_tick = ((f64::from(spec.sample_rate) * tick.as_secs_f64() * speed)
                    .ceil() as usize)
                    .max(1);
                let mut buf = vec![0.0f32; frames_per_tick * spec.channels];
                while !stop2.load(Ordering::Acquire) {
                    let audible = render(&mut consumer, &shared, &mut buf, spec.channels);
                    if let Some(c) = &capture {
                        c.lock().extend_from_slice(&buf[..audible * spec.channels]);
                    }
                    std::thread::sleep(tick);
                }
            })
            .map_err(|e| Error::Output(e.to_string()))?;
        Ok((
            Box::new(NullOutput {
                spec,
                stop,
                thread: Some(thread),
            }),
            producer,
        ))
    }
}

#[cfg(feature = "cpal-output")]
pub use self::cpal_backend::{CpalOutputFactory, output_device_names};

#[cfg(feature = "cpal-output")]
mod cpal_backend {
    use super::*;
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::{FromSample, SampleFormat, SizedSample};

    /// Output through the system's default audio device via cpal.
    #[derive(Debug, Clone, Default)]
    pub struct CpalOutputFactory {
        /// Device name to use instead of the default device.
        pub device: Option<String>,
    }

    /// Names of the output devices of the default host, for the `device` setting.
    pub fn output_device_names() -> Vec<String> {
        let host = cpal::default_host();
        host.output_devices()
            .map(|it| {
                it.filter_map(|d| d.description().ok().map(|desc| desc.name().to_string()))
                    .collect()
            })
            .unwrap_or_default()
    }

    struct CpalOutput {
        spec: OutputSpec,
        stop: crossbeam_channel::Sender<()>,
    }

    impl AudioOutput for CpalOutput {
        fn spec(&self) -> OutputSpec {
            self.spec
        }
    }

    impl Drop for CpalOutput {
        fn drop(&mut self) {
            let _ = self.stop.send(());
        }
    }

    fn build<T>(
        device: &cpal::Device,
        config: cpal::StreamConfig,
        mut consumer: Consumer<f32>,
        shared: Arc<OutputShared>,
    ) -> Result<cpal::Stream, Error>
    where
        T: SizedSample + FromSample<f32> + Send + 'static,
    {
        let channels = usize::from(config.channels);
        let mut scratch: Vec<f32> = vec![0.0; 8192];
        let shared_err = shared.clone();
        device
            .build_output_stream::<T, _, _>(
                config,
                move |data: &mut [T], _| {
                    if scratch.len() < data.len() {
                        scratch.resize(data.len(), 0.0);
                    }
                    let s = &mut scratch[..data.len()];
                    render(&mut consumer, &shared, s, channels);
                    for (d, x) in data.iter_mut().zip(s.iter()) {
                        *d = T::from_sample(*x);
                    }
                },
                move |err| {
                    // Underruns and a refused real-time priority are not fatal, and on a change
                    // of the default device cpal moves the stream itself (macOS). Everything
                    // else makes the engine open the output again (`recover_output`).
                    if matches!(
                        err.kind(),
                        cpal::ErrorKind::Xrun
                            | cpal::ErrorKind::RealtimeDenied
                            | cpal::ErrorKind::DeviceChanged
                    ) {
                        tracing::debug!("audio output: {err}");
                    } else {
                        tracing::warn!("audio output error: {err}");
                        shared_err.mark_failed();
                    }
                },
                None,
            )
            .map_err(|e| Error::Output(e.to_string()))
    }

    impl OutputFactory for CpalOutputFactory {
        fn open(
            &self,
            shared: Arc<OutputShared>,
            buffer: Duration,
        ) -> Result<(Box<dyn AudioOutput>, Producer<f32>), Error> {
            let (tx, rx) =
                crossbeam_channel::bounded::<Result<(OutputSpec, Producer<f32>), Error>>(1);
            let (stop_tx, stop_rx) = crossbeam_channel::bounded::<()>(1);
            let wanted = self.device.clone();
            // cpal streams are not `Send` on every platform, so a dedicated thread owns it.
            std::thread::Builder::new()
                .name("infrasonic-audio".into())
                .spawn(move || {
                    let host = cpal::default_host();
                    let device = match &wanted {
                        Some(name) => host.output_devices().ok().and_then(|mut it| {
                            it.find(|d| {
                                d.description()
                                    .map(|desc| desc.name() == name.as_str())
                                    .unwrap_or(false)
                            })
                        }),
                        None => None,
                    }
                    .or_else(|| host.default_output_device());
                    let Some(device) = device else {
                        let _ = tx.send(Err(Error::Output("no audio output device".into())));
                        return;
                    };
                    let config = match device.default_output_config() {
                        Ok(c) => c,
                        Err(e) => {
                            let _ = tx.send(Err(Error::Output(e.to_string())));
                            return;
                        }
                    };
                    let spec = OutputSpec {
                        sample_rate: config.sample_rate(),
                        channels: usize::from(config.channels()),
                    };
                    let (producer, consumer) = ring(spec, buffer);
                    let fmt = config.sample_format();
                    let cfg: cpal::StreamConfig = config.into();
                    let stream = match fmt {
                        SampleFormat::F32 => build::<f32>(&device, cfg, consumer, shared),
                        SampleFormat::I16 => build::<i16>(&device, cfg, consumer, shared),
                        SampleFormat::U16 => build::<u16>(&device, cfg, consumer, shared),
                        SampleFormat::I32 => build::<i32>(&device, cfg, consumer, shared),
                        SampleFormat::F64 => build::<f64>(&device, cfg, consumer, shared),
                        other => Err(Error::Output(format!("unsupported sample format {other}"))),
                    };
                    let stream = match stream {
                        Ok(s) => s,
                        Err(e) => {
                            let _ = tx.send(Err(e));
                            return;
                        }
                    };
                    if let Err(e) = stream.play() {
                        let _ = tx.send(Err(Error::Output(e.to_string())));
                        return;
                    }
                    let _ = tx.send(Ok((spec, producer)));
                    let _ = stop_rx.recv();
                    drop(stream);
                })
                .map_err(|e| Error::Output(e.to_string()))?;
            let (spec, producer) = rx
                .recv_timeout(Duration::from_secs(10))
                .map_err(|_| Error::Output("audio device did not respond".into()))??;
            Ok((
                Box::new(CpalOutput {
                    spec,
                    stop: stop_tx,
                }),
                producer,
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_applies_gain_and_counts_frames() {
        let shared = OutputShared::default();
        let (mut p, mut c) = RingBuffer::<f32>::new(16);
        for _ in 0..6 {
            p.push(1.0).expect("push");
        }
        shared.set_gain(0.5);
        let mut out = [9.0f32; 8];
        render(&mut c, &shared, &mut out, 2);
        assert_eq!(out, [0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.0, 0.0]);
        assert_eq!(shared.played_frames(), 3);
    }

    #[test]
    fn pause_fades_out_keeps_the_rest_and_fades_in_again() {
        let shared = OutputShared::default();
        let (mut p, mut c) = RingBuffer::<f32>::new(2048);
        for _ in 0..2048 {
            p.push(1.0).expect("push");
        }
        shared.set_paused(true);
        let mut out = [9.0f32; 1024];
        render(&mut c, &shared, &mut out, 2);
        // 256 frames fading to silence, then silence.
        let left: Vec<f32> = out.iter().step_by(2).copied().collect();
        assert!(left[0] > 0.99 && left[255] < 0.01 && left[256..].iter().all(|s| *s == 0.0));
        assert!(left[..256].windows(2).all(|w| w[1] <= w[0]));
        assert_eq!(shared.played_frames(), 256);
        // Paused: nothing more is taken.
        render(&mut c, &shared, &mut out, 2);
        assert!(out.iter().all(|s| *s == 0.0));
        assert_eq!(c.slots(), 2048 - 512);
        // Resumed: fades in.
        shared.set_paused(false);
        render(&mut c, &shared, &mut out, 2);
        let left: Vec<f32> = out.iter().step_by(2).copied().collect();
        assert!(left[0] < 0.01 && left[300] == 1.0);
        assert!(left[..256].windows(2).all(|w| w[1] >= w[0]));
    }

    #[test]
    fn flush_fades_out_drops_buffer_and_resets_counter() {
        let shared = OutputShared::default();
        let (mut p, mut c) = RingBuffer::<f32>::new(2048);
        for _ in 0..2048 {
            p.push(1.0).expect("push");
        }
        let mut out = [0.0f32; 2];
        render(&mut c, &shared, &mut out, 2);
        assert_eq!(shared.played_frames(), 1);
        shared.request_flush();
        let mut out = [9.0f32; 1024];
        render(&mut c, &shared, &mut out, 2);
        let left: Vec<f32> = out.iter().step_by(2).copied().collect();
        assert!(left[0] > 0.99 && left[255] < 0.01 && left[256..].iter().all(|s| *s == 0.0));
        assert_eq!(c.slots(), 0);
        assert_eq!(shared.played_frames(), 0);
        assert!(!shared.flush_pending());
    }

    #[test]
    fn requested_fade_in() {
        let shared = OutputShared::default();
        let (mut p, mut c) = RingBuffer::<f32>::new(1024);
        for _ in 0..1024 {
            p.push(1.0).expect("push");
        }
        shared.request_fade_in();
        let mut out = [0.0f32; 1024];
        render(&mut c, &shared, &mut out, 2);
        let left: Vec<f32> = out.iter().step_by(2).copied().collect();
        assert!(left[0] < 0.01 && left[256] == 1.0 && left[511] == 1.0);
    }
}
