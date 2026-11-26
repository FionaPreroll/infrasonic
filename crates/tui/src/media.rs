// SPDX-License-Identifier: GPL-3.0-only
//! System media controls: MPRIS on Linux, Now Playing on macOS, SMTC on Windows.

use infrasonic_core::engine::Command;

/// What a media key asks for.
#[derive(Debug, Clone, PartialEq)]
pub enum MediaAction {
    Command(Command),
    TogglePause,
    Quit,
}

#[cfg(feature = "media-controls")]
mod imp {
    use std::path::Path;
    use std::time::Duration;

    use infrasonic_core::engine::{Command, PlayState, Snapshot};
    use souvlaki::{
        MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition,
        PlatformConfig, SeekDirection,
    };
    use tokio::sync::mpsc::UnboundedSender;

    use super::MediaAction;
    use crate::app::Msg;

    /// Connection to the OS media controls.
    pub struct Media {
        controls: MediaControls,
        last_state: Option<(PlayState, u64)>,
    }

    impl std::fmt::Debug for Media {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("Media").finish_non_exhaustive()
        }
    }

    fn map(ev: MediaControlEvent) -> Option<MediaAction> {
        Some(match ev {
            MediaControlEvent::Play => MediaAction::Command(Command::Play),
            MediaControlEvent::Pause => MediaAction::Command(Command::Pause),
            MediaControlEvent::Toggle => MediaAction::TogglePause,
            MediaControlEvent::Next => MediaAction::Command(Command::Next),
            MediaControlEvent::Previous => MediaAction::Command(Command::Previous),
            MediaControlEvent::Stop => MediaAction::Command(Command::Stop),
            MediaControlEvent::Seek(d) => MediaAction::Command(Command::SeekBy(match d {
                SeekDirection::Forward => 10.0,
                SeekDirection::Backward => -10.0,
            })),
            MediaControlEvent::SeekBy(d, by) => {
                let s = by.as_secs_f64();
                MediaAction::Command(Command::SeekBy(match d {
                    SeekDirection::Forward => s,
                    SeekDirection::Backward => -s,
                }))
            }
            MediaControlEvent::SetPosition(MediaPosition(p)) => {
                MediaAction::Command(Command::Seek(p.as_secs_f64()))
            }
            MediaControlEvent::SetVolume(v) => {
                MediaAction::Command(Command::SetVolume((v.clamp(0.0, 1.0) * 100.0) as u8))
            }
            MediaControlEvent::Quit => MediaAction::Quit,
            _ => return None,
        })
    }

    /// souvlaki panics in its own thread when there is no session bus, so check first.
    #[cfg(all(unix, not(target_os = "macos")))]
    fn session_bus_available() -> bool {
        match std::env::var("DBUS_SESSION_BUS_ADDRESS") {
            Ok(addr) => addr
                .split(';')
                .filter_map(|a| a.strip_prefix("unix:path="))
                .all(|p| std::path::Path::new(p.split(',').next().unwrap_or(p)).exists()),
            Err(_) => std::env::var("XDG_RUNTIME_DIR")
                .map(|d| std::path::Path::new(&d).join("bus").exists())
                .unwrap_or(false),
        }
    }

    impl Media {
        /// Register with the OS. Fails without a session bus on Linux, for example.
        pub fn new(tx: UnboundedSender<Msg>) -> Result<Self, String> {
            #[cfg(all(unix, not(target_os = "macos")))]
            if !session_bus_available() {
                return Err("no D-Bus session bus".into());
            }
            let config = PlatformConfig {
                display_name: "infrasonic",
                dbus_name: "infrasonic",
                hwnd: None,
            };
            let mut controls = MediaControls::new(config).map_err(|e| format!("{e:?}"))?;
            controls
                .attach(move |ev| {
                    if let Some(a) = map(ev) {
                        let _ = tx.send(Msg::Media(a));
                    }
                })
                .map_err(|e| format!("{e:?}"))?;
            Ok(Self {
                controls,
                last_state: None,
            })
        }

        /// New song, or its cover file became available. The cover is passed as a local file
        /// so that no authenticated server URL leaks to other programs.
        pub fn track_changed(&mut self, snap: &Snapshot, cover: Option<&Path>) {
            let Some(cur) = &snap.current else {
                let _ = self.controls.set_metadata(MediaMetadata::default());
                return;
            };
            let t = &cur.track;
            let cover = cover.map(|p| format!("file://{}", p.display()));
            let _ = self.controls.set_metadata(MediaMetadata {
                title: Some(&t.title),
                album: Some(&t.album),
                artist: Some(&t.artist),
                cover_url: cover.as_deref(),
                duration: t.duration.map(Duration::from_secs_f64),
            });
            self.last_state = None;
            self.state_changed(snap);
        }

        pub fn state_changed(&mut self, snap: &Snapshot) {
            // Update on state changes and every few seconds for the position.
            let key = (snap.state, (snap.position / 5.0) as u64);
            if self.last_state == Some(key) {
                return;
            }
            self.last_state = Some(key);
            let progress = Some(MediaPosition(Duration::from_secs_f64(
                snap.position.max(0.0),
            )));
            let playback = match snap.state {
                PlayState::Playing | PlayState::Buffering => MediaPlayback::Playing { progress },
                PlayState::Paused => MediaPlayback::Paused { progress },
                PlayState::Stopped => MediaPlayback::Stopped,
            };
            let _ = self.controls.set_playback(playback);
        }
    }
}

#[cfg(not(feature = "media-controls"))]
mod imp {
    use std::path::Path;

    use infrasonic_core::engine::Snapshot;
    use tokio::sync::mpsc::UnboundedSender;

    use crate::app::Msg;

    #[derive(Debug)]
    pub struct Media;

    impl Media {
        pub fn new(_tx: UnboundedSender<Msg>) -> Result<Self, String> {
            Err("built without media controls".into())
        }
        pub fn track_changed(&mut self, _: &Snapshot, _: Option<&Path>) {}
        pub fn state_changed(&mut self, _: &Snapshot) {}
    }
}

pub use imp::Media;
