// SPDX-License-Identifier: GPL-3.0-only
//! Parts shared by the pages: sidebar, player bar, song lists, album cards, dialogs.

use infrasonic_engine::{Command, PlayState, RepeatMode};
use infrasonic_subsonic::{Album, Artist, Child, Client, PlaylistUpdate, StarTarget};
use leptos::prelude::*;
use wasm_bindgen::JsCast;

use super::{App, Route, app};
use crate::i18n::{t, tf};

/// `3:07`, `1:02:03`.
pub fn fmt_time(secs: f64) -> String {
    if !secs.is_finite() || secs < 0.0 {
        return "0:00".into();
    }
    let s = secs as u64;
    let (h, m, s) = (s / 3600, (s / 60) % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// Total length of songs, `1 h 12 min` style.
pub fn fmt_total(secs: u64) -> String {
    let (h, m) = (secs / 3600, (secs / 60) % 60);
    if h > 0 {
        format!("{h} h {m} min")
    } else {
        format!("{m} min")
    }
}

/// "1 song", "12 songs".
pub fn songs_n(n: usize) -> String {
    if n == 1 {
        t("album.songs_one")
    } else {
        tf("album.songs_n", &[("n", &n)])
    }
}

pub fn cover_url(art: Option<&str>, size: u32) -> Option<String> {
    let art = art?;
    app()
        .client()
        .map(|c: Client| c.cover_art_url(art, Some(size)).to_string())
}

/// A cover image, or a neutral tile when there is none.
pub fn cover(art: Option<String>, size: u32, class: &'static str) -> AnyView {
    match cover_url(art.as_deref(), size) {
        Some(src) => view! { <img class=class src=src loading="lazy" alt=""/> }.into_any(),
        None => view! { <div class=format!("{class} nocover")>"♪"</div> }.into_any(),
    }
}

/// Load something from the server when the page opens and after [`App::reload`].
pub fn load<T, Fut>(f: impl Fn(Client) -> Fut + 'static) -> RwSignal<Option<T>>
where
    T: Send + Sync + 'static,
    Fut: std::future::Future<Output = infrasonic_subsonic::Result<T>> + 'static,
{
    let app = app();
    let data = RwSignal::new(None);
    Effect::new(move |_| {
        app.version.track();
        let Some(c) = app.client() else {
            return;
        };
        let fut = f(c);
        wasm_bindgen_futures::spawn_local(async move {
            match fut.await {
                Ok(v) => {
                    let _ = data.try_set(Some(v));
                }
                Err(e) => app.error(e.to_string()),
            }
        });
    });
    data
}

pub fn loading() -> AnyView {
    view! { <p class="muted">{t("common.loading")}</p> }.into_any()
}

pub fn focus_search() {
    if let Some(el) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id("search"))
        .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
    {
        let _ = el.focus();
    }
}

/// Copy text to the clipboard; show it either way, since the browser may refuse.
pub fn copy_text(app: App, text: String) {
    let Some(w) = web_sys::window() else { return };
    // The clipboard API needs a secure context; over plain HTTP just show the link.
    let nav = w.navigator();
    if !w.is_secure_context() || !js_sys::Reflect::has(&nav, &"clipboard".into()).unwrap_or(false) {
        app.info(tf("status.link", &[("url", &text)]));
        return;
    }
    let promise = nav.clipboard().write_text(&text);
    wasm_bindgen_futures::spawn_local(async move {
        match wasm_bindgen_futures::JsFuture::from(promise).await {
            Ok(_) => app.info(tf("status.copied", &[("url", &text)])),
            Err(_) => app.info(tf("status.link", &[("url", &text)])),
        }
    });
}

/// Star or unstar the playing song (key `l`).
pub fn toggle_star_current(app: App) {
    let Some(cur) = app.player.snapshot.get_untracked().current else {
        return;
    };
    if cur.track.id.starts_with("radio:") {
        return;
    }
    let mut track = cur.track.clone();
    track.starred = !track.starred;
    let star = track.starred;
    let p = app.player;
    app.star(StarTarget::id(track.id.clone()), star, move || {
        p.send(Command::UpdateTrack(track));
    });
}

// --- layout ----------------------------------------------------------------------------

#[component]
pub fn Sidebar() -> impl IntoView {
    let app = app();
    let query = RwSignal::new(String::new());
    let link = move |r: Route, key: &'static str, icon: &'static str| {
        let href = r.hash();
        let target = r.clone();
        let active = move || {
            let cur = app.route.get();
            std::mem::discriminant(&cur) == std::mem::discriminant(&target)
                || matches!((&cur, &target), (Route::Album(_), Route::Albums(_)))
        };
        view! {
            <a href=href class:active=active>
                <span class="icon">{icon}</span>
                <span class="label">{t(key)}</span>
            </a>
        }
    };
    view! {
        <nav class="sidebar">
            <div class="brand">"infrasonic"</div>
            <form
                class="search"
                on:submit=move |ev| {
                    ev.prevent_default();
                    let q = query.get_untracked();
                    if !q.trim().is_empty() {
                        app.go(Route::Search(q.trim().to_string()));
                    }
                }
            >
                <input
                    id="search"
                    type="search"
                    placeholder=t("nav.search")
                    prop:value=move || query.get()
                    on:input=move |ev| query.set(event_target_value(&ev))
                />
            </form>
            {link(Route::Albums("newest".into()), "nav.albums", "◫")}
            {link(Route::Artists, "nav.artists", "☺")}
            {link(Route::Playlists, "nav.playlists", "≣")}
            {link(Route::Favorites, "nav.favorites", "♥")}
            {link(Route::Radio, "nav.radio", "📻")}
            {link(Route::Shares, "nav.shares", "🔗")}
            {link(Route::Queue, "nav.queue", "☰")}
            {link(Route::Lyrics, "nav.lyrics", "✎")}
            {link(Route::Settings, "nav.settings", "⚙")}
        </nav>
    }
}

#[component]
pub fn PlayerBar() -> impl IntoView {
    let app = app();
    let p = app.player;
    let snap = p.snapshot;
    let current = move || snap.with(|s| s.current.clone());
    let playing =
        move || snap.with(|s| matches!(s.state, PlayState::Playing | PlayState::Buffering));
    let duration = move || {
        snap.with(|s| {
            s.duration
                .or(s.current.as_ref().and_then(|c| c.track.duration))
                .unwrap_or(0.0)
        })
    };
    // While the user drags the slider, it shows the drag position.
    let dragging = RwSignal::new(None::<f64>);
    let position = move || dragging.get().unwrap_or_else(|| snap.with(|s| s.position));
    let info = move || {
        current().map(|c| {
            let tr = c.track;
            let album = tr.album_id.clone();
            let artist = tr.artist_id.clone();
            let radio = tr.id.starts_with("radio:");
            view! {
                {cover(tr.cover_art.clone(), 96, "thumb")}
                <div class="now">
                    <a
                        class="title"
                        on:click=move |_| {
                            if let Some(a) = album.clone() {
                                app.go(Route::Album(a));
                            }
                        }
                    >
                        {tr.title.clone()}
                    </a>
                    <a
                        class="artist muted"
                        on:click=move |_| {
                            if let Some(a) = artist.clone() {
                                app.go(Route::Artist(a));
                            }
                        }
                    >
                        {tr.artist.clone()}
                    </a>
                </div>
                <Show when=move || !radio>
                    <button
                        class="icon-btn star"
                        class:on=move || snap.with(|s| s.current.as_ref().is_some_and(|c| c.track.starred))
                        title=t("action.star")
                        on:click=move |_| toggle_star_current(app)
                    >
                        "♥"
                    </button>
                </Show>
            }
        })
    };
    let repeat_icon = move || match snap.with(|s| s.repeat) {
        RepeatMode::Off => "↻",
        RepeatMode::All => "🔁",
        RepeatMode::One => "🔂",
    };
    view! {
        <footer class="player">
            <div class="track">{info}</div>
            <div class="controls">
                <div class="buttons">
                    <button class="icon-btn" title=t("action.previous") on:click=move |_| p.send(Command::Previous)>
                        "⏮"
                    </button>
                    <button
                        class="icon-btn play"
                        id="play"
                        title=t("action.play_pause")
                        on:click=move |_| p.send(Command::TogglePause)
                    >
                        {move || if playing() { "⏸" } else { "▶" }}
                    </button>
                    <button class="icon-btn" title=t("action.next") on:click=move |_| p.send(Command::Next)>
                        "⏭"
                    </button>
                    <button
                        class="icon-btn"
                        class:on=move || snap.with(|s| s.repeat != RepeatMode::Off)
                        title=move || t(match snap.with(|s| s.repeat) {
                            RepeatMode::Off => "repeat.off",
                            RepeatMode::All => "repeat.all",
                            RepeatMode::One => "repeat.one",
                        })
                        on:click=move |_| p.send(Command::SetRepeat(snap.with_untracked(|s| s.repeat).cycle()))
                    >
                        {repeat_icon}
                    </button>
                </div>
                <div class="seek">
                    <span class="time" id="position">{move || fmt_time(position())}</span>
                    <input
                        type="range"
                        min="0"
                        step="0.5"
                        prop:max=move || duration().max(1.0)
                        prop:value=position
                        on:input=move |ev| {
                            if let Ok(v) = event_target_value(&ev).parse::<f64>() {
                                dragging.set(Some(v));
                            }
                        }
                        on:change=move |ev| {
                            if let Ok(v) = event_target_value(&ev).parse::<f64>() {
                                p.send(Command::Seek(v));
                            }
                            dragging.set(None);
                        }
                    />
                    <span class="time">{move || fmt_time(duration())}</span>
                </div>
            </div>
            <div class="extra">
                <button class="icon-btn" title=t("nav.lyrics") on:click=move |_| app.go(Route::Lyrics)>
                    "✎"
                </button>
                <button class="icon-btn" title=t("nav.queue") on:click=move |_| app.go(Route::Queue)>
                    "☰"
                </button>
                <span class="icon">{move || if snap.with(|s| s.volume) == 0 { "🔇" } else { "🔊" }}</span>
                <input
                    type="range"
                    class="volume"
                    min="0"
                    max="100"
                    title=t("settings.volume")
                    prop:value=move || snap.with(|s| s.volume)
                    on:input=move |ev| {
                        if let Ok(v) = event_target_value(&ev).parse::<u8>() {
                            p.send(Command::SetVolume(v));
                            app.settings.update(|s| {
                                s.volume = v;
                                s.save();
                            });
                        }
                    }
                />
            </div>
        </footer>
    }
}

#[component]
pub fn Toast() -> impl IntoView {
    let app = app();
    move || {
        app.toast.get().map(|(text, err)| {
            view! {
                <div class="toast" class:error=err role="status" on:click=move |_| app.toast.set(None)>
                    {text}
                </div>
            }
        })
    }
}

#[component]
pub fn Help() -> impl IntoView {
    let app = app();
    let rows = [
        ("Space", "help.play_pause"),
        ("← →", "help.seek"),
        ("Shift + ← →", "help.skip"),
        ("= / -", "help.volume"),
        ("l", "help.star"),
        ("r", "help.repeat"),
        ("q", "help.queue"),
        ("y", "help.lyrics"),
        ("/", "help.search"),
        ("?", "help.help"),
        ("Esc", "help.close"),
    ];
    view! {
        <Show when=move || app.help.get()>
            <div class="modal" on:click=move |_| app.help.set(false)>
                <div class="dialog" on:click=|ev| ev.stop_propagation()>
                    <h2>{t("help.title")}</h2>
                    <table class="keys">
                        {rows
                            .iter()
                            .map(|(k, d)| view! { <tr><td><kbd>{*k}</kbd></td><td>{t(d)}</td></tr> })
                            .collect_view()}
                    </table>
                    <button on:click=move |_| app.help.set(false)>{t("common.close")}</button>
                </div>
            </div>
        </Show>
    }
}

/// "Add to playlist": pick one or create a new one.
#[component]
pub fn PlaylistPicker() -> impl IntoView {
    let app = app();
    let lists = RwSignal::new(None::<Vec<infrasonic_subsonic::Playlist>>);
    let name = RwSignal::new(String::new());
    Effect::new(move |_| {
        if app.picker.with(|p| p.is_some())
            && let Some(c) = app.client()
        {
            app.call(async move { c.get_playlists().await }, move |l| {
                lists.set(Some(l))
            });
        }
    });
    let add = move |id: String| {
        let Some(songs) = app.picker.get_untracked() else {
            return;
        };
        let Some(c) = app.client() else {
            return;
        };
        let n = songs.len();
        app.call(
            async move {
                c.update_playlist(
                    &id,
                    &PlaylistUpdate {
                        add_song_ids: songs,
                        ..PlaylistUpdate::default()
                    },
                )
                .await
            },
            move |()| {
                app.picker.set(None);
                app.info(tf("status.added_to_playlist", &[("n", &n)]));
                app.reload();
            },
        );
    };
    let create = move || {
        let Some(songs) = app.picker.get_untracked() else {
            return;
        };
        let n = name.get_untracked();
        if n.trim().is_empty() {
            return;
        }
        let Some(c) = app.client() else {
            return;
        };
        app.call(
            async move { c.create_playlist(n.trim(), &songs).await },
            move |_| {
                app.picker.set(None);
                name.set(String::new());
                app.info(t("status.playlist_created"));
                app.reload();
            },
        );
    };
    view! {
        <Show when=move || app.picker.with(|p| p.is_some())>
            <div class="modal" on:click=move |_| app.picker.set(None)>
                <div class="dialog" on:click=|ev| ev.stop_propagation()>
                    <h2>{t("playlist.add_to")}</h2>
                    <ul class="pick">
                        {move || {
                            lists
                                .get()
                                .map(|l| {
                                    l.into_iter()
                                        .map(|pl| {
                                            let id = pl.id.clone();
                                            view! {
                                                <li>
                                                    <button class="link" on:click=move |_| add(id.clone())>
                                                        {pl.name.clone()}
                                                    </button>
                                                </li>
                                            }
                                        })
                                        .collect_view()
                                })
                        }}
                    </ul>
                    <form
                        class="row"
                        on:submit=move |ev| {
                            ev.prevent_default();
                            create();
                        }
                    >
                        <input
                            placeholder=t("playlist.new_name")
                            prop:value=move || name.get()
                            on:input=move |ev| name.set(event_target_value(&ev))
                        />
                        <button type="submit">{t("playlist.create")}</button>
                    </form>
                    <button class="secondary" on:click=move |_| app.picker.set(None)>
                        {t("common.cancel")}
                    </button>
                </div>
            </div>
        </Show>
    }
}

// --- songs, albums, artists ------------------------------------------------------------

/// Clickable stars 1–5; a click on the set rating clears it.
pub fn stars(id: String, rating: Option<u8>) -> impl IntoView {
    let app = app();
    let value = RwSignal::new(rating.unwrap_or(0));
    view! {
        <span class="stars" title=t("action.rate")>
            {(1..=5u8)
                .map(|n| {
                    let id = id.clone();
                    view! {
                        <button
                            class="star-btn"
                            class:on={move || value.get() >= n}
                            on:click=move |ev| {
                                ev.stop_propagation();
                                let new = if value.get_untracked() == n { 0 } else { n };
                                let Some(c) = app.client() else { return };
                                let id = id.clone();
                                app.call(
                                    async move { c.set_rating(&id, new).await },
                                    move |()| value.set(new),
                                );
                            }
                        >
                            "★"
                        </button>
                    }
                })
                .collect_view()}
        </span>
    }
}

/// A heart that stars and unstars.
pub fn star_button(target: StarTarget, starred: bool) -> impl IntoView {
    let app = app();
    let on = RwSignal::new(starred);
    view! {
        <button
            class="icon-btn star"
            class:on=move || on.get()
            title=t("action.star")
            on:click=move |ev| {
                ev.stop_propagation();
                let new = !on.get_untracked();
                app.star(target.clone(), new, move || on.set(new));
            }
        >
            "♥"
        </button>
    }
}

/// How a song list looks and what it can do.
#[derive(Clone, Default)]
pub struct ListOpts {
    pub show_album: bool,
    pub show_track: bool,
    /// Called with (from, to) when a row is dragged elsewhere.
    pub on_move: Option<Callback<(usize, usize)>>,
    /// Shows a remove button per row.
    pub on_remove: Option<Callback<usize>>,
}

/// A table of songs. Clicking a title plays the list from there.
pub fn song_list(songs: Vec<Child>, opts: ListOpts) -> impl IntoView {
    let app = app();
    let songs = StoredValue::new(songs);
    let drag = StoredValue::new(None::<usize>);
    let over = RwSignal::new(None::<usize>);
    let playing_id = move || {
        app.player
            .snapshot
            .with(|s| s.current.as_ref().map(|c| c.track.id.clone()))
    };
    let rows = songs.with_value(|list| {
        list.iter()
            .enumerate()
            .map(|(i, s)| {
                let s = s.clone();
                let id = s.id.clone();
                let id2 = id.clone();
                let current = move || playing_id().as_deref() == Some(id2.as_str());
                let album_id = s.album_id.clone();
                let artist_id = s.artist_id.clone();
                let movable = opts.on_move.is_some();
                let on_move = opts.on_move;
                let on_remove = opts.on_remove;
                let one = s.clone();
                let one2 = s.clone();
                let song_id = id.clone();
                view! {
                    <tr
                        class:current=current
                        class:over=move || over.get() == Some(i)
                        draggable=if movable { "true" } else { "false" }
                        on:dragstart=move |ev| {
                            drag.set_value(Some(i));
                            if let Some(dt) = ev.data_transfer() {
                                let _ = dt.set_data("text/plain", &i.to_string());
                                dt.set_effect_allowed("move");
                            }
                        }
                        on:dragover=move |ev| {
                            if movable {
                                ev.prevent_default();
                                over.set(Some(i));
                            }
                        }
                        on:dragleave=move |_| over.set(None)
                        on:drop=move |ev| {
                            ev.prevent_default();
                            over.set(None);
                            if let (Some(from), Some(cb)) = (drag.get_value(), on_move)
                                && from != i
                            {
                                cb.run((from, i));
                            }
                            drag.set_value(None);
                        }
                    >
                        <td class="num muted">
                            {if opts.show_track { s.track.map(|n| n.to_string()).unwrap_or_default() } else { (i + 1).to_string() }}
                        </td>
                        <td class="title">
                            <a on:click=move |_| songs.with_value(|l| app.player.play_songs(l, i))>
                                {s.title.clone()}
                            </a>
                            <div class="sub muted">
                                <a on:click=move |_| {
                                    if let Some(a) = artist_id.clone() {
                                        app.go(Route::Artist(a));
                                    }
                                }>{s.artist_name().to_string()}</a>
                                {opts.show_album.then(|| view! {
                                    " · "
                                    <a on:click=move |_| {
                                        if let Some(a) = album_id.clone() {
                                            app.go(Route::Album(a));
                                        }
                                    }>{s.album.clone().unwrap_or_default()}</a>
                                })}
                            </div>
                        </td>
                        <td class="rating">{stars(id.clone(), s.user_rating.filter(|r| *r > 0))}</td>
                        <td class="actions">
                            {star_button(StarTarget::id(id.clone()), s.is_starred())}
                            <button class="icon-btn" title=t("action.play_next") on:click=move |_| app.player.play_next(std::slice::from_ref(&one))>
                                "⤵"
                            </button>
                            <button class="icon-btn" title=t("action.enqueue") on:click=move |_| app.player.enqueue(std::slice::from_ref(&one2))>
                                "+"
                            </button>
                            <button class="icon-btn" title=t("playlist.add_to") on:click=move |_| app.picker.set(Some(vec![song_id.clone()]))>
                                "≣"
                            </button>
                            {on_remove.map(|cb| view! {
                                <button class="icon-btn" title=t("action.remove") on:click=move |_| cb.run(i)>"✕"</button>
                            })}
                        </td>
                        <td class="dur muted">{fmt_time(f64::from(s.duration.unwrap_or(0)))}</td>
                    </tr>
                }
            })
            .collect_view()
    });
    view! { <table class="songs"><tbody>{rows}</tbody></table> }
}

pub fn album_card(a: &Album) -> impl IntoView + use<> {
    let app = app();
    let id = a.id.clone();
    let id2 = a.id.clone();
    let artist = a.artist_id.clone();
    let play = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        let Some(c) = app.client() else { return };
        let id = id2.clone();
        app.call(async move { c.get_album(&id).await }, move |al| {
            app.player.play_songs(&al.song, 0);
        });
    };
    view! {
        <div class="card">
            <div class="art" on:click=move |_| app.go(Route::Album(id.clone()))>
                {cover(a.cover_art.clone(), 300, "cover")}
                <button class="play-over" title=t("action.play") on:click=play>"▶"</button>
            </div>
            <a class="name" href=Route::Album(a.id.clone()).hash()>{a.display_name().to_string()}</a>
            <a
                class="muted sub"
                on:click=move |_| {
                    if let Some(a) = artist.clone() {
                        app.go(Route::Artist(a));
                    }
                }
            >
                {a.artist.clone().unwrap_or_default()}
            </a>
            <span class="muted sub">{a.year.map(|y| y.to_string()).unwrap_or_default()}</span>
        </div>
    }
}

pub fn album_grid(albums: &[Album]) -> impl IntoView + use<> {
    view! { <div class="grid">{albums.iter().map(album_card).collect_view()}</div> }
}

pub fn artist_card(a: &Artist) -> impl IntoView + use<> {
    view! {
        <a class="card artist-card" href=Route::Artist(a.id.clone()).hash()>
            {cover(a.cover_art.clone(), 300, "cover round")}
            <span class="name">{a.name.clone()}</span>
            <span class="muted sub">
                {a.album_count.map(|n| tf("artist.albums_n", &[("n", &n)])).unwrap_or_default()}
            </span>
        </a>
    }
}

/// Buttons for a set of songs: play, shuffle, add, add to playlist.
pub fn song_actions(songs: Vec<Child>) -> impl IntoView {
    let app = app();
    let songs = StoredValue::new(songs);
    view! {
        <div class="actions-row">
            <button class="primary" on:click=move |_| songs.with_value(|s| app.player.play_songs(s, 0))>
                "▶ " {t("action.play")}
            </button>
            <button on:click=move |_| {
                songs.with_value(|s| app.player.play_songs(s, 0));
                app.player.send(Command::Shuffle);
            }>
                "⤮ " {t("action.shuffle")}
            </button>
            <button on:click=move |_| songs.with_value(|s| app.player.enqueue(s))>
                "+ " {t("action.enqueue")}
            </button>
            <button on:click=move |_| songs.with_value(|s| app.picker.set(Some(s.iter().map(|c| c.id.clone()).collect())))>
                "≣ " {t("playlist.add_to")}
            </button>
        </div>
    }
}

/// Create a public link to `id` (album, playlist or song) and copy it.
pub fn share(app: App, id: String, description: String) {
    let Some(c) = app.client() else { return };
    app.call(
        async move { c.create_share(&[&id], Some(&description), None).await },
        move |s| {
            app.reload();
            copy_text(app, s.url);
        },
    );
}
