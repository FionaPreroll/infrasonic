// SPDX-License-Identifier: GPL-3.0-only
//! Translations. The texts live in `locales/<lang>.toml`, grouped in tables; a key is
//! `table.name`. English is complete and the fallback for anything missing.
//!
//! ```ignore
//! t!("status.playing_n", n = 3)   // "Playing {n} songs" with {n} replaced
//! ```

use std::collections::HashMap;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU8, Ordering};

/// Supported languages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    De,
}

impl Lang {
    pub const ALL: [Lang; 2] = [Lang::En, Lang::De];

    pub fn code(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::De => "de",
        }
    }

    /// `en`, `de`, or `auto` (from `LC_ALL`, `LC_MESSAGES` or `LANG`).
    pub fn from_setting(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "de" | "german" | "deutsch" => Self::De,
            "en" | "english" => Self::En,
            _ => Self::from_env(),
        }
    }

    fn from_env() -> Self {
        let var = ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .filter_map(|v| std::env::var(v).ok())
            .find(|v| !v.is_empty())
            .unwrap_or_default();
        if var.to_ascii_lowercase().starts_with("de") {
            Self::De
        } else {
            Self::En
        }
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

/// Switch the language for all texts from now on.
pub fn set_language(lang: Lang) {
    CURRENT.store(lang as u8, Ordering::Relaxed);
}

pub fn language() -> Lang {
    if CURRENT.load(Ordering::Relaxed) == Lang::De as u8 {
        Lang::De
    } else {
        Lang::En
    }
}

type Table = HashMap<String, &'static str>;

fn parse(src: &str) -> Table {
    let value: toml::Table = toml::from_str(src).unwrap_or_default();
    let mut out = HashMap::new();
    fn walk(prefix: &str, t: &toml::Table, out: &mut Table) {
        for (k, v) in t {
            let key = if prefix.is_empty() {
                k.clone()
            } else {
                format!("{prefix}.{k}")
            };
            match v {
                toml::Value::String(s) => {
                    out.insert(key, &*Box::leak(s.clone().into_boxed_str()));
                }
                toml::Value::Table(t) => walk(&key, t, out),
                _ => {}
            }
        }
    }
    walk("", &value, &mut out);
    out
}

static EN: LazyLock<Table> = LazyLock::new(|| parse(include_str!("../locales/en.toml")));
static DE: LazyLock<Table> = LazyLock::new(|| parse(include_str!("../locales/de.toml")));

fn table(lang: Lang) -> &'static Table {
    match lang {
        Lang::En => &EN,
        Lang::De => &DE,
    }
}

/// The text for `key` in the current language. Unknown keys come back as the key itself.
pub fn tr_dyn(key: &str) -> &'static str {
    let lang = language();
    if let Some(s) = table(lang).get(key) {
        return s;
    }
    if let Some(s) = EN.get(key) {
        return s;
    }
    // Leaking the key keeps the signature simple; it only happens for missing texts.
    Box::leak(key.to_string().into_boxed_str())
}

/// Replace `{name}` placeholders.
pub fn fill(text: &str, args: &[(&str, &dyn std::fmt::Display)]) -> String {
    let mut out = text.to_string();
    for (name, value) in args {
        out = out.replace(&format!("{{{name}}}"), &value.to_string());
    }
    out
}

/// `t!("key")` or `t!("key", name = value, …)`. A key `x` with a numeric argument `n` uses
/// `x_one` when `n == 1` and that text exists.
#[macro_export]
macro_rules! t {
    ($key:literal) => {
        $crate::i18n::tr_dyn($key)
    };
    ($key:literal, n = $n:expr $(, $name:ident = $v:expr)* $(,)?) => {{
        let n = $n;
        let one = concat!($key, "_one");
        let text = if n == 1 && $crate::i18n::tr_dyn(one) != one {
            $crate::i18n::tr_dyn(one)
        } else {
            $crate::i18n::tr_dyn($key)
        };
        $crate::i18n::fill(text, &[("n", &n as &dyn std::fmt::Display) $(, (stringify!($name), &$v as &dyn std::fmt::Display))*])
    }};
    ($key:literal, $($name:ident = $v:expr),+ $(,)?) => {
        $crate::i18n::fill($crate::i18n::tr_dyn($key), &[$((stringify!($name), &$v as &dyn std::fmt::Display)),+])
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(t: &Table) -> Vec<&String> {
        let mut v: Vec<&String> = t.keys().collect();
        v.sort();
        v
    }

    #[test]
    fn languages_have_the_same_keys() {
        let en = keys(&EN);
        let de = keys(&DE);
        let missing: Vec<_> = en.iter().filter(|k| !DE.contains_key(**k)).collect();
        let extra: Vec<_> = de.iter().filter(|k| !EN.contains_key(**k)).collect();
        assert!(missing.is_empty(), "missing in de.toml: {missing:?}");
        assert!(extra.is_empty(), "only in de.toml: {extra:?}");
        assert!(en.len() > 100);
    }

    #[test]
    fn placeholders_match() {
        let re = |s: &str| {
            let mut v: Vec<String> = s
                .split('{')
                .skip(1)
                .filter_map(|p| p.split_once('}').map(|(a, _)| a.to_string()))
                .collect();
            v.sort();
            v
        };
        for (k, en) in EN.iter() {
            if let Some(de) = DE.get(k) {
                assert_eq!(re(en), re(de), "placeholders of {k}");
            }
        }
    }

    /// Every literal key used in the source exists in English.
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
                for part in text.split("t!(\"").skip(1) {
                    let key = part.split('"').next().unwrap_or("");
                    if key.contains('.') && !key.contains('{') && !EN.contains_key(key) {
                        missing.push(format!("{}: {key}", p.display()));
                    }
                }
            }
        }
        assert!(
            missing.is_empty(),
            "keys missing in en.toml:\n{}",
            missing.join("\n")
        );
    }

    /// Actions, contexts and palette commands are looked up by name at runtime.
    #[test]
    fn dynamic_keys_exist() {
        let mut missing = Vec::new();
        for b in crate::keys::Keymap::default().bindings() {
            let a = b.action.name();
            if !matches!(b.action, crate::keys::Action::Page(_))
                && !EN.contains_key(&format!("action.{a}"))
            {
                missing.push(format!("action.{a}"));
            }
            let c = format!("context.{}", b.context.name());
            if !EN.contains_key(&c) {
                missing.push(c);
            }
        }
        for c in crate::palette::COMMANDS {
            if !EN.contains_key(&format!("command.{c}")) {
                missing.push(format!("command.{c}"));
            }
        }
        missing.dedup();
        assert!(missing.is_empty(), "{missing:?}");
    }

    #[test]
    fn fills_placeholders_and_plurals() {
        set_language(Lang::En);
        assert_eq!(fill("{a} and {b}", &[("a", &1), ("b", &"x")]), "1 and x");
        assert_eq!(tr_dyn("no.such.key"), "no.such.key");
        assert_eq!(Lang::from_setting("de"), Lang::De);
        assert_eq!(Lang::from_setting("EN"), Lang::En);
        assert_eq!(t!("status.added_n", n = 1), "Added 1 song to the queue");
        assert_eq!(t!("status.added_n", n = 3), "Added 3 songs to the queue");
    }
}
