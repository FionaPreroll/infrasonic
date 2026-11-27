// SPDX-License-Identifier: GPL-3.0-only
//! Texts of the web player in `locales/<lang>.json`. English is complete and the fallback.
//! The language is chosen once at start; changing it reloads the page.

use std::collections::HashMap;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, Ordering};

static EN: LazyLock<HashMap<String, String>> =
    LazyLock::new(|| serde_json::from_str(include_str!("../locales/en.json")).unwrap_or_default());
static DE: LazyLock<HashMap<String, String>> =
    LazyLock::new(|| serde_json::from_str(include_str!("../locales/de.json")).unwrap_or_default());
static GERMAN: AtomicBool = AtomicBool::new(false);

/// `en`, `de`, or `auto` with the browser languages (`navigator.languages`).
pub fn set_language(setting: &str, browser: &[String]) {
    let de = match setting {
        "de" => true,
        "en" => false,
        _ => browser
            .iter()
            .find(|l| l.starts_with("de") || l.starts_with("en"))
            .is_some_and(|l| l.starts_with("de")),
    };
    GERMAN.store(de, Ordering::Relaxed);
}

pub fn is_german() -> bool {
    GERMAN.load(Ordering::Relaxed)
}

/// The text for `key`; the key itself if it is missing everywhere.
pub fn t(key: &str) -> String {
    let table = if is_german() { &*DE } else { &*EN };
    table
        .get(key)
        .or_else(|| EN.get(key))
        .cloned()
        .unwrap_or_else(|| key.to_string())
}

/// [`t`] with `{name}` placeholders filled in.
pub fn tf(key: &str, args: &[(&str, &dyn std::fmt::Display)]) -> String {
    let mut s = t(key);
    for (k, v) in args {
        s = s.replace(&format!("{{{k}}}"), &v.to_string());
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn languages_have_the_same_keys() {
        let mut en: Vec<_> = EN.keys().collect();
        let mut de: Vec<_> = DE.keys().collect();
        en.sort();
        de.sort();
        assert!(en.len() > 50, "en.json did not parse");
        assert_eq!(en, de);
    }

    const NAMESPACES: &[&str] = &[
        "action",
        "album",
        "albums",
        "artist",
        "common",
        "error",
        "favorites",
        "help",
        "login",
        "lyrics",
        "nav",
        "playlist",
        "queue",
        "radio",
        "repeat",
        "search",
        "settings",
        "shares",
        "status",
    ];

    /// Every literal key used in the source exists.
    #[test]
    fn used_keys_exist() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut missing = Vec::new();
        let mut stack = vec![dir];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).expect("src").flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                    continue;
                }
                let text = std::fs::read_to_string(&p).unwrap_or_default();
                // Keys passed around as plain strings, like `("newest", "albums.newest")`.
                for part in text.split('"').skip(1).step_by(2) {
                    let ns = part.split('.').next().unwrap_or("");
                    let simple = part
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c == '.' || c == '_');
                    if simple
                        && part.contains('.')
                        && NAMESPACES.contains(&ns)
                        && !EN.contains_key(part)
                    {
                        missing.push(format!("{}: {part}", p.display()));
                    }
                }
                for pat in ["t(\"", "tf(\""] {
                    for part in text.split(pat).skip(1) {
                        let key = part.split('"').next().unwrap_or("");
                        if key.contains('.') && !key.contains('{') && !EN.contains_key(key) {
                            missing.push(format!("{}: {key}", p.display()));
                        }
                    }
                }
            }
        }
        assert!(
            missing.is_empty(),
            "missing in en.json:\n{}",
            missing.join("\n")
        );
    }

    #[test]
    fn auto_follows_the_browser() {
        set_language("auto", &["fr".into(), "de-AT".into()]);
        assert!(is_german());
        set_language("auto", &["en-GB".into(), "de".into()]);
        assert!(!is_german());
        set_language("de", &[]);
        assert_eq!(tf("x.{n}", &[("n", &1)]), "x.1");
        set_language("en", &[]);
    }
}
