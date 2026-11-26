// SPDX-License-Identifier: GPL-3.0-only
//! The command palette (`:`): every action of the current page, plus commands with arguments
//! such as `volume 40` or `replaygain album`.

use crossterm::event::{KeyCode, KeyEvent};
use infrasonic_core::config::ReplayGainSetting;
use infrasonic_core::engine::Command;

use crate::app::{App, ConfirmAction, Overlay, Page, PromptAction};
use crate::keys::{Action, Context};
use crate::list::{Filterable, ListView};
use crate::t;
use crate::theme::Theme;

/// Commands that take an argument.
pub const COMMANDS: &[&str] = &[
    "volume",
    "seek",
    "replaygain",
    "bitrate",
    "theme",
    "language",
    "profile",
    "years",
    "cache",
];

/// A row of the palette.
#[derive(Debug, Clone, PartialEq)]
pub struct PaletteItem {
    pub label: String,
    pub keys: String,
    pub kind: ItemKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ItemKind {
    Action(Action),
    /// A command; choosing it puts `name ` into the input.
    Command(&'static str),
}

impl Filterable for PaletteItem {
    fn filter_text(&self) -> String {
        match &self.kind {
            ItemKind::Action(a) => format!("{} {}", self.label, a.name()),
            ItemKind::Command(c) => format!("{c} {}", self.label),
        }
    }
}

/// Parse `1:30`, `90` or `1:02:03` as seconds.
pub fn parse_time(s: &str) -> Option<f64> {
    let mut total = 0.0;
    for part in s.trim().split(':') {
        let v: f64 = part.trim().parse().ok()?;
        if !v.is_finite() || v < 0.0 {
            return None;
        }
        total = total * 60.0 + v;
    }
    Some(total)
}

impl App {
    pub fn open_palette(&mut self) {
        let contexts = self.contexts();
        let mut items = Vec::new();
        for ctx in contexts.iter().copied() {
            for b in self.keys.bindings().iter().filter(|b| b.context == ctx) {
                if b.action.is_navigation() || b.action == Action::Palette {
                    continue;
                }
                if items
                    .iter()
                    .any(|i: &PaletteItem| i.kind == ItemKind::Action(b.action))
                {
                    continue;
                }
                items.push(PaletteItem {
                    label: b.action.help(ctx),
                    keys: self.keys.keys_label(ctx, b.action),
                    kind: ItemKind::Action(b.action),
                });
            }
        }
        for c in COMMANDS {
            items.push(PaletteItem {
                label: crate::i18n::tr_dyn(&format!("command.{c}")).to_string(),
                keys: format!(":{c}"),
                kind: ItemKind::Command(c),
            });
        }
        self.overlay = Some(Overlay::Palette {
            input: String::new(),
            list: ListView::new(items),
        });
    }

    pub fn palette_key(&mut self, k: KeyEvent) {
        let Some(Overlay::Palette { input, list }) = self.overlay.as_mut() else {
            return;
        };
        match k.code {
            KeyCode::Esc => self.overlay = None,
            KeyCode::Up => list.move_by(-1),
            KeyCode::Down => list.move_by(1),
            KeyCode::Tab => {
                if let Some(ItemKind::Command(c)) = list.selected().map(|i| i.kind.clone()) {
                    *input = format!("{c} ");
                    list.set_filter(String::new());
                }
            }
            KeyCode::Backspace => {
                input.pop();
                let f = filter_part(input);
                list.set_filter(f);
            }
            KeyCode::Char(c) => {
                input.push(c);
                let f = filter_part(input);
                list.set_filter(f);
            }
            KeyCode::Enter => {
                let input = input.trim().to_string();
                let selected = list.selected().map(|i| i.kind.clone());
                self.overlay = None;
                if let Some((cmd, arg)) = split_command(&input) {
                    self.run_command(cmd, arg);
                    return;
                }
                match selected {
                    Some(ItemKind::Action(a)) => self.action(a),
                    Some(ItemKind::Command(c)) => {
                        // Ask for the argument.
                        self.open_palette();
                        if let Some(Overlay::Palette { input, list }) = self.overlay.as_mut() {
                            *input = format!("{c} ");
                            list.set_filter(String::new());
                        }
                    }
                    None if !input.is_empty() => {
                        self.error(t!("error.unknown_command", cmd = input))
                    }
                    None => {}
                }
            }
            _ => {}
        }
    }

    /// Run `cmd arg` from the palette.
    pub fn run_command(&mut self, cmd: &str, arg: &str) {
        match cmd {
            "volume" | "vol" => match arg.parse::<u8>() {
                Ok(v) if v <= 100 => self.send(Command::SetVolume(v)),
                _ => self.error(t!("error.argument", cmd = cmd, arg = arg)),
            },
            "seek" => match parse_time(arg) {
                Some(t) => self.send(Command::Seek(t)),
                None => self.error(t!("error.argument", cmd = cmd, arg = arg)),
            },
            "replaygain" | "rg" => {
                let s = match arg {
                    "off" => ReplayGainSetting::Off,
                    "track" => ReplayGainSetting::Track,
                    "album" => ReplayGainSetting::Album,
                    "auto" => ReplayGainSetting::Auto,
                    _ => {
                        self.error(t!("error.argument", cmd = cmd, arg = arg));
                        return;
                    }
                };
                self.session.set_replay_gain(s);
                self.playback.replay_gain = s;
                self.info(t!("status.replaygain", mode = arg));
            }
            "bitrate" => {
                let mut parts = arg.split_whitespace();
                let kbps = parts.next().and_then(|b| b.parse::<u32>().ok());
                let format = parts.next().map(str::to_string);
                match kbps {
                    Some(k) => {
                        self.session.set_max_bit_rate(k, format);
                        self.playback.max_bit_rate = k;
                        if k == 0 {
                            self.info(t!("status.bitrate_original"));
                        } else {
                            self.info(t!("status.bitrate", n = k));
                        }
                    }
                    None => self.error(t!("error.argument", cmd = cmd, arg = arg)),
                }
            }
            "theme" => {
                let (theme, warnings) =
                    Theme::load(arg, self.themes_dir.as_deref(), &self.ui.colors);
                crate::theme::apply(theme);
                self.ui.theme = arg.to_string();
                match warnings.first() {
                    Some(w) => self.error(w.clone()),
                    None => self.info(t!("status.theme", name = arg)),
                }
            }
            "language" | "lang" => {
                crate::i18n::set_language(crate::i18n::Lang::from_setting(arg));
                self.info(t!("status.language"));
            }
            "profile" => {
                if arg.is_empty() {
                    self.action(Action::SwitchProfile);
                } else {
                    self.switch_profile(arg);
                }
            }
            "years" => {
                if arg.is_empty() {
                    self.overlay = Some(Overlay::Prompt {
                        title: t!("prompt.years").into(),
                        input: String::new(),
                        action: PromptAction::Years,
                    });
                } else {
                    match crate::pages::library::parse_years(arg) {
                        Some((from, to)) => {
                            self.goto(Page::Library);
                            self.library_open_years(from, to);
                        }
                        None => self.error(t!("error.years", input = arg)),
                    }
                }
            }
            "cache" => match (arg, self.session.cache()) {
                (_, None) => self.error(t!("error.no_store")),
                ("clear", Some(_)) => {
                    self.overlay = Some(Overlay::Confirm {
                        text: t!("confirm.clear_cache").into(),
                        action: ConfirmAction::ClearCache,
                    });
                }
                (_, Some(c)) => {
                    let u = c.usage();
                    self.info(t!(
                        "status.cache_usage",
                        n = u.files,
                        mb = u.bytes / (1 << 20),
                        pinned = c.pinned().len()
                    ));
                }
            },
            other => {
                // An action by name, e.g. `:shuffle`.
                match Action::parse(other) {
                    Some(a) => self.action(a),
                    None => self.error(t!("error.unknown_command", cmd = other)),
                }
            }
        }
    }
}

/// The part of the input that filters the list (everything, unless it is a command).
fn filter_part(input: &str) -> String {
    match split_command(input) {
        Some(_) if input.contains(' ') => String::new(),
        _ => input.trim().to_string(),
    }
}

/// `volume 40` → (`volume`, `40`). Only for known commands, or actions by name.
fn split_command(input: &str) -> Option<(&str, &str)> {
    let (cmd, arg) = match input.split_once(' ') {
        Some((c, a)) => (c, a.trim()),
        None => (input, ""),
    };
    let known = COMMANDS.contains(&cmd)
        || matches!(cmd, "vol" | "rg" | "lang")
        || (arg.is_empty() && Action::parse(cmd).is_some());
    (known && (!arg.is_empty() || input.contains(' ') || Action::parse(cmd).is_some()))
        .then_some((cmd, arg))
}

/// Contexts shown in the palette title.
pub fn title(ctx: &[Context]) -> String {
    ctx.iter()
        .map(|c| c.title())
        .collect::<Vec<_>>()
        .join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times() {
        assert_eq!(parse_time("90"), Some(90.0));
        assert_eq!(parse_time("1:30"), Some(90.0));
        assert_eq!(parse_time("1:02:03"), Some(3723.0));
        assert_eq!(parse_time("x"), None);
        assert_eq!(parse_time("-3"), None);
    }

    #[test]
    fn commands() {
        assert_eq!(split_command("volume 40"), Some(("volume", "40")));
        assert_eq!(split_command("shuffle"), Some(("shuffle", "")));
        assert_eq!(split_command("shuf"), None);
        assert_eq!(split_command("volume"), None, "needs an argument");
        assert_eq!(split_command("cache "), Some(("cache", "")));
    }
}
