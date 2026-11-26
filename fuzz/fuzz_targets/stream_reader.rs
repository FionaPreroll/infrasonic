// SPDX-License-Identifier: GPL-3.0-only
//! The streaming buffer behaves like a file for any mix of writes, reads and seeks.
#![no_main]

use std::io::{Read, Seek, SeekFrom};

use arbitrary::Arbitrary;
use infrasonic_engine::source::stream_pair;
use libfuzzer_sys::fuzz_target;

#[derive(Debug, Arbitrary)]
enum Op {
    Read(u16),
    SeekStart(u32),
    SeekCurrent(i32),
}

#[derive(Debug, Arbitrary)]
struct Input {
    data: Vec<u8>,
    chunks: Vec<u8>,
    ops: Vec<Op>,
}

fuzz_target!(|input: Input| {
    let (writer, mut reader) = stream_pair(Some(input.data.len() as u64));
    // Write everything up front in arbitrary chunk sizes, then finish.
    let mut rest = input.data.as_slice();
    for c in input.chunks.iter().map(|c| usize::from(*c).max(1)) {
        if rest.is_empty() {
            break;
        }
        let n = c.min(rest.len());
        assert!(writer.push(&rest[..n]));
        rest = &rest[n..];
    }
    if !rest.is_empty() {
        assert!(writer.push(rest));
    }
    writer.finish();

    let mut pos: u64 = 0;
    let len = input.data.len() as u64;
    for op in input.ops {
        match op {
            Op::Read(n) => {
                let mut buf = vec![0u8; usize::from(n)];
                let got = reader.read(&mut buf).expect("read");
                let start = pos.min(len) as usize;
                let want = (len.saturating_sub(pos) as usize).min(buf.len());
                assert_eq!(got, want);
                assert_eq!(&buf[..got], &input.data[start..start + got]);
                pos += got as u64;
            }
            Op::SeekStart(p) => {
                pos = reader.seek(SeekFrom::Start(u64::from(p))).expect("seek");
                assert_eq!(pos, u64::from(p));
            }
            Op::SeekCurrent(d) => match reader.seek(SeekFrom::Current(i64::from(d))) {
                Ok(p) => {
                    assert_eq!(p as i64, pos as i64 + i64::from(d));
                    pos = p;
                }
                Err(_) => assert!(pos as i64 + i64::from(d) < 0),
            },
        }
    }
});
