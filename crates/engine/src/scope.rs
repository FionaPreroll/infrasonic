// SPDX-License-Identifier: GPL-3.0-only
//! A copy of the audio on its way to the output, for a visualizer.
//!
//! [`ScopedOutputFactory`] wraps any [`OutputFactory`]: every stream it opens hands the samples it plays to a
//! [`Scope`], at the moment the output callback takes them from the ring buffer (before volume and equalizer, so the
//! picture does not follow the volume knob). A reader takes them with [`ScopeReader::read`]. The callback never
//! waits: without a reader it does nothing, and what does not fit into the reader's ring is dropped.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

use parking_lot::Mutex;
use rtrb::{Consumer, Producer, RingBuffer};

use crate::Error;
use crate::output::{AudioOutput, OutputFactory, OutputShared, OutputSpec};

/// Where the output callback leaves its samples. Shared by the factory, every stream it opened and the reader.
#[derive(Debug, Default)]
pub struct Scope {
    /// The ring of the current reader, with that reader's number.
    producer: Mutex<Option<(u64, Producer<f32>)>>,
    /// Number of the last reader opened.
    readers: AtomicU64,
    /// A reader is attached: the callback copies samples.
    active: AtomicBool,
    /// Format of the stream that feeds the scope (0 = none opened yet).
    sample_rate: AtomicU32,
    channels: AtomicU32,
    /// Samples the reader's ring had no room for.
    dropped: AtomicU64,
}

impl Scope {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Start copying samples into a ring of `capacity` samples for the returned reader. An earlier reader stops
    /// receiving (and dropping it later does not stop the new one).
    pub fn open(self: &Arc<Self>, capacity: usize) -> ScopeReader {
        let (producer, consumer) = RingBuffer::new(capacity.max(1));
        let id = self.readers.fetch_add(1, Ordering::Relaxed) + 1;
        *self.producer.lock() = Some((id, producer));
        self.active.store(true, Ordering::Release);
        ScopeReader {
            consumer,
            scope: Arc::clone(self),
            id,
        }
    }

    /// Format of the stream currently feeding the scope; `None` before an output was opened.
    pub fn format(&self) -> Option<OutputSpec> {
        let sample_rate = self.sample_rate.load(Ordering::Acquire);
        let channels = self.channels.load(Ordering::Acquire) as usize;
        (sample_rate > 0 && channels > 0).then_some(OutputSpec {
            sample_rate,
            channels,
        })
    }

    /// Samples dropped because the reader fell behind.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    fn set_format(&self, spec: OutputSpec) {
        self.sample_rate.store(spec.sample_rate, Ordering::Release);
        self.channels
            .store(u32::try_from(spec.channels).unwrap_or(0), Ordering::Release);
    }

    /// The output callback's side: copy `samples` (interleaved, whole frames) if a reader is attached. Never
    /// blocks: a busy lock (the reader is being replaced) or a full ring drops them.
    pub(crate) fn feed(&self, samples: &[f32]) {
        if !self.active.load(Ordering::Acquire) {
            return;
        }
        let Some(mut guard) = self.producer.try_lock() else {
            return;
        };
        let Some((_, producer)) = guard.as_mut() else {
            return;
        };
        let n = producer.slots().min(samples.len());
        if let Ok(mut chunk) = producer.write_chunk(n) {
            let (a, b) = chunk.as_mut_slices();
            let split = a.len();
            a.copy_from_slice(&samples[..split]);
            b.copy_from_slice(&samples[split..n]);
            chunk.commit_all();
        }
        if n < samples.len() {
            self.dropped
                .fetch_add((samples.len() - n) as u64, Ordering::Relaxed);
        }
    }

    /// Reader `id` is gone; stop copying unless a newer reader took over.
    fn close(&self, id: u64) {
        let mut producer = self.producer.lock();
        if producer.as_ref().is_some_and(|(current, _)| *current == id) {
            self.active.store(false, Ordering::Release);
            *producer = None;
        }
    }
}

/// The reader of a [`Scope`]. Dropping it stops the copying.
#[derive(Debug)]
pub struct ScopeReader {
    consumer: Consumer<f32>,
    scope: Arc<Scope>,
    id: u64,
}

impl ScopeReader {
    /// Append every sample that arrived since the last call to `out` (interleaved, in [`Scope::format`]).
    pub fn read(&mut self, out: &mut Vec<f32>) -> usize {
        let n = self.consumer.slots();
        if let Ok(chunk) = self.consumer.read_chunk(n) {
            let (a, b) = chunk.as_slices();
            out.extend_from_slice(a);
            out.extend_from_slice(b);
            chunk.commit_all();
        }
        n
    }

    pub fn format(&self) -> Option<OutputSpec> {
        self.scope.format()
    }
}

impl Drop for ScopeReader {
    fn drop(&mut self) {
        self.scope.close(self.id);
    }
}

/// An [`OutputFactory`] whose streams also feed a [`Scope`].
#[derive(Debug)]
pub struct ScopedOutputFactory<F> {
    inner: F,
    scope: Arc<Scope>,
}

impl<F: OutputFactory> ScopedOutputFactory<F> {
    pub fn new(inner: F, scope: Arc<Scope>) -> Self {
        Self { inner, scope }
    }
}

impl<F: OutputFactory> OutputFactory for ScopedOutputFactory<F> {
    fn open(
        &self,
        shared: Arc<OutputShared>,
        buffer: Duration,
    ) -> Result<(Box<dyn AudioOutput>, Producer<f32>), Error> {
        shared.set_scope(Arc::clone(&self.scope));
        let (output, producer) = self.inner.open(shared, buffer)?;
        self.scope.set_format(output.spec());
        Ok((output, producer))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reader_gets_what_was_fed_and_nothing_before_it() {
        let scope = Scope::new();
        scope.feed(&[1.0, 2.0]);
        let mut reader = scope.open(8);
        scope.feed(&[3.0, 4.0, 5.0, 6.0]);
        let mut out = Vec::new();
        assert_eq!(reader.read(&mut out), 4);
        assert_eq!(out, [3.0, 4.0, 5.0, 6.0]);
        assert_eq!(reader.read(&mut out), 0);
    }

    #[test]
    fn a_full_ring_drops_instead_of_waiting() {
        let scope = Scope::new();
        let mut reader = scope.open(4);
        scope.feed(&[1.0; 6]);
        assert_eq!(scope.dropped(), 2);
        let mut out = Vec::new();
        assert_eq!(reader.read(&mut out), 4);
        // wraps around the ring
        scope.feed(&[2.0, 3.0, 4.0]);
        out.clear();
        reader.read(&mut out);
        assert_eq!(out, [2.0, 3.0, 4.0]);
    }

    #[test]
    fn dropping_the_reader_stops_the_copying() {
        let scope = Scope::new();
        let reader = scope.open(4);
        drop(reader);
        scope.feed(&[1.0; 4]);
        assert_eq!(scope.dropped(), 0);
        assert!(scope.producer.lock().is_none());
    }

    #[test]
    fn an_old_reader_dropped_late_does_not_stop_the_new_one() {
        let scope = Scope::new();
        let old = scope.open(4);
        let mut new = scope.open(4);
        drop(old);
        scope.feed(&[1.0, 2.0]);
        let mut out = Vec::new();
        assert_eq!(new.read(&mut out), 2);
        drop(new);
        assert!(scope.producer.lock().is_none());
    }

    #[test]
    fn the_output_callback_feeds_the_samples_before_volume() {
        let scope = Scope::new();
        let shared = OutputShared::default();
        shared.set_scope(scope.clone());
        shared.set_gain(0.5);
        let mut reader = scope.open(64);
        let (mut producer, mut consumer) = RingBuffer::new(64);
        for s in [0.1f32, 0.2, 0.3, 0.4] {
            producer.push(s).expect("room");
        }
        let mut out = [0.0f32; 4];
        assert_eq!(
            crate::output::render(&mut consumer, &shared, &mut out, 2),
            2
        );
        let mut seen = Vec::new();
        reader.read(&mut seen);
        assert_eq!(seen, [0.1, 0.2, 0.3, 0.4]);
        assert!(
            (out[3] - 0.2).abs() < 1e-6,
            "the output still gets the volume: {out:?}"
        );
    }

    #[test]
    fn opened_streams_feed_the_scope_and_report_their_format() {
        let scope = Scope::new();
        let spec = OutputSpec {
            sample_rate: 48_000,
            channels: 2,
        };
        let factory = ScopedOutputFactory::new(
            crate::output::NullOutputFactory::new(spec, 1.0),
            scope.clone(),
        );
        let mut reader = scope.open(1 << 16);
        let (_output, mut producer) = factory
            .open(
                Arc::new(OutputShared::default()),
                Duration::from_millis(100),
            )
            .expect("open");
        assert_eq!(scope.format(), Some(spec));
        while producer.push(0.25).is_ok() {}
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut seen = Vec::new();
        while seen.is_empty() && std::time::Instant::now() < deadline {
            reader.read(&mut seen);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!seen.is_empty() && seen.iter().all(|&s| s == 0.25));
    }
}
