// SPDX-License-Identifier: GPL-3.0-only
//! Progressive byte sources.
//!
//! A [`StreamWriter`] is fed by whoever downloads the audio (core uses an async HTTP client),
//! the matching [`StreamReader`] implements `Read + Seek` for the decoder and blocks until the
//! requested bytes have arrived. This keeps the engine independent of any HTTP stack or async
//! runtime.

use std::io::{self, Read, Seek, SeekFrom};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::{Condvar, Mutex};
use symphonia::core::io::MediaSource;

/// How long a read waits for new data before giving up.
const STALL_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Default)]
struct State {
    data: Vec<u8>,
    total: Option<u64>,
    done: bool,
    error: Option<String>,
    cancelled: bool,
}

#[derive(Debug, Default)]
struct Shared {
    state: Mutex<State>,
    cond: Condvar,
}

/// Create a connected writer/reader pair. `expected_len` is the `Content-Length`, if known.
pub fn stream_pair(expected_len: Option<u64>) -> (StreamWriter, StreamReader) {
    let shared = Arc::new(Shared::default());
    {
        let mut st = shared.state.lock();
        st.total = expected_len;
        if let Some(n) = expected_len.and_then(|n| usize::try_from(n).ok()) {
            st.data.reserve(n.min(256 * 1024 * 1024));
        }
    }
    (
        StreamWriter {
            shared: shared.clone(),
            finished: false,
        },
        StreamReader {
            shared,
            pos: 0,
            timeout: STALL_TIMEOUT,
        },
    )
}

/// Producer side of a progressive source.
#[derive(Debug)]
pub struct StreamWriter {
    shared: Arc<Shared>,
    finished: bool,
}

impl StreamWriter {
    /// Append bytes. Returns `false` if the reader is gone and the download can stop.
    pub fn push(&self, bytes: &[u8]) -> bool {
        let mut st = self.shared.state.lock();
        if st.cancelled {
            return false;
        }
        st.data.extend_from_slice(bytes);
        drop(st);
        self.shared.cond.notify_all();
        true
    }

    /// Set the total length once it is known (e.g. from `Content-Length`).
    pub fn set_len(&self, len: u64) {
        self.shared.state.lock().total = Some(len);
        self.shared.cond.notify_all();
    }

    /// Whether the reader side has been dropped.
    pub fn is_cancelled(&self) -> bool {
        self.shared.state.lock().cancelled
    }

    /// Number of bytes received so far.
    pub fn len(&self) -> usize {
        self.shared.state.lock().data.len()
    }

    /// Whether no bytes have been received yet.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Mark the stream as complete.
    pub fn finish(mut self) {
        self.close(None);
    }

    /// Mark the stream as failed; readers get an I/O error once they reach the end of data.
    pub fn fail(mut self, message: impl Into<String>) {
        self.close(Some(message.into()));
    }

    fn close(&mut self, error: Option<String>) {
        if self.finished {
            return;
        }
        self.finished = true;
        let mut st = self.shared.state.lock();
        st.done = true;
        if error.is_none() {
            st.total = Some(st.data.len() as u64);
        }
        st.error = error;
        drop(st);
        self.shared.cond.notify_all();
    }
}

impl Drop for StreamWriter {
    fn drop(&mut self) {
        self.close(Some("download aborted".into()));
    }
}

/// Consumer side of a progressive source. Implements [`MediaSource`] for Symphonia.
#[derive(Debug)]
pub struct StreamReader {
    shared: Arc<Shared>,
    pos: u64,
    timeout: Duration,
}

impl StreamReader {
    /// A reader over bytes that are already complete (files, caches, tests).
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        let (w, r) = stream_pair(Some(bytes.len() as u64));
        w.push(&bytes);
        w.finish();
        r
    }

    /// Change how long reads wait for missing data.
    pub fn set_timeout(&mut self, timeout: Duration) {
        self.timeout = timeout;
    }

    /// Bytes received so far.
    pub fn available(&self) -> u64 {
        self.shared.state.lock().data.len() as u64
    }

    /// Whether the download completed successfully.
    pub fn is_complete(&self) -> bool {
        let st = self.shared.state.lock();
        st.done && st.error.is_none()
    }
}

impl Drop for StreamReader {
    fn drop(&mut self) {
        self.shared.state.lock().cancelled = true;
        self.shared.cond.notify_all();
    }
}

impl Read for StreamReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let mut st = self.shared.state.lock();
        loop {
            let len = st.data.len() as u64;
            if self.pos < len {
                let start = usize::try_from(self.pos).map_err(io::Error::other)?;
                let n = buf.len().min(st.data.len() - start);
                buf[..n].copy_from_slice(&st.data[start..start + n]);
                self.pos += n as u64;
                return Ok(n);
            }
            if st.done {
                return match &st.error {
                    Some(e) => Err(io::Error::new(io::ErrorKind::UnexpectedEof, e.clone())),
                    None => Ok(0),
                };
            }
            if self.shared.cond.wait_for(&mut st, self.timeout).timed_out()
                && st.data.len() as u64 <= self.pos
                && !st.done
            {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "stream stalled: no data received",
                ));
            }
        }
    }
}

impl Seek for StreamReader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let new = match pos {
            SeekFrom::Start(p) => Some(p),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d),
            SeekFrom::End(d) => {
                let mut st = self.shared.state.lock();
                while st.total.is_none() && !st.done {
                    if self.shared.cond.wait_for(&mut st, self.timeout).timed_out() {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "stream length unknown",
                        ));
                    }
                }
                let total = st.total.unwrap_or(st.data.len() as u64);
                total.checked_add_signed(d)
            }
        };
        match new {
            Some(p) => {
                self.pos = p;
                Ok(p)
            }
            None => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "seek before start of stream",
            )),
        }
    }
}

impl MediaSource for StreamReader {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        self.shared.state.lock().total
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_complete_buffer() {
        let mut r = StreamReader::from_bytes(b"hello world".to_vec());
        let mut s = String::new();
        r.read_to_string(&mut s).expect("read");
        assert_eq!(s, "hello world");
        assert_eq!(r.byte_len(), Some(11));
    }

    #[test]
    fn reader_waits_for_writer() {
        let (w, mut r) = stream_pair(None);
        let t = std::thread::spawn(move || {
            for chunk in [b"ab".as_slice(), b"cd", b"ef"] {
                std::thread::sleep(Duration::from_millis(10));
                assert!(w.push(chunk));
            }
            w.finish();
        });
        let mut out = Vec::new();
        r.read_to_end(&mut out).expect("read");
        assert_eq!(out, b"abcdef");
        t.join().expect("join");
    }

    #[test]
    fn seek_end_waits_for_length() {
        let (w, mut r) = stream_pair(None);
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            w.push(b"0123456789");
            w.finish();
        });
        assert_eq!(r.seek(SeekFrom::End(-2)).expect("seek"), 8);
        let mut b = [0u8; 2];
        r.read_exact(&mut b).expect("read");
        assert_eq!(&b, b"89");
        t.join().expect("join");
    }

    #[test]
    fn failure_is_reported_after_data() {
        let (w, mut r) = stream_pair(None);
        w.push(b"abc");
        w.fail("boom");
        let mut b = [0u8; 3];
        r.read_exact(&mut b).expect("data first");
        let err = r.read(&mut b).expect_err("then error");
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn dropping_reader_cancels_writer() {
        let (w, r) = stream_pair(None);
        drop(r);
        assert!(!w.push(b"x"));
        assert!(w.is_cancelled());
    }

    #[test]
    fn stalled_stream_times_out() {
        let (_w, mut r) = stream_pair(None);
        r.set_timeout(Duration::from_millis(20));
        let mut b = [0u8; 1];
        assert_eq!(
            r.read(&mut b).expect_err("timeout").kind(),
            io::ErrorKind::TimedOut
        );
    }

    #[test]
    fn seek_before_start_is_error() {
        let mut r = StreamReader::from_bytes(vec![1, 2, 3]);
        assert!(r.seek(SeekFrom::Current(-1)).is_err());
    }
}
