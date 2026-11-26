// SPDX-License-Identifier: GPL-3.0-only
//! Configuration parsing: no panics, and whatever parses survives a round trip.
#![no_main]

use std::path::Path;

use infrasonic_core::Config;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let origin = Path::new("fuzz.toml");
    if let Ok(c) = Config::parse(text, origin) {
        let again = Config::parse(&c.to_toml(), origin).expect("serialized config parses");
        assert_eq!(c, again);
    }
    if let Ok(c) = Config::from_stmps(text, origin) {
        let again = Config::parse(&c.to_toml(), origin).expect("imported config serializes");
        assert_eq!(c, again);
    }
});
