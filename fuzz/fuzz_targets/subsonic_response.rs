// SPDX-License-Identifier: GPL-3.0-only
//! Arbitrary server responses must never panic the parser.
#![no_main]

use infrasonic_subsonic::{
    Album, Artist, Artists, Directory, Indexes, PlayQueue, Playlist, SearchResult3, Starred2,
    parse_response,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some((&sel, body)) = data.split_first() else {
        return;
    };
    let required = sel & 0x80 != 0;
    match sel % 10 {
        0 => drop(parse_response::<()>("ping", body, &[], required)),
        1 => drop(parse_response::<Indexes>("getIndexes", body, &["indexes"], required)),
        2 => drop(parse_response::<Directory>("getMusicDirectory", body, &["directory"], required)),
        3 => drop(parse_response::<Artists>("getArtists", body, &["artists"], required)),
        4 => drop(parse_response::<Artist>("getArtist", body, &["artist"], required)),
        5 => drop(parse_response::<Album>("getAlbum", body, &["album"], required)),
        6 => drop(parse_response::<SearchResult3>("search3", body, &["searchResult3"], required)),
        7 => drop(parse_response::<Playlist>("getPlaylist", body, &["playlist"], required)),
        8 => drop(parse_response::<PlayQueue>("getPlayQueue", body, &["playQueue"], required)),
        _ => drop(parse_response::<Starred2>("getStarred2", body, &["starred2"], required)),
    }
});
