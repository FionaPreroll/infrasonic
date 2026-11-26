// SPDX-License-Identifier: GPL-3.0-only
//! Logging to a file plus an in-memory buffer for the log page.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;

const MAX_LINES: usize = 2000;

/// One entry of the log page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLine {
    pub time: String,
    pub level: Level,
    pub target: String,
    pub message: String,
}

/// Shared ring buffer of recent log lines.
#[derive(Debug, Clone, Default)]
pub struct LogBuffer {
    inner: Arc<Mutex<(u64, VecDeque<LogLine>)>>,
}

impl LogBuffer {
    pub fn push(&self, line: LogLine) {
        let mut g = self.inner.lock();
        g.0 += 1;
        if g.1.len() >= MAX_LINES {
            g.1.pop_front();
        }
        g.1.push_back(line);
    }

    /// Counter that changes with every new line.
    pub fn generation(&self) -> u64 {
        self.inner.lock().0
    }

    pub fn lines(&self) -> Vec<LogLine> {
        self.inner.lock().1.iter().cloned().collect()
    }
}

struct BufferLayer(LogBuffer);

#[derive(Default)]
struct MessageVisitor(String);

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            let _ = write!(self.0, "{value:?}");
        } else {
            let _ = write!(self.0, " {}={value:?}", field.name());
        }
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.0.push_str(value);
        } else {
            let _ = write!(self.0, " {}={value}", field.name());
        }
    }
}

fn clock() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // UTC; good enough to order events within a session.
    format!(
        "{:02}:{:02}:{:02}",
        (secs / 3600) % 24,
        (secs / 60) % 60,
        secs % 60
    )
}

impl<S: Subscriber> Layer<S> for BufferLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut v = MessageVisitor::default();
        event.record(&mut v);
        self.0.push(LogLine {
            time: clock(),
            level: *event.metadata().level(),
            target: event.metadata().target().to_string(),
            message: v.0,
        });
    }
}

/// Install the global subscriber: `log_file` (if it can be opened) and the returned buffer.
/// The level comes from `INFRASONIC_LOG` (default `info`).
pub fn init(log_file: Option<&Path>) -> LogBuffer {
    let buffer = LogBuffer::default();
    let filter = || {
        EnvFilter::try_from_env("INFRASONIC_LOG")
            .unwrap_or_else(|_| EnvFilter::new("info,symphonia=warn,zbus=warn"))
    };
    let file = log_file.and_then(|p| {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(p)
            .ok()
    });
    let file_layer = file.map(|f| {
        tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(std::sync::Mutex::new(f))
            .with_filter(filter())
    });
    let _ = tracing_subscriber::registry()
        .with(file_layer)
        .with(BufferLayer(buffer.clone()).with_filter(filter()))
        .try_init();
    buffer
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_buffer() {
        let b = LogBuffer::default();
        for i in 0..MAX_LINES + 5 {
            b.push(LogLine {
                time: String::new(),
                level: Level::INFO,
                target: String::new(),
                message: i.to_string(),
            });
        }
        let l = b.lines();
        assert_eq!(l.len(), MAX_LINES);
        assert_eq!(l[0].message, "5");
        assert_eq!(b.generation(), (MAX_LINES + 5) as u64);
    }
}
