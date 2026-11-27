// SPDX-License-Identifier: GPL-3.0-only
//! Messages between the page and the audio worker, sent as JSON strings with `postMessage`.
//! The AudioWorklet port travels next to [`ToWorker::Init`] as a transferable.

use infrasonic_engine::{Command, Event, QueueEntry, ReplayGainConfig, Snapshot};
use serde::{Deserialize, Serialize};

/// Page → worker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t")]
pub enum ToWorker {
    /// Start the engine for an output at `sample_rate`.
    Init {
        sample_rate: u32,
        volume: u8,
        replay_gain: ReplayGainConfig,
    },
    /// Stream URLs of track ids used by the commands that follow.
    Urls {
        urls: Vec<(String, String)>,
    },
    Command {
        cmd: Command,
    },
}

/// Worker → page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t")]
pub enum FromWorker {
    Event {
        event: Event,
    },
    /// The engine state without the queue (that comes with [`FromWorker::Queue`]).
    Status {
        snapshot: Snapshot,
        /// Songs downloaded and kept in memory, and songs still loading.
        cached: usize,
        loading: usize,
    },
    Queue {
        entries: Vec<QueueEntry>,
    },
    /// A download or decoder problem worth showing.
    Error {
        text: String,
    },
}

impl ToWorker {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

impl FromWorker {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

/// Parse a message; `None` for anything else that arrives on the port.
pub fn parse<T: for<'de> Deserialize<'de>>(text: &str) -> Option<T> {
    serde_json::from_str(text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use infrasonic_engine::{PlayState, RepeatMode, Track};

    #[test]
    fn messages_roundtrip() {
        let mut t = Track::new("id1", "Song");
        t.duration = Some(3.5);
        let msgs = [
            ToWorker::Init {
                sample_rate: 48_000,
                volume: 80,
                replay_gain: ReplayGainConfig::default(),
            },
            ToWorker::Urls {
                urls: vec![("id1".into(), "https://x/rest/stream?id=id1".into())],
            },
            ToWorker::Command {
                cmd: Command::SetQueue {
                    tracks: vec![t.clone()],
                    start: 0,
                    position: 1.5,
                    play: true,
                },
            },
            ToWorker::Command {
                cmd: Command::SetRepeat(RepeatMode::All),
            },
            ToWorker::Command {
                cmd: Command::Seek(10.0),
            },
        ];
        for m in msgs {
            assert_eq!(parse::<ToWorker>(&m.to_json()), Some(m));
        }
        let back = FromWorker::Event {
            event: Event::StateChanged(PlayState::Playing),
        };
        assert_eq!(parse::<FromWorker>(&back.to_json()), Some(back));
        assert_eq!(parse::<ToWorker>("{\"t\":\"nope\"}"), None);
    }
}
