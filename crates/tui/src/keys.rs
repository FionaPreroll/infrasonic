// SPDX-License-Identifier: GPL-3.0-only
//! Key bindings. One table drives both dispatch and the help overlay.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Where a binding applies. Page bindings win over list navigation, which wins over global ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Context {
    Global,
    List,
    Browser,
    Queue,
    Playlists,
    Search,
}

impl Context {
    pub fn title(self) -> &'static str {
        match self {
            Self::Global => "Global",
            Self::List => "Lists",
            Self::Browser => "Browser",
            Self::Queue => "Queue",
            Self::Playlists => "Playlists",
            Self::Search => "Search",
        }
    }
}

/// Everything a key can do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Quit,
    Help,
    Page(usize),
    NextPage,
    PrevPage,
    TogglePause,
    Stop,
    Next,
    Previous,
    SeekForward,
    SeekBack,
    VolumeUp,
    VolumeDown,
    CycleRepeat,
    AddRandom,
    Scan,
    StarPlaying,
    Redraw,

    Up,
    Down,
    PageUp,
    PageDown,
    Top,
    Bottom,
    Left,
    Right,
    FocusNext,
    Filter,
    Back,

    /// Play the selection, replacing the queue (or open a directory).
    Activate,
    Append,
    PlayNext,
    Star,
    AddToPlaylist,
    Refresh,
    Similar,

    Remove,
    ClearQueue,
    MoveUp,
    MoveDown,
    SaveQueue,
    Shuffle,
    LoadServerQueue,
    JumpToPlaying,

    NewPlaylist,
    DeletePlaylist,

    EditQuery,
}

/// One row of the key table.
#[derive(Debug)]
pub struct Binding {
    pub context: Context,
    pub keys: &'static [Key],
    pub action: Action,
    pub help: &'static str,
}

/// A key with modifiers. Shift is implied by upper case characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Key {
    pub code: KeyCode,
    pub ctrl: bool,
}

const fn c(ch: char) -> Key {
    Key {
        code: KeyCode::Char(ch),
        ctrl: false,
    }
}
const fn ctrl(ch: char) -> Key {
    Key {
        code: KeyCode::Char(ch),
        ctrl: true,
    }
}
const fn k(code: KeyCode) -> Key {
    Key { code, ctrl: false }
}

impl Key {
    pub fn matches(&self, ev: &KeyEvent) -> bool {
        let ev_ctrl = ev.modifiers.contains(KeyModifiers::CONTROL);
        if ev_ctrl != self.ctrl {
            return false;
        }
        match (self.code, ev.code) {
            (KeyCode::Char(a), KeyCode::Char(b)) => a == b,
            (a, b) => a == b,
        }
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
        if self.ctrl {
            format!("Ctrl-{base}")
        } else {
            base
        }
    }
}

use Action as A;
use Context as C;

/// The default key map, modelled on STMPS.
#[rustfmt::skip]
pub static BINDINGS: &[Binding] = &[
    // Global
    Binding { context: C::Global, keys: &[c('q'), c('Q')], action: A::Quit, help: "Quit (saves the queue on the server)" },
    Binding { context: C::Global, keys: &[c('?'), k(KeyCode::F(1))], action: A::Help, help: "Show this help" },
    Binding { context: C::Global, keys: &[c('1')], action: A::Page(0), help: "Browser" },
    Binding { context: C::Global, keys: &[c('2')], action: A::Page(1), help: "Queue" },
    Binding { context: C::Global, keys: &[c('3')], action: A::Page(2), help: "Playlists" },
    Binding { context: C::Global, keys: &[c('4')], action: A::Page(3), help: "Search" },
    Binding { context: C::Global, keys: &[c('5')], action: A::Page(4), help: "Log" },
    Binding { context: C::Global, keys: &[ctrl('n')], action: A::NextPage, help: "Next page" },
    Binding { context: C::Global, keys: &[ctrl('p')], action: A::PrevPage, help: "Previous page" },
    Binding { context: C::Global, keys: &[c('p'), c(' ')], action: A::TogglePause, help: "Play / pause" },
    Binding { context: C::Global, keys: &[c('P')], action: A::Stop, help: "Stop" },
    Binding { context: C::Global, keys: &[c('>')], action: A::Next, help: "Next track" },
    Binding { context: C::Global, keys: &[c('<')], action: A::Previous, help: "Previous track (restarts after 3 s)" },
    Binding { context: C::Global, keys: &[c('.')], action: A::SeekForward, help: "Seek forward" },
    Binding { context: C::Global, keys: &[c(',')], action: A::SeekBack, help: "Seek back" },
    Binding { context: C::Global, keys: &[c('='), c('+')], action: A::VolumeUp, help: "Volume up" },
    Binding { context: C::Global, keys: &[c('-')], action: A::VolumeDown, help: "Volume down" },
    Binding { context: C::Global, keys: &[c('m')], action: A::CycleRepeat, help: "Repeat: off, all, one" },
    Binding { context: C::Global, keys: &[c('r')], action: A::AddRandom, help: "Add random songs to the queue" },
    Binding { context: C::Global, keys: &[c('U')], action: A::Scan, help: "Start a library scan on the server" },
    Binding { context: C::Global, keys: &[c('Y')], action: A::StarPlaying, help: "Star / unstar the playing song" },
    Binding { context: C::Global, keys: &[ctrl('l')], action: A::Redraw, help: "Redraw the screen" },
    // Lists
    Binding { context: C::List, keys: &[c('k'), k(KeyCode::Up)], action: A::Up, help: "Up" },
    Binding { context: C::List, keys: &[c('j'), k(KeyCode::Down)], action: A::Down, help: "Down" },
    Binding { context: C::List, keys: &[k(KeyCode::PageUp), ctrl('u')], action: A::PageUp, help: "Page up" },
    Binding { context: C::List, keys: &[k(KeyCode::PageDown), ctrl('d')], action: A::PageDown, help: "Page down" },
    Binding { context: C::List, keys: &[c('g'), k(KeyCode::Home)], action: A::Top, help: "First entry" },
    Binding { context: C::List, keys: &[c('G'), k(KeyCode::End)], action: A::Bottom, help: "Last entry" },
    Binding { context: C::List, keys: &[c('h'), k(KeyCode::Left)], action: A::Left, help: "Left column / parent directory" },
    Binding { context: C::List, keys: &[c('l'), k(KeyCode::Right)], action: A::Right, help: "Right column / open" },
    Binding { context: C::List, keys: &[k(KeyCode::Tab)], action: A::FocusNext, help: "Next column" },
    Binding { context: C::List, keys: &[c('/')], action: A::Filter, help: "Filter the list (fuzzy)" },
    Binding { context: C::List, keys: &[k(KeyCode::Backspace), k(KeyCode::Esc)], action: A::Back, help: "Clear filter / go back" },
    // Browser
    Binding { context: C::Browser, keys: &[k(KeyCode::Enter)], action: A::Activate, help: "Open, or play from here" },
    Binding { context: C::Browser, keys: &[c('a')], action: A::Append, help: "Add to queue (directories recursively)" },
    Binding { context: C::Browser, keys: &[c('i')], action: A::PlayNext, help: "Play next" },
    Binding { context: C::Browser, keys: &[c('y')], action: A::Star, help: "Star / unstar" },
    Binding { context: C::Browser, keys: &[c('A')], action: A::AddToPlaylist, help: "Add to playlist" },
    Binding { context: C::Browser, keys: &[c('R')], action: A::Refresh, help: "Reload from the server" },
    Binding { context: C::Browser, keys: &[c('S')], action: A::Similar, help: "Queue similar songs" },
    // Queue
    Binding { context: C::Queue, keys: &[k(KeyCode::Enter)], action: A::Activate, help: "Play this song" },
    Binding { context: C::Queue, keys: &[c('d'), k(KeyCode::Delete)], action: A::Remove, help: "Remove from queue" },
    Binding { context: C::Queue, keys: &[c('D')], action: A::ClearQueue, help: "Clear the queue" },
    Binding { context: C::Queue, keys: &[c('K')], action: A::MoveUp, help: "Move song up" },
    Binding { context: C::Queue, keys: &[c('J')], action: A::MoveDown, help: "Move song down" },
    Binding { context: C::Queue, keys: &[c('y')], action: A::Star, help: "Star / unstar" },
    Binding { context: C::Queue, keys: &[c('A')], action: A::AddToPlaylist, help: "Add song to playlist" },
    Binding { context: C::Queue, keys: &[c('s')], action: A::SaveQueue, help: "Save queue as playlist" },
    Binding { context: C::Queue, keys: &[c('S')], action: A::Shuffle, help: "Shuffle" },
    Binding { context: C::Queue, keys: &[c('L')], action: A::LoadServerQueue, help: "Load the queue saved on the server" },
    Binding { context: C::Queue, keys: &[c('o')], action: A::JumpToPlaying, help: "Jump to the playing song" },
    // Playlists
    Binding { context: C::Playlists, keys: &[k(KeyCode::Enter)], action: A::Activate, help: "Open, or play from here" },
    Binding { context: C::Playlists, keys: &[c('a')], action: A::Append, help: "Add to queue" },
    Binding { context: C::Playlists, keys: &[c('i')], action: A::PlayNext, help: "Play next" },
    Binding { context: C::Playlists, keys: &[c('n')], action: A::NewPlaylist, help: "New playlist" },
    Binding { context: C::Playlists, keys: &[c('d'), k(KeyCode::Delete)], action: A::DeletePlaylist, help: "Delete playlist / remove song" },
    Binding { context: C::Playlists, keys: &[c('y')], action: A::Star, help: "Star / unstar song" },
    Binding { context: C::Playlists, keys: &[c('R')], action: A::Refresh, help: "Reload playlists" },
    // Search
    Binding { context: C::Search, keys: &[k(KeyCode::Enter)], action: A::Activate, help: "Play" },
    Binding { context: C::Search, keys: &[c('a')], action: A::Append, help: "Add to queue" },
    Binding { context: C::Search, keys: &[c('i')], action: A::PlayNext, help: "Play next" },
    Binding { context: C::Search, keys: &[c('y')], action: A::Star, help: "Star / unstar" },
    Binding { context: C::Search, keys: &[c('A')], action: A::AddToPlaylist, help: "Add to playlist" },
    Binding { context: C::Search, keys: &[c('e'), c('s')], action: A::EditQuery, help: "Edit the search" },
];

/// Find the action for a key in the given contexts, in order.
pub fn lookup(ev: &KeyEvent, contexts: &[Context]) -> Option<Action> {
    contexts.iter().find_map(|ctx| {
        BINDINGS
            .iter()
            .filter(|b| b.context == *ctx)
            .find(|b| b.keys.iter().any(|k| k.matches(ev)))
            .map(|b| b.action)
    })
}

/// Help lines grouped by context: (context, keys, description).
pub fn help_rows() -> Vec<(Context, String, &'static str)> {
    BINDINGS
        .iter()
        .map(|b| {
            let keys: Vec<String> = b.keys.iter().map(Key::label).collect();
            (b.context, keys.join(" "), b.help)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn page_context_wins() {
        let ev = key(KeyCode::Char('s'));
        assert_eq!(
            lookup(&ev, &[C::Queue, C::List, C::Global]),
            Some(A::SaveQueue)
        );
        assert_eq!(lookup(&ev, &[C::Browser, C::List, C::Global]), None);
    }

    #[test]
    fn shift_letters_and_ctrl() {
        let ev = KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT);
        assert_eq!(lookup(&ev, &[C::List]), Some(A::Bottom));
        let ev = KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL);
        assert_eq!(lookup(&ev, &[C::Queue, C::List]), Some(A::PageDown));
    }

    #[test]
    fn no_duplicate_keys_within_a_context() {
        for (i, a) in BINDINGS.iter().enumerate() {
            for b in &BINDINGS[i + 1..] {
                if a.context != b.context {
                    continue;
                }
                for ka in a.keys {
                    assert!(
                        !b.keys.contains(ka),
                        "{:?} bound twice in {:?}",
                        ka,
                        a.context
                    );
                }
            }
        }
    }
}
