// SPDX-License-Identifier: GPL-3.0-only
//! When to send "now playing" and when to submit a play, following the common last.fm rules:
//! a track counts once it is longer than 30 seconds and was listened to for half its length or
//! four minutes, whichever comes first. Seeking does not count as listening.

/// What the caller should send to the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScrobbleAction {
    NowPlaying { id: String },
    Submit { id: String, time_ms: u64 },
}

#[derive(Debug, Clone)]
struct Current {
    id: String,
    duration: Option<f64>,
    started_ms: u64,
    listened: f64,
    last_pos: Option<f64>,
    submitted: bool,
}

/// Tracks listening time of the current track.
#[derive(Debug, Clone, Default)]
pub struct ScrobbleTracker {
    current: Option<Current>,
}

/// Largest position step that still counts as continuous playback (seconds).
const MAX_STEP: f64 = 2.5;

impl ScrobbleTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// A new track started (or the same one again). `None` means playback stopped.
    pub fn track_changed(
        &mut self,
        id: Option<&str>,
        duration: Option<f64>,
        now_ms: u64,
    ) -> Option<ScrobbleAction> {
        self.current = id.map(|id| Current {
            id: id.to_string(),
            duration,
            started_ms: now_ms,
            listened: 0.0,
            last_pos: None,
            submitted: false,
        });
        id.map(|id| ScrobbleAction::NowPlaying { id: id.to_string() })
    }

    /// Playback position update.
    pub fn position(&mut self, pos: f64) -> Option<ScrobbleAction> {
        let c = self.current.as_mut()?;
        if let Some(last) = c.last_pos {
            let step = pos - last;
            if step > 0.0 && step <= MAX_STEP {
                c.listened += step;
            }
        }
        c.last_pos = Some(pos);
        if c.submitted {
            return None;
        }
        let needed = match c.duration {
            Some(d) if d < 30.0 => return None,
            Some(d) => (d / 2.0).min(240.0),
            None => 240.0,
        };
        if c.listened >= needed {
            c.submitted = true;
            return Some(ScrobbleAction::Submit {
                id: c.id.clone(),
                time_ms: c.started_ms,
            });
        }
        None
    }

    /// Seconds listened to the current track.
    pub fn listened(&self) -> f64 {
        self.current.as_ref().map_or(0.0, |c| c.listened)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn play(t: &mut ScrobbleTracker, from: f64, to: f64) -> Vec<ScrobbleAction> {
        let mut out = Vec::new();
        let mut p = from;
        while p <= to {
            out.extend(t.position(p));
            p += 0.5;
        }
        out
    }

    #[test]
    fn submits_after_half() {
        let mut t = ScrobbleTracker::new();
        assert_eq!(
            t.track_changed(Some("a"), Some(100.0), 7),
            Some(ScrobbleAction::NowPlaying { id: "a".into() })
        );
        assert!(play(&mut t, 0.0, 49.0).is_empty());
        assert_eq!(
            play(&mut t, 49.5, 60.0),
            vec![ScrobbleAction::Submit {
                id: "a".into(),
                time_ms: 7
            }]
        );
        assert!(play(&mut t, 60.5, 100.0).is_empty(), "only once");
    }

    #[test]
    fn long_tracks_need_four_minutes() {
        let mut t = ScrobbleTracker::new();
        t.track_changed(Some("a"), Some(3600.0), 0);
        assert!(play(&mut t, 0.0, 239.0).is_empty());
        assert_eq!(play(&mut t, 239.5, 241.0).len(), 1);
    }

    #[test]
    fn seeking_does_not_count() {
        let mut t = ScrobbleTracker::new();
        t.track_changed(Some("a"), Some(100.0), 0);
        play(&mut t, 0.0, 10.0);
        assert!(play(&mut t, 90.0, 100.0).is_empty());
        assert!((t.listened() - 20.0).abs() < 1.0);
    }

    #[test]
    fn short_tracks_are_never_submitted() {
        let mut t = ScrobbleTracker::new();
        t.track_changed(Some("a"), Some(20.0), 0);
        assert!(play(&mut t, 0.0, 20.0).is_empty());
    }

    #[test]
    fn stop_clears_state() {
        let mut t = ScrobbleTracker::new();
        t.track_changed(Some("a"), Some(100.0), 0);
        assert_eq!(t.track_changed(None, None, 0), None);
        assert!(play(&mut t, 0.0, 100.0).is_empty());
    }
}
