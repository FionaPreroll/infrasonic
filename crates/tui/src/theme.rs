// SPDX-License-Identifier: GPL-3.0-only
//! Colour themes: built-in ones, files in `<config dir>/themes/<name>.toml`, and single colour
//! overrides from `[ui.colors]`. Colours are names (`cyan`, `lightblue`), `#rrggbb` or a
//! 256-colour index; `reset` means the terminal's own colour.

use std::collections::BTreeMap;
use std::path::Path;
use std::str::FromStr;
use std::sync::RwLock;

use ratatui::style::{Color, Modifier, Style};

/// The colours of a theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub accent: Color,
    pub dim: Color,
    pub error: Color,
    pub warn: Color,
    pub playing: Color,
    pub header: Color,
    pub selected_fg: Color,
    pub selected_bg: Color,
}

impl Theme {
    /// Works on dark and light terminals: text stays in the terminal's colours.
    pub const DEFAULT: Theme = Theme {
        accent: Color::Cyan,
        dim: Color::DarkGray,
        error: Color::Red,
        warn: Color::Yellow,
        playing: Color::Green,
        header: Color::Gray,
        selected_fg: Color::Black,
        selected_bg: Color::Cyan,
    };
    pub const LIGHT: Theme = Theme {
        accent: Color::Blue,
        dim: Color::Gray,
        error: Color::Red,
        warn: Color::Magenta,
        playing: Color::Green,
        header: Color::DarkGray,
        selected_fg: Color::White,
        selected_bg: Color::Blue,
    };
    /// No colours, only bold and reverse video.
    pub const MONO: Theme = Theme {
        accent: Color::Reset,
        dim: Color::Reset,
        error: Color::Reset,
        warn: Color::Reset,
        playing: Color::Reset,
        header: Color::Reset,
        selected_fg: Color::Reset,
        selected_bg: Color::Reset,
    };

    pub fn builtin(name: &str) -> Option<Theme> {
        match name {
            "default" | "dark" | "" => Some(Self::DEFAULT),
            "light" => Some(Self::LIGHT),
            "mono" | "none" => Some(Self::MONO),
            _ => None,
        }
    }

    pub const BUILTIN: [&'static str; 3] = ["default", "light", "mono"];

    /// Set one colour by name. Returns false for unknown names or colours.
    pub fn set(&mut self, key: &str, value: &str) -> bool {
        let Ok(c) = Color::from_str(value.trim()) else {
            return false;
        };
        let slot = match key {
            "accent" => &mut self.accent,
            "dim" => &mut self.dim,
            "error" => &mut self.error,
            "warn" => &mut self.warn,
            "playing" => &mut self.playing,
            "header" => &mut self.header,
            "selected_fg" => &mut self.selected_fg,
            "selected_bg" => &mut self.selected_bg,
            _ => return false,
        };
        *slot = c;
        true
    }

    /// Resolve `name` (built-in or `<themes_dir>/<name>.toml`) and apply `overrides`.
    /// Problems are returned as warnings; the result is always usable.
    pub fn load(
        name: &str,
        themes_dir: Option<&Path>,
        overrides: &BTreeMap<String, String>,
    ) -> (Theme, Vec<String>) {
        let mut warnings = Vec::new();
        let mut theme = match Self::builtin(name) {
            Some(t) => t,
            None => {
                let file = themes_dir.map(|d| d.join(format!("{name}.toml")));
                match file.as_deref().map(std::fs::read_to_string) {
                    Some(Ok(text)) => {
                        let mut t = Self::DEFAULT;
                        match toml::from_str::<BTreeMap<String, String>>(&text) {
                            Ok(map) => {
                                for (k, v) in &map {
                                    if !t.set(k, v) {
                                        warnings.push(format!("theme {name}: bad entry {k} = {v}"));
                                    }
                                }
                            }
                            Err(e) => warnings.push(format!("theme {name}: {e}")),
                        }
                        t
                    }
                    _ => {
                        warnings.push(format!("unknown theme `{name}`, using the default"));
                        Self::DEFAULT
                    }
                }
            }
        };
        for (k, v) in overrides {
            if !theme.set(k, v) {
                warnings.push(format!("[ui.colors] bad entry {k} = {v}"));
            }
        }
        (theme, warnings)
    }
}

static CURRENT: RwLock<Theme> = RwLock::new(Theme::DEFAULT);

/// Use `t` for everything drawn from now on.
pub fn apply(t: Theme) {
    if let Ok(mut c) = CURRENT.write() {
        *c = t;
    }
}

pub fn current() -> Theme {
    CURRENT.read().map(|t| *t).unwrap_or(Theme::DEFAULT)
}

fn fg(c: Color) -> Style {
    Style::default().fg(c)
}

pub fn accent() -> Style {
    let t = current();
    if t.accent == Color::Reset {
        Style::default().add_modifier(Modifier::BOLD)
    } else {
        fg(t.accent)
    }
}
pub fn dim() -> Style {
    let t = current();
    if t.dim == Color::Reset {
        Style::default().add_modifier(Modifier::DIM)
    } else {
        fg(t.dim)
    }
}
pub fn error() -> Style {
    fg(current().error).add_modifier(Modifier::BOLD)
}
pub fn warn() -> Style {
    fg(current().warn)
}
pub fn playing() -> Style {
    fg(current().playing).add_modifier(Modifier::BOLD)
}
pub fn header() -> Style {
    fg(current().header).add_modifier(Modifier::BOLD)
}
pub fn border(focused: bool) -> Style {
    if focused { accent() } else { dim() }
}
pub fn selected(focused: bool) -> Style {
    let t = current();
    if focused && t.selected_bg != Color::Reset {
        Style::default().bg(t.selected_bg).fg(t.selected_fg)
    } else {
        Style::default().add_modifier(Modifier::REVERSED)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_files_and_overrides() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("nord.toml"),
            "accent = \"#88c0d0\"\ndim = \"240\"\nbogus = \"red\"\n",
        )
        .expect("write");
        let mut o = BTreeMap::new();
        o.insert("warn".to_string(), "lightmagenta".to_string());
        let (t, w) = Theme::load("nord", Some(dir.path()), &o);
        assert_eq!(t.accent, Color::Rgb(0x88, 0xc0, 0xd0));
        assert_eq!(t.dim, Color::Indexed(240));
        assert_eq!(t.warn, Color::LightMagenta);
        assert_eq!(w.len(), 1, "{w:?}");
        let (t, w) = Theme::load("nothing", Some(dir.path()), &BTreeMap::new());
        assert_eq!(t, Theme::DEFAULT);
        assert_eq!(w.len(), 1);
        assert_eq!(Theme::load("light", None, &BTreeMap::new()).0, Theme::LIGHT);
    }
}
