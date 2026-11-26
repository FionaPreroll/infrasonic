// SPDX-License-Identifier: GPL-3.0-only
//! Any sequence of queue edits keeps the queue consistent.
#![no_main]

use arbitrary::Arbitrary;
use infrasonic_engine::{Queue, RepeatMode, Track};
use libfuzzer_sys::fuzz_target;

#[derive(Debug, Arbitrary)]
enum Op {
    Set(u8, Option<u8>),
    Append(u8),
    InsertNext(u8),
    Remove(u8),
    Clear,
    Move(u8, u8),
    Shuffle,
    SetCurrent(Option<u8>),
    Repeat(u8),
    Next(bool),
    Prev,
}

fn tracks(n: u8) -> Vec<Track> {
    (0..n % 16).map(|i| Track::new(i.to_string(), format!("t{i}"))).collect()
}

fuzz_target!(|ops: Vec<Op>| {
    let mut q = Queue::new();
    for op in ops {
        match op {
            Op::Set(n, c) => q.set(tracks(n), c.map(usize::from)),
            Op::Append(n) => q.append(tracks(n)),
            Op::InsertNext(n) => q.insert_next(tracks(n)),
            Op::Remove(i) => drop(q.remove(usize::from(i))),
            Op::Clear => q.clear(),
            Op::Move(a, b) => drop(q.move_entry(usize::from(a), usize::from(b))),
            Op::Shuffle => q.shuffle(),
            Op::SetCurrent(i) => drop(q.set_current(i.map(usize::from))),
            Op::Repeat(r) => q.set_repeat(match r % 3 {
                0 => RepeatMode::Off,
                1 => RepeatMode::All,
                _ => RepeatMode::One,
            }),
            Op::Next(auto) => {
                if let Some(i) = q.next_index(q.current_index(), auto) {
                    assert!(i < q.len());
                }
            }
            Op::Prev => {
                if let Some(i) = q.prev_index(q.current_index()) {
                    assert!(i < q.len());
                }
            }
        }
        q.check_invariants().expect("queue invariants");
    }
});
