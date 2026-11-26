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
use rtrb::{Consumer, Producer, RingBuffer};

use crate::Error;

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
    paused: AtomicBool,
    flush: AtomicBool,
    gain_bits: AtomicU32,
    underruns: AtomicU64,
    failed: AtomicBool,
    expect_audio: AtomicBool,
}

impl Default for OutputShared {
    fn default() -> Self {
        Self {
            played_frames: AtomicU64::new(0),
            paused: AtomicBool::new(false),
            flush: AtomicBool::new(false),
            gain_bits: AtomicU32::new(1.0f32.to_bits()),
            underruns: AtomicU64::new(0),
            failed: AtomicBool::new(false),
            expect_audio: AtomicBool::new(false),
        }
    }
}

impl OutputShared {
    /// Frames played since the last flush.
    pub fn played_frames(&self) -> u64 {
        self.played_frames.load(Ordering::Acquire)
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
}

/// Fill `out` from the ring buffer. Called from the backend's audio callback.
pub fn render(
    consumer: &mut Consumer<f32>,
    shared: &OutputShared,
    out: &mut [f32],
    channels: usize,
) {
    if shared.flush.load(Ordering::Acquire) {
        let n = consumer.slots();
        if let Ok(chunk) = consumer.read_chunk(n) {
            chunk.commit_all();
        }
        shared.played_frames.store(0, Ordering::Release);
        shared.flush.store(false, Ordering::Release);
    }
    if shared.paused.load(Ordering::Acquire) {
        out.fill(0.0);
        return;
    }
    let gain = shared.gain();
    let channels = channels.max(1);
    let avail = consumer.slots() / channels * channels;
    let n = avail.min(out.len() / channels * channels);
    if n > 0 {
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
    }
    if n < out.len() {
        out[n..].fill(0.0);
        if shared.expect_audio.load(Ordering::Acquire) && n > 0 {
            shared.underruns.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// A running output stream. Dropping it stops playback.
pub trait AudioOutput: Send {
    fn spec(&self) -> OutputSpec;
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
                    let before = shared.played_frames();
                    render(&mut consumer, &shared, &mut buf, spec.channels);
                    if let Some(c) = &capture {
                        let after = shared.played_frames();
                        let played = after.saturating_sub(before) as usize;
                        c.lock().extend_from_slice(&buf[..played * spec.channels]);
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
                    tracing::warn!("audio output error: {err}");
                    if !matches!(
                        err.kind(),
                        cpal::ErrorKind::Xrun | cpal::ErrorKind::RealtimeDenied
                    ) {
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
    fn paused_output_keeps_buffer() {
        let shared = OutputShared::default();
        let (mut p, mut c) = RingBuffer::<f32>::new(4);
        p.push(1.0).expect("push");
        p.push(1.0).expect("push");
        shared.set_paused(true);
        let mut out = [9.0f32; 2];
        render(&mut c, &shared, &mut out, 2);
        assert_eq!(out, [0.0, 0.0]);
        assert_eq!(c.slots(), 2);
        assert_eq!(shared.played_frames(), 0);
    }

    #[test]
    fn flush_drops_buffer_and_resets_counter() {
        let shared = OutputShared::default();
        let (mut p, mut c) = RingBuffer::<f32>::new(8);
        for _ in 0..8 {
            p.push(1.0).expect("push");
        }
        let mut out = [0.0f32; 2];
        render(&mut c, &shared, &mut out, 2);
        assert_eq!(shared.played_frames(), 1);
        shared.request_flush();
        render(&mut c, &shared, &mut out, 2);
        assert_eq!(out, [0.0, 0.0]);
        assert_eq!(shared.played_frames(), 0);
        assert!(!shared.flush_pending());
    }
}
