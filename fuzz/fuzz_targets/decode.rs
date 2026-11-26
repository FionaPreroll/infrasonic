// SPDX-License-Identifier: GPL-3.0-only
//! Broken or hostile audio files must produce errors, not panics or hangs.
#![no_main]

use infrasonic_engine::decode::TrackDecoder;
use infrasonic_engine::source::StreamReader;
use libfuzzer_sys::fuzz_target;

const HINTS: [Option<&str>; 9] = [
    None,
    Some("mp3"),
    Some("flac"),
    Some("ogg"),
    Some("opus"),
    Some("m4a"),
    Some("wav"),
    Some("audio/aac"),
    Some("mka"),
];

fuzz_target!(|data: &[u8]| {
    let Some((&sel, body)) = data.split_first() else {
        return;
    };
    let hint = HINTS[usize::from(sel) % HINTS.len()];
    let Ok(mut dec) = TrackDecoder::open(Box::new(StreamReader::from_bytes(body.to_vec())), hint)
    else {
        return;
    };
    // Bound the work per input; a tiny file must not decode forever.
    let mut chunks = 0;
    while let Ok(Some(chunk)) = dec.next_chunk() {
        assert!(chunk.channels > 0);
        assert_eq!(chunk.samples.len() % chunk.channels, 0);
        chunks += 1;
        if chunks > 2000 {
            break;
        }
        if sel & 0x80 != 0 && chunks == 2 {
            let _ = dec.seek(0.0);
        }
    }
});
