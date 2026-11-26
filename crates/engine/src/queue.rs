// SPDX-License-Identifier: GPL-3.0-only
//! The play queue: an ordered list of entries with a current position and repeat mode.
//!
//! Every entry gets a unique id when it is added, so the playing entry keeps its identity when
//! the queue is reordered.

use rand::seq::SliceRandom;

use crate::Track;

/// Repeat behaviour when a track ends.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum RepeatMode {
    #[default]
    Off,
    /// Start over at the first entry after the last one.
    All,
    /// Repeat the current entry.
    One,
}

impl RepeatMode {
    /// Off → All → One → Off.
    pub fn cycle(self) -> Self {
        match self {
            Self::Off => Self::All,
            Self::All => Self::One,
            Self::One => Self::Off,
        }
    }
}

/// A track in the queue.
#[derive(Debug, Clone, PartialEq)]
pub struct QueueEntry {
    /// Unique within the lifetime of the queue.
    pub uid: u64,
    pub track: Track,
}

/// The play queue.
#[derive(Debug, Clone, Default)]
pub struct Queue {
    entries: Vec<QueueEntry>,
    current: Option<usize>,
    next_uid: u64,
    repeat: RepeatMode,
}

impl Queue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn entries(&self) -> &[QueueEntry] {
        &self.entries
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn current_index(&self) -> Option<usize> {
        self.current
    }
    pub fn current(&self) -> Option<&QueueEntry> {
        self.current.and_then(|i| self.entries.get(i))
    }
    pub fn get(&self, index: usize) -> Option<&QueueEntry> {
        self.entries.get(index)
    }
    pub fn repeat(&self) -> RepeatMode {
        self.repeat
    }
    pub fn set_repeat(&mut self, r: RepeatMode) {
        self.repeat = r;
    }

    /// Index of the entry with this uid.
    pub fn index_of(&self, uid: u64) -> Option<usize> {
        self.entries.iter().position(|e| e.uid == uid)
    }

    fn wrap(&mut self, tracks: Vec<Track>) -> Vec<QueueEntry> {
        tracks
            .into_iter()
            .map(|track| {
                self.next_uid += 1;
                QueueEntry {
                    uid: self.next_uid,
                    track,
                }
            })
            .collect()
    }

    /// Replace everything. `current` is clamped to the new length.
    pub fn set(&mut self, tracks: Vec<Track>, current: Option<usize>) {
        self.entries = self.wrap(tracks);
        self.current = current.filter(|i| *i < self.entries.len());
    }

    /// Append at the end.
    pub fn append(&mut self, tracks: Vec<Track>) {
        let e = self.wrap(tracks);
        self.entries.extend(e);
    }

    /// Insert right after the current entry (or at the front if nothing is current).
    pub fn insert_next(&mut self, tracks: Vec<Track>) {
        let at = self.current.map_or(0, |c| c + 1);
        let e = self.wrap(tracks);
        self.entries.splice(at..at, e);
    }

    /// Remove an entry. Returns it if the index was valid.
    pub fn remove(&mut self, index: usize) -> Option<QueueEntry> {
        if index >= self.entries.len() {
            return None;
        }
        let e = self.entries.remove(index);
        self.current = match self.current {
            Some(c) if c == index => None,
            Some(c) if c > index => Some(c - 1),
            other => other,
        };
        Some(e)
    }

    /// Remove all entries.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.current = None;
    }

    /// Move an entry; the current entry stays current.
    pub fn move_entry(&mut self, from: usize, to: usize) -> bool {
        let len = self.entries.len();
        if from >= len || to >= len {
            return false;
        }
        if from == to {
            return true;
        }
        let cur_uid = self.current().map(|e| e.uid);
        let e = self.entries.remove(from);
        self.entries.insert(to, e);
        self.current = cur_uid.and_then(|u| self.index_of(u));
        true
    }

    /// Shuffle all entries. The current entry moves to the front and stays current.
    pub fn shuffle(&mut self) {
        let mut rng = rand::rng();
        match self.current {
            Some(c) => {
                let cur = self.entries.remove(c);
                self.entries.shuffle(&mut rng);
                self.entries.insert(0, cur);
                self.current = Some(0);
            }
            None => self.entries.shuffle(&mut rng),
        }
    }

    /// Make `index` current. Returns false if out of range.
    pub fn set_current(&mut self, index: Option<usize>) -> bool {
        match index {
            Some(i) if i >= self.entries.len() => false,
            other => {
                self.current = other;
                true
            }
        }
    }

    /// Entry that follows `from`. `auto` is true when a track ended on its own (then
    /// `RepeatMode::One` repeats it), false for an explicit "next".
    pub fn next_index(&self, from: Option<usize>, auto: bool) -> Option<usize> {
        let len = self.entries.len();
        if len == 0 {
            return None;
        }
        let Some(f) = from.filter(|f| *f < len) else {
            return Some(0);
        };
        if auto && self.repeat == RepeatMode::One {
            return Some(f);
        }
        if f + 1 < len {
            Some(f + 1)
        } else if self.repeat == RepeatMode::Off {
            None
        } else {
            Some(0)
        }
    }

    /// Entry before `from`, wrapping only with repeat all.
    pub fn prev_index(&self, from: Option<usize>) -> Option<usize> {
        let len = self.entries.len();
        let f = from.filter(|f| *f < len)?;
        if f > 0 {
            Some(f - 1)
        } else if self.repeat == RepeatMode::All {
            Some(len - 1)
        } else {
            Some(0)
        }
    }

    /// Replace the metadata of all entries with `track.id`. Returns whether anything changed.
    pub fn update_tracks(&mut self, track: &Track) -> bool {
        let mut changed = false;
        for e in &mut self.entries {
            if e.track.id == track.id {
                e.track = track.clone();
                changed = true;
            }
        }
        changed
    }

    /// Check internal invariants (used by tests and fuzzing).
    pub fn check_invariants(&self) -> Result<(), String> {
        if let Some(c) = self.current
            && c >= self.entries.len()
        {
            return Err(format!("current {c} out of range {}", self.entries.len()));
        }
        let mut uids: Vec<u64> = self.entries.iter().map(|e| e.uid).collect();
        uids.sort_unstable();
        uids.dedup();
        if uids.len() != self.entries.len() {
            return Err("duplicate uid".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn t(id: &str) -> Track {
        Track::new(id, id)
    }

    fn ids(q: &Queue) -> Vec<String> {
        q.entries().iter().map(|e| e.track.id.clone()).collect()
    }

    #[test]
    fn insert_next_goes_after_current() {
        let mut q = Queue::new();
        q.set(vec![t("a"), t("b"), t("c")], Some(1));
        q.insert_next(vec![t("x"), t("y")]);
        assert_eq!(ids(&q), ["a", "b", "x", "y", "c"]);
        assert_eq!(q.current_index(), Some(1));
    }

    #[test]
    fn removing_before_current_shifts_index() {
        let mut q = Queue::new();
        q.set(vec![t("a"), t("b"), t("c")], Some(2));
        q.remove(0);
        assert_eq!(q.current().map(|e| e.track.id.as_str()), Some("c"));
        q.remove(1);
        assert_eq!(q.current_index(), None);
    }

    #[test]
    fn move_keeps_current_entry() {
        let mut q = Queue::new();
        q.set(vec![t("a"), t("b"), t("c")], Some(0));
        assert!(q.move_entry(0, 2));
        assert_eq!(ids(&q), ["b", "c", "a"]);
        assert_eq!(q.current_index(), Some(2));
        assert!(!q.move_entry(0, 3));
    }

    #[test]
    fn repeat_modes() {
        let mut q = Queue::new();
        q.set(vec![t("a"), t("b")], Some(1));
        assert_eq!(q.next_index(Some(1), true), None);
        q.set_repeat(RepeatMode::All);
        assert_eq!(q.next_index(Some(1), true), Some(0));
        assert_eq!(q.prev_index(Some(0)), Some(1));
        q.set_repeat(RepeatMode::One);
        assert_eq!(q.next_index(Some(1), true), Some(1));
        assert_eq!(q.next_index(Some(0), false), Some(1));
        assert_eq!(RepeatMode::One.cycle(), RepeatMode::Off);
    }

    #[test]
    fn shuffle_keeps_current_first() {
        let mut q = Queue::new();
        q.set((0..50).map(|i| t(&i.to_string())).collect(), Some(17));
        q.shuffle();
        assert_eq!(q.current_index(), Some(0));
        assert_eq!(q.entries()[0].track.id, "17");
        let mut all = ids(&q);
        all.sort();
        let mut want: Vec<String> = (0..50).map(|i| i.to_string()).collect();
        want.sort();
        assert_eq!(all, want);
    }

    #[derive(Debug, Clone)]
    enum Op {
        Set(u8, Option<u8>),
        Append(u8),
        InsertNext(u8),
        Remove(u8),
        Move(u8, u8),
        Shuffle,
        SetCurrent(Option<u8>),
        Clear,
        Repeat,
    }

    fn op() -> impl Strategy<Value = Op> {
        prop_oneof![
            (0u8..20, proptest::option::of(0u8..25)).prop_map(|(n, c)| Op::Set(n, c)),
            (0u8..5).prop_map(Op::Append),
            (0u8..5).prop_map(Op::InsertNext),
            (0u8..25).prop_map(Op::Remove),
            (0u8..25, 0u8..25).prop_map(|(a, b)| Op::Move(a, b)),
            Just(Op::Shuffle),
            proptest::option::of(0u8..25).prop_map(Op::SetCurrent),
            Just(Op::Clear),
            Just(Op::Repeat),
        ]
    }

    proptest! {
        #[test]
        fn invariants_hold_for_any_operation_sequence(ops in proptest::collection::vec(op(), 0..60)) {
            let mut q = Queue::new();
            for o in ops {
                let cur_uid = q.current().map(|e| e.uid);
                match o {
                    Op::Set(n, c) => q.set((0..n).map(|i| t(&i.to_string())).collect(), c.map(usize::from)),
                    Op::Append(n) => q.append((0..n).map(|i| t(&i.to_string())).collect()),
                    Op::InsertNext(n) => q.insert_next((0..n).map(|i| t(&i.to_string())).collect()),
                    Op::Remove(i) => { q.remove(usize::from(i)); }
                    Op::Move(a, b) => {
                        q.move_entry(usize::from(a), usize::from(b));
                        prop_assert_eq!(q.current().map(|e| e.uid), cur_uid);
                    }
                    Op::Shuffle => {
                        q.shuffle();
                        prop_assert_eq!(q.current().map(|e| e.uid), cur_uid);
                    }
                    Op::SetCurrent(c) => { q.set_current(c.map(usize::from)); }
                    Op::Clear => q.clear(),
                    Op::Repeat => { let r = q.repeat().cycle(); q.set_repeat(r); }
                }
                prop_assert!(q.check_invariants().is_ok(), "{:?}", q.check_invariants());
                if let Some(n) = q.next_index(q.current_index(), true) {
                    prop_assert!(n < q.len());
                }
                if let Some(p) = q.prev_index(q.current_index()) {
                    prop_assert!(p < q.len());
                }
            }
        }
    }
}
