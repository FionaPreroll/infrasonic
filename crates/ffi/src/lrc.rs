// SPDX-License-Identifier: GPL-3.0-only
//! LRC lyrics (`[01:23.45]A line`), which some servers return as plain text.

use crate::types::LyricsLine;

/// Parse `[mm:ss.xx]` time tags, several per line allowed, and `[offset:±ms]`. Returns the
/// lines sorted by time, or None if the text has no time tags.
pub(crate) fn parse(text: &str) -> Option<Vec<LyricsLine>> {
    let mut offset: i64 = 0;
    let mut lines = Vec::new();
    for raw in text.lines() {
        let mut rest = raw.trim();
        let mut times = Vec::new();
        while let Some(tag) = rest.strip_prefix('[') {
            let Some(end) = tag.find(']') else { break };
            let inner = &tag[..end];
            if let Some(ms) = time(inner) {
                times.push(ms);
            } else if let Some(v) = inner.strip_prefix("offset:") {
                offset = v.trim().parse().unwrap_or(0);
            }
            rest = tag[end + 1..].trim_start();
        }
        for t in times {
            lines.push((t, rest.to_string()));
        }
    }
    if lines.is_empty() {
        return None;
    }
    lines.sort_by_key(|(t, _)| *t);
    // A positive offset shows the lyrics earlier.
    Some(
        lines
            .into_iter()
            .map(|(t, text)| LyricsLine {
                start_ms: Some((t - offset).max(0)),
                text,
            })
            .collect(),
    )
}

/// `mm:ss`, `mm:ss.x`, `mm:ss.xx`, `mm:ss.xxx` (also with `:` before the fraction).
fn time(s: &str) -> Option<i64> {
    let (min, sec) = s.split_once(':')?;
    let min: i64 = min.trim().parse().ok()?;
    let (whole, frac) = match sec.find(['.', ':']) {
        Some(i) => (&sec[..i], &sec[i + 1..]),
        None => (sec, ""),
    };
    let whole: i64 = whole.parse().ok()?;
    if whole >= 60 || !frac.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let frac_ms = match frac.len() {
        0 => 0,
        1 => frac.parse::<i64>().ok()? * 100,
        2 => frac.parse::<i64>().ok()? * 10,
        _ => frac[..3].parse::<i64>().ok()?,
    };
    Some((min * 60 + whole) * 1000 + frac_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lrc_lines() {
        let text = "[ar:Someone]\n[ti:Song]\n[offset:+500]\n\n[00:12.30]First\n[00:05.5][01:00.123]Twice\n[00:20]\n[01:02:50]Colon";
        let l = parse(text).expect("lrc");
        let got: Vec<(i64, &str)> = l
            .iter()
            .map(|x| (x.start_ms.unwrap(), x.text.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![
                (5000, "Twice"),
                (11_800, "First"),
                (19_500, ""),
                (59_623, "Twice"),
                (62_000, "Colon"),
            ]
        );
    }

    #[test]
    fn plain_text_is_not_lrc() {
        assert!(parse("Just\nsome [words] here").is_none());
        assert!(parse("[99:99]x").is_none());
    }
}
