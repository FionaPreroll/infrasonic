// SPDX-License-Identifier: GPL-3.0-only
//! Encoder delay and padding of MP4/M4A files from the edit list (`moov/trak/edts/elst`).
//!
//! Symphonia's MP4 reader ignores edit lists, so AAC files would start with the encoder's
//! priming samples (usually 1024 or 2112 frames) and end with padding, which leaves an audible
//! gap between tracks. This module reads just enough of the container to trim both.

use std::io::{Read, Seek, SeekFrom};

/// Largest `moov` box we read into memory.
const MAX_MOOV: u64 = 32 << 20;

/// Playable range of the first audio track, in units of its media time scale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edit {
    /// Media time of the first audible sample (the encoder delay).
    pub start: u64,
    /// Number of audible samples, if the edit says so.
    pub duration: Option<u64>,
    /// Media time scale (samples per second for audio).
    pub timescale: u32,
}

/// Read the edit of the first audio track. Leaves the stream at position 0.
/// Returns `None` for anything that is not a well-formed MP4 file with an edit list.
pub fn read_edit<R: Read + Seek + ?Sized>(r: &mut R) -> Option<Edit> {
    let res = find_moov(r).and_then(|moov| parse_moov(&moov));
    r.seek(SeekFrom::Start(0)).ok()?;
    res
}

fn find_moov<R: Read + Seek + ?Sized>(r: &mut R) -> Option<Vec<u8>> {
    r.seek(SeekFrom::Start(0)).ok()?;
    let mut pos = 0u64;
    let mut first = true;
    loop {
        let mut h = [0u8; 8];
        r.read_exact(&mut h).ok()?;
        let mut size = u64::from(u32::from_be_bytes([h[0], h[1], h[2], h[3]]));
        let kind = [h[4], h[5], h[6], h[7]];
        let mut header = 8;
        if first && &kind != b"ftyp" {
            return None;
        }
        first = false;
        if size == 1 {
            let mut l = [0u8; 8];
            r.read_exact(&mut l).ok()?;
            size = u64::from_be_bytes(l);
            header = 16;
        }
        if size == 0 || size < header {
            return None;
        }
        if &kind == b"moov" {
            let len = size - header;
            if len > MAX_MOOV {
                return None;
            }
            let mut buf = vec![0u8; usize::try_from(len).ok()?];
            r.read_exact(&mut buf).ok()?;
            return Some(buf);
        }
        pos = pos.checked_add(size)?;
        r.seek(SeekFrom::Start(pos)).ok()?;
    }
}

/// Child boxes of `data`: (type, payload).
fn boxes(mut data: &[u8]) -> impl Iterator<Item = ([u8; 4], &[u8])> {
    std::iter::from_fn(move || {
        if data.len() < 8 {
            return None;
        }
        let size = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
        let kind = [data[4], data[5], data[6], data[7]];
        let (header, size) = match size {
            0 => (8, data.len()),
            1 => {
                let l = u64::from_be_bytes(data.get(8..16)?.try_into().ok()?);
                (16, usize::try_from(l).ok()?)
            }
            s => (8, s),
        };
        if size < header || size > data.len() {
            return None;
        }
        let payload = &data[header..size];
        data = &data[size..];
        Some((kind, payload))
    })
}

fn child<'a>(data: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    boxes(data).find(|(k, _)| k == kind).map(|(_, p)| p)
}

fn be32(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(at..at + 4)?.try_into().ok()?))
}

fn be64(d: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_be_bytes(d.get(at..at + 8)?.try_into().ok()?))
}

/// Time scale of an `mvhd` or `mdhd` payload.
fn timescale(d: &[u8]) -> Option<u32> {
    match d.first()? {
        0 => be32(d, 12),
        1 => be32(d, 20),
        _ => None,
    }
}

fn parse_moov(moov: &[u8]) -> Option<Edit> {
    let movie_ts = timescale(child(moov, b"mvhd")?)?;
    let trak = boxes(moov).filter(|(k, _)| k == b"trak").find(|(_, t)| {
        child(t, b"mdia")
            .and_then(|m| child(m, b"hdlr"))
            .and_then(|h| h.get(8..12))
            == Some(b"soun".as_slice())
    })?;
    let trak = trak.1;
    let media_ts = timescale(child(child(trak, b"mdia")?, b"mdhd")?)?;
    let elst = child(child(trak, b"edts")?, b"elst")?;
    let version = *elst.first()?;
    let count = be32(elst, 4)? as usize;
    let entry_len = if version == 1 { 20 } else { 12 };
    let mut start = None;
    let mut duration = 0u64;
    for i in 0..count.min(64) {
        let at = 8 + i * entry_len;
        let (seg, media_time) = if version == 1 {
            (be64(elst, at)?, be64(elst, at + 8)? as i64)
        } else {
            (
                u64::from(be32(elst, at)?),
                i64::from(be32(elst, at + 4)? as i32),
            )
        };
        if media_time < 0 {
            // An empty edit (a pause before the track starts): nothing to trim.
            continue;
        }
        if start.is_none() {
            start = Some(media_time as u64);
        }
        duration = duration.saturating_add(seg);
    }
    if movie_ts == 0 || media_ts == 0 {
        return None;
    }
    let duration = (duration > 0)
        .then(|| (u128::from(duration) * u128::from(media_ts) / u128::from(movie_ts)) as u64);
    Some(Edit {
        start: start?,
        duration,
        timescale: media_ts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn bx(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut v = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(kind);
        v.extend_from_slice(payload);
        v
    }

    fn header(ts: u32) -> Vec<u8> {
        let mut v = vec![0u8; 12];
        v.extend_from_slice(&ts.to_be_bytes());
        v.extend_from_slice(&[0; 4]);
        v
    }

    fn file(elst: &[u8]) -> Vec<u8> {
        let mut hdlr = vec![0u8; 8];
        hdlr.extend_from_slice(b"soun");
        let mdia = [bx(b"mdhd", &header(44_100)), bx(b"hdlr", &hdlr)].concat();
        let trak = [bx(b"edts", &bx(b"elst", elst)), bx(b"mdia", &mdia)].concat();
        let moov = [bx(b"mvhd", &header(1000)), bx(b"trak", &trak)].concat();
        [
            bx(b"ftyp", b"M4A \0\0\0\0"),
            bx(b"mdat", &[0; 100]),
            bx(b"moov", &moov),
        ]
        .concat()
    }

    #[test]
    fn reads_delay_and_duration() {
        let mut elst = vec![0, 0, 0, 0, 0, 0, 0, 1];
        elst.extend_from_slice(&2000u32.to_be_bytes()); // 2 s in movie time
        elst.extend_from_slice(&1024i32.to_be_bytes());
        elst.extend_from_slice(&0x0001_0000u32.to_be_bytes());
        let mut c = Cursor::new(file(&elst));
        let e = read_edit(&mut c).expect("edit");
        assert_eq!(
            e,
            Edit {
                start: 1024,
                duration: Some(88_200),
                timescale: 44_100
            }
        );
        assert_eq!(c.position(), 0);
    }

    #[test]
    fn rejects_other_files() {
        assert_eq!(
            read_edit(&mut Cursor::new(b"ID3\x04garbage".to_vec())),
            None
        );
        assert_eq!(read_edit(&mut Cursor::new(Vec::new())), None);
        let mut f = file(&[0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff]);
        f.truncate(f.len() - 3);
        assert_eq!(read_edit(&mut Cursor::new(f)), None);
    }
}
