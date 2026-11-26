// SPDX-License-Identifier: GPL-3.0-only
//! Key bindings. One table drives dispatch, the help overlay and the command palette; the
//! `[keys]` section of the configuration file overrides it.

use std::collections::BTreeMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use infrasonic_core::config::KeyList;

use crate::i18n::tr_dyn;
use crate::t;

/// Where a binding applies. Page bindings win over list navigation, which wins over global ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Context {
    Global,
    List,
    Browser,
    Queue,
    Playlists,
    Search,
    Library,
}

impl Context {
    pub const ALL: [Context; 7] = [
        Self::Global,
        Self::List,
        Self::Browser,
        Self::Queue,
        Self::Playlists,
        Self::Search,
        Self::Library,
    ];

    /// Name in the configuration file.
    pub fn name(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::List => "list",
            Self::Browser => "browser",
            Self::Queue => "queue",
            Self::Playlists => "playlists",
            Self::Search => "search",
            Self::Library => "library",
        }
    }

    pub fn title(self) -> &'static str {
        tr_dyn(&format!("context.{}", self.name()))
    }

    fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.name() == s)
    }
}

macro_rules! actions {
    ($($variant:ident => $name:literal),+ $(,)?) => {
        /// Everything a key can do.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum Action {
            /// Switch to page `n` (0-based).
            Page(u8),
            $($variant),+
        }

        impl Action {
            const NAMED: &'static [(Action, &'static str)] = &[$((Action::$variant, $name)),+];
        }
    };
}

actions! {
    Quit => "quit",
    Help => "help",
    Palette => "palette",
    NextPage => "next_page",
    PrevPage => "prev_page",
    TogglePause => "toggle_pause",
    Stop => "stop",
    Next => "next",
    Previous => "previous",
    SeekForward => "seek_forward",
    SeekBack => "seek_back",
    VolumeUp => "volume_up",
    VolumeDown => "volume_down",
    CycleRepeat => "cycle_repeat",
    AddRandom => "add_random",
    Scan => "scan",
    StarPlaying => "star_playing",
    Bookmark => "bookmark",
    SwitchProfile => "switch_profile",
    Redraw => "redraw",

    Up => "up",
    Down => "down",
    PageUp => "page_up",
    PageDown => "page_down",
    Top => "top",
    Bottom => "bottom",
    Left => "left",
    Right => "right",
    FocusNext => "focus_next",
    Filter => "filter",
    Back => "back",

    Activate => "activate",
    Append => "append",
    PlayNext => "play_next",
    Star => "star",
    Rate => "rate",
    Pin => "pin",
    AddToPlaylist => "add_to_playlist",
    Refresh => "refresh",
    Similar => "similar",

    Remove => "remove",
    ClearQueue => "clear_queue",
    MoveUp => "move_up",
    MoveDown => "move_down",
    SaveQueue => "save_queue",
    Shuffle => "shuffle",
    LoadServerQueue => "load_server_queue",
    JumpToPlaying => "jump_to_playing",

    NewPlaylist => "new_playlist",
    DeletePlaylist => "delete_playlist",

    EditQuery => "edit_query",
}

/// Number of pages reachable with `page_N`.
pub const PAGES: u8 = 7;

impl Action {
    /// Name in the configuration file and the command palette.
    pub fn name(self) -> String {
        match self {
            Self::Page(n) => format!("page_{}", n + 1),
            a => Self::NAMED
                .iter()
                .find(|(x, _)| *x == a)
                .map(|(_, n)| (*n).to_string())
                .unwrap_or_default(),
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        if let Some(n) = s.strip_prefix("page_").and_then(|n| n.parse::<u8>().ok()) {
            return (1..=PAGES).contains(&n).then(|| Self::Page(n - 1));
        }
        Self::NAMED.iter().find(|(_, n)| *n == s).map(|(a, _)| *a)
    }

    /// Translated description, context dependent where the same action means different things.
    pub fn help(self, ctx: Context) -> String {
        if let Self::Page(n) = self {
            return t!(
                "action.page",
                page = crate::app::Page::ALL[usize::from(n)].title()
            );
        }
        let specific = format!("action.{}.{}", ctx.name(), self.name());
        let s = tr_dyn(&specific);
        if s != specific {
            return s.to_string();
        }
        tr_dyn(&format!("action.{}", self.name())).to_string()
    }

    /// List navigation; not offered in the command palette.
    pub fn is_navigation(self) -> bool {
        matches!(
            self,
            Self::Up
                | Self::Down
                | Self::PageUp
                | Self::PageDown
                | Self::Top
                | Self::Bottom
                | Self::Left
                | Self::Right
                | Self::FocusNext
                | Self::Back
        )
    }
}

/// A key with modifiers. Shift is implied by upper case characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Key {
    pub code: KeyCode,
    pub ctrl: bool,
    pub alt: bool,
}

impl Key {
    pub fn matches(&self, ev: &KeyEvent) -> bool {
        if ev.modifiers.contains(KeyModifiers::CONTROL) != self.ctrl
            || ev.modifiers.contains(KeyModifiers::ALT) != self.alt
        {
            return false;
        }
        match (self.code, ev.code) {
            (KeyCode::Char(a), KeyCode::Char(b)) => a == b,
            (a, b) => a == b,
        }
    }

    /// Parse `q`, `Q`, `Space`, `Enter`, `Ctrl-x`, `Alt-x`, `F5`, `PgDn`, …
    pub fn parse(s: &str) -> Option<Self> {
        let mut rest = s.trim();
        let (mut ctrl, mut alt) = (false, false);
        loop {
            let lower = rest.to_ascii_lowercase();
            let strip = |p: &str| (lower.starts_with(p) && rest.len() > p.len()).then_some(p.len());
            if let Some(n) = strip("ctrl-")
                .or_else(|| strip("ctrl+"))
                .or_else(|| strip("c-"))
            {
                ctrl = true;
                rest = &rest[n..];
            } else if let Some(n) = strip("alt-")
                .or_else(|| strip("alt+"))
                .or_else(|| strip("m-"))
            {
                alt = true;
                rest = &rest[n..];
            } else {
                break;
            }
        }
        let mut chars = rest.chars();
        let code = match (chars.next(), chars.next()) {
            (Some(c), None) => KeyCode::Char(c),
            _ => match rest.to_ascii_lowercase().as_str() {
                "space" => KeyCode::Char(' '),
                "enter" | "return" => KeyCode::Enter,
                "esc" | "escape" => KeyCode::Esc,
                "tab" => KeyCode::Tab,
                "backtab" | "shift-tab" => KeyCode::BackTab,
                "backspace" | "bs" => KeyCode::Backspace,
                "del" | "delete" => KeyCode::Delete,
                "ins" | "insert" => KeyCode::Insert,
                "up" => KeyCode::Up,
                "down" => KeyCode::Down,
                "left" => KeyCode::Left,
                "right" => KeyCode::Right,
                "pgup" | "pageup" => KeyCode::PageUp,
                "pgdn" | "pagedown" => KeyCode::PageDown,
                "home" => KeyCode::Home,
                "end" => KeyCode::End,
                f if f.starts_with('f') => KeyCode::F(f[1..].parse().ok().filter(|n| *n <= 24)?),
                _ => return None,
            },
        };
        Some(Self { code, ctrl, alt })
    }

    pub fn label(&self) -> String {
        let base = match self.code {
            KeyCode::Char(' ') => "Space".to_string(),
            KeyCode::Char(ch) => ch.to_string(),
            KeyCode::Enter => "Enter".into(),
            KeyCode::Esc => "Esc".into(),
            KeyCode::Tab => "Tab".into(),
            KeyCode::BackTab => "Shift-Tab".into(),
            KeyCode::Backspace => "Backspace".into(),
            KeyCode::Delete => "Del".into(),
            KeyCode::Insert => "Ins".into(),
            KeyCode::Up => "↑".into(),
            KeyCode::Down => "↓".into(),
            KeyCode::Left => "←".into(),
            KeyCode::Right => "→".into(),
            KeyCode::PageUp => "PgUp".into(),
            KeyCode::PageDown => "PgDn".into(),
            KeyCode::Home => "Home".into(),
            KeyCode::End => "End".into(),
            KeyCode::F(n) => format!("F{n}"),
            other => format!("{other:?}"),
        };
        let mut s = String::new();
        if self.ctrl {
            s.push_str("Ctrl-");
        }
        if self.alt {
            s.push_str("Alt-");
        }
        s + &base
    }
}

use Action as A;
use Context as C;

/// The default key map, modelled on STMPS.
#[rustfmt::skip]
const DEFAULTS: &[(Context, Action, &str)] = &[
    (C::Global, A::Quit, "q Q"),
    (C::Global, A::Help, "? F1"),
    (C::Global, A::Palette, ":"),
    (C::Global, A::Page(0), "1"),
    (C::Global, A::Page(1), "2"),
    (C::Global, A::Page(2), "3"),
    (C::Global, A::Page(3), "4"),
    (C::Global, A::Page(4), "5"),
    (C::Global, A::Page(5), "6"),
    (C::Global, A::Page(6), "7"),
    (C::Global, A::NextPage, "Ctrl-n"),
    (C::Global, A::PrevPage, "Ctrl-p"),
    (C::Global, A::TogglePause, "p Space"),
    (C::Global, A::Stop, "P"),
    (C::Global, A::Next, ">"),
    (C::Global, A::Previous, "<"),
    (C::Global, A::SeekForward, "."),
    (C::Global, A::SeekBack, ","),
    (C::Global, A::VolumeUp, "= +"),
    (C::Global, A::VolumeDown, "-"),
    (C::Global, A::CycleRepeat, "m"),
    (C::Global, A::AddRandom, "r"),
    (C::Global, A::Scan, "U"),
    (C::Global, A::StarPlaying, "Y"),
    (C::Global, A::Bookmark, "b"),
    (C::Global, A::SwitchProfile, "F2"),
    (C::Global, A::Redraw, "Ctrl-l"),
    (C::List, A::Up, "k Up"),
    (C::List, A::Down, "j Down"),
    (C::List, A::PageUp, "PgUp Ctrl-u"),
    (C::List, A::PageDown, "PgDn Ctrl-d"),
    (C::List, A::Top, "g Home"),
    (C::List, A::Bottom, "G End"),
    (C::List, A::Left, "h Left"),
    (C::List, A::Right, "l Right"),
    (C::List, A::FocusNext, "Tab"),
    (C::List, A::Filter, "/"),
    (C::List, A::Back, "Backspace Esc"),
    (C::Browser, A::Activate, "Enter"),
    (C::Browser, A::Append, "a"),
    (C::Browser, A::PlayNext, "i"),
    (C::Browser, A::Star, "y"),
    (C::Browser, A::Rate, "*"),
    (C::Browser, A::Pin, "O"),
    (C::Browser, A::AddToPlaylist, "A"),
    (C::Browser, A::Refresh, "R"),
    (C::Browser, A::Similar, "S"),
    (C::Queue, A::Activate, "Enter"),
    (C::Queue, A::Remove, "d Del"),
    (C::Queue, A::ClearQueue, "D"),
    (C::Queue, A::MoveUp, "K"),
    (C::Queue, A::MoveDown, "J"),
    (C::Queue, A::Star, "y"),
    (C::Queue, A::Rate, "*"),
    (C::Queue, A::Pin, "O"),
    (C::Queue, A::AddToPlaylist, "A"),
    (C::Queue, A::SaveQueue, "s"),
    (C::Queue, A::Shuffle, "S"),
    (C::Queue, A::LoadServerQueue, "L"),
    (C::Queue, A::JumpToPlaying, "o"),
    (C::Playlists, A::Activate, "Enter"),
    (C::Playlists, A::Append, "a"),
    (C::Playlists, A::PlayNext, "i"),
    (C::Playlists, A::NewPlaylist, "n"),
    (C::Playlists, A::DeletePlaylist, "d Del"),
    (C::Playlists, A::Star, "y"),
    (C::Playlists, A::Rate, "*"),
    (C::Playlists, A::Pin, "O"),
    (C::Playlists, A::Refresh, "R"),
    (C::Search, A::Activate, "Enter"),
    (C::Search, A::Append, "a"),
    (C::Search, A::PlayNext, "i"),
    (C::Search, A::Star, "y"),
    (C::Search, A::Rate, "*"),
    (C::Search, A::Pin, "O"),
    (C::Search, A::AddToPlaylist, "A"),
    (C::Search, A::EditQuery, "e s"),
    (C::Library, A::Activate, "Enter"),
    (C::Library, A::Append, "a"),
    (C::Library, A::PlayNext, "i"),
    (C::Library, A::Star, "y"),
    (C::Library, A::Rate, "*"),
    (C::Library, A::Pin, "O"),
    (C::Library, A::AddToPlaylist, "A"),
    (C::Library, A::Remove, "d Del"),
    (C::Library, A::Refresh, "R"),
];

/// One binding at run time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub context: Context,
    pub action: Action,
    pub keys: Vec<Key>,
}

/// The active key map.
#[derive(Debug, Clone)]
pub struct Keymap {
    bindings: Vec<Binding>,
    /// Problems with the `[keys]` section, for the log.
    pub warnings: Vec<String>,
}

impl Default for Keymap {
    fn default() -> Self {
        let bindings = DEFAULTS
            .iter()
            .map(|(context, action, keys)| Binding {
                context: *context,
                action: *action,
                keys: keys.split_whitespace().filter_map(Key::parse).collect(),
            })
            .collect();
        Self {
            bindings,
            warnings: Vec::new(),
        }
    }
}

impl Keymap {
    /// The defaults with `[keys]` overrides applied. Entries are `"context.action"` or just
    /// `"action"` (every context that has it). An empty list unbinds.
    pub fn with_overrides(overrides: &BTreeMap<String, KeyList>) -> Self {
        let mut map = Self::default();
        let mut touched: Vec<(Context, Action)> = Vec::new();
        for (name, keys) in overrides {
            let (ctx, action) = match name.split_once('.') {
                Some((c, a)) => match Context::parse(c) {
                    Some(c) => (Some(c), a),
                    None => {
                        map.warnings
                            .push(format!("[keys] {name}: unknown context `{c}`"));
                        continue;
                    }
                },
                None => (None, name.as_str()),
            };
            let Some(action) = Action::parse(action) else {
                map.warnings
                    .push(format!("[keys] {name}: unknown action `{action}`"));
                continue;
            };
            let mut parsed = Vec::new();
            for k in keys.keys() {
                match Key::parse(&k) {
                    Some(key) => parsed.push(key),
                    None => map
                        .warnings
                        .push(format!("[keys] {name}: unknown key `{k}`")),
                }
            }
            let contexts: Vec<Context> = match ctx {
                Some(c) => vec![c],
                None => {
                    let v: Vec<Context> = map
                        .bindings
                        .iter()
                        .filter(|b| b.action == action)
                        .map(|b| b.context)
                        .collect();
                    if v.is_empty() {
                        vec![Context::Global]
                    } else {
                        v
                    }
                }
            };
            for c in contexts {
                touched.push((c, action));
                match map
                    .bindings
                    .iter_mut()
                    .find(|b| b.context == c && b.action == action)
                {
                    Some(b) => b.keys = parsed.clone(),
                    None => map.bindings.push(Binding {
                        context: c,
                        action,
                        keys: parsed.clone(),
                    }),
                }
            }
        }
        map.check_conflicts(&touched);
        map
    }

    /// Report keys that an override made ambiguous.
    fn check_conflicts(&mut self, touched: &[(Context, Action)]) {
        let mut warnings = Vec::new();
        for b in self
            .bindings
            .iter()
            .filter(|b| touched.contains(&(b.context, b.action)))
        {
            for other in &self.bindings {
                if other.action == b.action && other.context == b.context {
                    continue;
                }
                // Same context: one of them never fires. Page vs. list/global: the page wins.
                let related = other.context == b.context
                    || matches!(other.context, Context::Global | Context::List)
                    || matches!(b.context, Context::Global | Context::List);
                if !related {
                    continue;
                }
                for k in b.keys.iter().filter(|k| other.keys.contains(k)) {
                    warnings.push(format!(
                        "[keys] {} is bound to {}.{} and {}.{}",
                        k.label(),
                        b.context.name(),
                        b.action.name(),
                        other.context.name(),
                        other.action.name()
                    ));
                }
            }
        }
        warnings.sort();
        warnings.dedup();
        self.warnings.extend(warnings);
    }

    /// The action for a key in the given contexts, in order.
    pub fn lookup(&self, ev: &KeyEvent, contexts: &[Context]) -> Option<Action> {
        contexts.iter().find_map(|ctx| {
            self.bindings
                .iter()
                .filter(|b| b.context == *ctx)
                .find(|b| b.keys.iter().any(|k| k.matches(ev)))
                .map(|b| b.action)
        })
    }

    pub fn bindings(&self) -> &[Binding] {
        &self.bindings
    }

    /// Key labels for an action, e.g. `"d Del"`.
    pub fn keys_label(&self, ctx: Context, action: Action) -> String {
        self.bindings
            .iter()
            .filter(|b| b.context == ctx && b.action == action)
            .flat_map(|b| b.keys.iter().map(Key::label))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Help lines grouped by context: (context, keys, description).
    pub fn help_rows(&self) -> Vec<(Context, String, String)> {
        let mut rows: Vec<(Context, String, String)> = self
            .bindings
            .iter()
            .filter(|b| !b.keys.is_empty())
            .map(|b| {
                let keys: Vec<String> = b.keys.iter().map(Key::label).collect();
                (b.context, keys.join(" "), b.action.help(b.context))
            })
            .collect();
        rows.sort_by_key(|r| r.0);
        rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn page_context_wins() {
        let m = Keymap::default();
        let ev = key(KeyCode::Char('s'));
        assert_eq!(
            m.lookup(&ev, &[C::Queue, C::List, C::Global]),
            Some(A::SaveQueue)
        );
        assert_eq!(m.lookup(&ev, &[C::Browser, C::List, C::Global]), None);
    }

    #[test]
    fn shift_letters_and_ctrl() {
        let m = Keymap::default();
        let ev = KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT);
        assert_eq!(m.lookup(&ev, &[C::List]), Some(A::Bottom));
        let ev = KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL);
        assert_eq!(m.lookup(&ev, &[C::Queue, C::List]), Some(A::PageDown));
        let ev = key(KeyCode::F(2));
        assert_eq!(m.lookup(&ev, &[C::Global]), Some(A::SwitchProfile));
    }

    #[test]
    fn defaults_parse_and_do_not_clash() {
        for (ctx, action, keys) in DEFAULTS {
            for k in keys.split_whitespace() {
                assert!(Key::parse(k).is_some(), "{k} for {action:?}");
            }
            assert_eq!(Action::parse(&action.name()), Some(*action), "{ctx:?}");
        }
        let m = Keymap::default();
        for (i, a) in m.bindings.iter().enumerate() {
            for b in &m.bindings[i + 1..] {
                if a.context != b.context {
                    continue;
                }
                for ka in &a.keys {
                    assert!(
                        !b.keys.contains(ka),
                        "{ka:?} bound twice in {:?}",
                        a.context
                    );
                }
            }
        }
    }

    #[test]
    fn parses_keys() {
        let p = |s| Key::parse(s).expect(s);
        assert_eq!(p("Ctrl-x").code, KeyCode::Char('x'));
        assert!(p("C-x").ctrl && p("ctrl+x").ctrl && p("Alt-x").alt);
        assert_eq!(p("Space").code, KeyCode::Char(' '));
        assert_eq!(p("F12").code, KeyCode::F(12));
        assert_eq!(p("-").code, KeyCode::Char('-'));
        assert_eq!(p("PgDn").code, KeyCode::PageDown);
        assert_eq!(Key::parse("F99"), None);
        assert_eq!(Key::parse("bogus"), None);
        assert_eq!(p("Ctrl-x").label(), "Ctrl-x");
    }

    #[test]
    fn overrides_replace_and_warn() {
        let mut o = BTreeMap::new();
        o.insert("queue.remove".to_string(), KeyList::One("x".into()));
        o.insert("star".to_string(), KeyList::Many(vec!["*".into()]));
        o.insert("quit".to_string(), KeyList::Many(vec![]));
        o.insert("nonsense".to_string(), KeyList::One("z".into()));
        o.insert("global.stop".to_string(), KeyList::One("Hyper-q".into()));
        let m = Keymap::with_overrides(&o);
        let ctx = [C::Queue, C::List, C::Global];
        assert_eq!(m.lookup(&key(KeyCode::Char('x')), &ctx), Some(A::Remove));
        assert_eq!(m.lookup(&key(KeyCode::Char('d')), &ctx), None);
        assert_eq!(
            m.lookup(&key(KeyCode::Char('q')), &ctx),
            None,
            "quit unbound"
        );
        // `star` without a context applies to every page that has it; `*` was `rate`.
        assert_eq!(
            m.lookup(&key(KeyCode::Char('*')), &[C::Search]),
            Some(A::Star)
        );
        let w = m.warnings.join("\n");
        assert!(w.contains("unknown action `nonsense`"), "{w}");
        assert!(w.contains("unknown key `Hyper-q`"), "{w}");
        assert!(w.contains("* is bound to"), "{w}");
        assert_eq!(Keymap::default().warnings, Vec::<String>::new());
    }
}
