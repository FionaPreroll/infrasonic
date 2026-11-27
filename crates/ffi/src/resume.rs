// SPDX-License-Identifier: GPL-3.0-only
//! Where long tracks were left (podcasts, audio books), per server, so that they start there
//! again. The engine decides which tracks count; this keeps its notes across restarts.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The most tracks remembered; the oldest are forgotten first.
const MAX_POINTS: usize = 500;

#[derive(Debug, Default, Serialize, Deserialize)]
struct Stored {
    /// Oldest first.
    points: Vec<(String, f64)>,
}

#[derive(Debug)]
pub(crate) struct ResumeStore {
    path: PathBuf,
    points: Vec<(String, f64)>,
}

impl ResumeStore {
    pub(crate) fn load(path: PathBuf) -> Self {
        let points = crate::app::read_json::<Stored>(&path)
            .map(|s| s.points)
            .unwrap_or_default()
            .into_iter()
            .filter(|(_, p)| p.is_finite() && *p > 0.0)
            .collect();
        Self { path, points }
    }

    pub(crate) fn points(&self) -> Vec<(String, f64)> {
        self.points.clone()
    }

    pub(crate) fn get(&self, id: &str) -> Option<f64> {
        self.points.iter().find(|(i, _)| i == id).map(|(_, p)| *p)
    }

    /// Note a new position (or forget the track) and save.
    pub(crate) fn set(&mut self, id: &str, position: Option<f64>) {
        self.points.retain(|(i, _)| i != id);
        if let Some(p) = position.filter(|p| p.is_finite() && *p > 0.0) {
            self.points.push((id.to_string(), p));
            if self.points.len() > MAX_POINTS {
                let extra = self.points.len() - MAX_POINTS;
                self.points.drain(..extra);
            }
        }
        let stored = Stored {
            points: self.points.clone(),
        };
        if let Err(e) = crate::app::write_json(&self.path, &stored) {
            tracing::warn!("{e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn points_are_saved_and_capped() {
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("r/x.json");
        let mut s = ResumeStore::load(path.clone());
        s.set("a", Some(100.0));
        s.set("b", Some(f64::NAN));
        s.set("c", Some(5.0));
        s.set("a", Some(200.0));
        s.set("c", None);
        let again = ResumeStore::load(path.clone());
        assert_eq!(again.points(), vec![("a".to_string(), 200.0)]);
        let mut s = again;
        for i in 0..MAX_POINTS + 10 {
            s.set(&i.to_string(), Some(1.0));
        }
        assert_eq!(s.points().len(), MAX_POINTS);
        assert_eq!(s.get("a"), None, "the oldest goes first");
        assert_eq!(s.get(&(MAX_POINTS + 9).to_string()), Some(1.0));
    }
}
