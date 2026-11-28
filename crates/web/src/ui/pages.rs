// SPDX-License-Identifier: GPL-3.0-only
//! The pages behind the routes.

use infrasonic_engine::{Command, PlayState};
use infrasonic_subsonic::{
    Album, AlbumListType, Child, PlaylistUpdate, SearchOptions, StarTarget, StructuredLyrics,
};
use leptos::prelude::*;
use wasm_bindgen::JsCast;

use super::icons::{Icon, icon};
use super::widgets::{
    ListOpts, album_grid, artist_card, cover, fmt_time, fmt_total, load, loading, no_select,
    on_row, share, song_actions, song_list, songs_n, star_button, stars,
};
use super::{App, Route, app, apply_theme, playback_settings, replay_gain};
use crate::i18n::{t, tf};
use crate::store::{Profile, Settings, web_link};

pub fn page(r: Route) -> AnyView {
    match r {
        Route::Login => view! { <Login/> }.into_any(),
        Route::Albums(list) => albums(list).into_any(),
        Route::Genre(g) => genre(g).into_any(),
        Route::Years(a, b) => years(a, b).into_any(),
        Route::Album(id) => album(id).into_any(),
        Route::Artists => artists().into_any(),
        Route::Artist(id) => artist(id).into_any(),
        Route::Playlists => playlists().into_any(),
        Route::Playlist(id) => playlist(id).into_any(),
        Route::Search(q) => search(q).into_any(),
        Route::Queue => queue().into_any(),
        Route::Favorites => favorites().into_any(),
        Route::Radio => radio().into_any(),
        Route::Shares => shares().into_any(),
        Route::Settings => settings().into_any(),
        Route::Lyrics => lyrics().into_any(),
    }
}

/// A server URL preset by the deployment (`config.js`, see the Docker image).
fn preset(key: &str) -> Option<String> {
    let w = web_sys::window()?;
    let cfg = js_sys::Reflect::get(&w, &"INFRASONIC_CONFIG".into()).ok()?;
    js_sys::Reflect::get(&cfg, &key.into())
        .ok()?
        .as_string()
        .filter(|s| !s.is_empty())
}

fn text_input(sig: RwSignal<String>, kind: &'static str, id: &'static str) -> impl IntoView {
    view! {
        <input
            id=id
            type=kind
            prop:value=move || sig.get()
            on:input=move |ev| sig.set(event_target_value(&ev))
        />
    }
}

#[component]
pub fn Login() -> impl IntoView {
    let app = app();
    let fixed = preset("server");
    let url = RwSignal::new(
        fixed
            .clone()
            .or_else(|| {
                app.profiles
                    .with_untracked(|p| p.current().map(|c| c.url.clone()))
            })
            .unwrap_or_default(),
    );
    let user = RwSignal::new(String::new());
    let password = RwSignal::new(String::new());
    let api_key = RwSignal::new(String::new());
    let legacy = RwSignal::new(false);
    let busy = RwSignal::new(false);
    let submit = move |ev: web_sys::SubmitEvent| {
        ev.prevent_default();
        let p = Profile::new(
            &url.get_untracked(),
            &user.get_untracked(),
            &password.get_untracked(),
            &api_key.get_untracked(),
            legacy.get_untracked(),
        );
        busy.set(true);
        app.player.send(Command::Clear);
        app.login(p);
        // `login` reports the result; allow a new try in any case.
        let cb = wasm_bindgen::closure::Closure::once_into_js(move || {
            let _ = busy.try_set(false);
        });
        if let Some(w) = web_sys::window() {
            let _ =
                w.set_timeout_with_callback_and_timeout_and_arguments_0(cb.unchecked_ref(), 1500);
        }
    };
    let saved = move || {
        app.profiles
            .get()
            .list
            .into_iter()
            .map(|p| {
                let key = p.key();
                let label = format!("{} · {}", p.name, p.username);
                let p2 = p.clone();
                view! {
                    <li>
                        <button class="link" on:click=move |_| {
                            app.player.send(Command::Clear);
                            app.login(p2.clone());
                        }>{label}</button>
                        <button class="icon-btn" title=t("action.remove") on:click=move |_| {
                            app.profiles.update(|ps| {
                                ps.remove(&key);
                                ps.save();
                            });
                        }>{icon(Icon::Remove)}</button>
                    </li>
                }
            })
            .collect_view()
    };
    view! {
        <div class="login">
            <form class="dialog" on:submit=submit>
                <h1>"infrasonic"</h1>
                <p class="muted">{t("login.intro")}</p>
                <Show when={
                    let f = fixed.is_some();
                    move || !f
                }>
                    <label for="url">{t("login.server")}</label>
                    {text_input(url, "url", "url")}
                </Show>
                <label for="user">{t("login.username")}</label>
                {text_input(user, "text", "user")}
                <label for="password">{t("login.password")}</label>
                {text_input(password, "password", "password")}
                <details>
                    <summary>{t("login.more")}</summary>
                    <label for="apikey">{t("login.api_key")}</label>
                    {text_input(api_key, "password", "apikey")}
                    <p class="hint muted">{t("login.api_key_hint")}</p>
                    <label class="check">
                        <input
                            type="checkbox"
                            prop:checked=move || legacy.get()
                            on:change=move |ev| legacy.set(event_target_checked(&ev))
                        />
                        {t("login.legacy")}
                    </label>
                </details>
                <button type="submit" class="primary" id="login" disabled=move || busy.get()>
                    {t("login.connect")}
                </button>
                <p class="hint muted">{t("login.privacy")}</p>
                <Show when=move || app.profiles.with(|p| !p.list.is_empty())>
                    <h3>{t("login.saved")}</h3>
                    <ul class="pick">{saved}</ul>
                </Show>
            </form>
        </div>
    }
}

// --- albums ----------------------------------------------------------------------------

const LISTS: [(&str, &str); 8] = [
    ("newest", "albums.newest"),
    ("recent", "albums.recent"),
    ("frequent", "albums.frequent"),
    ("highest", "albums.highest"),
    ("starred", "albums.starred"),
    ("random", "albums.random"),
    ("alphabeticalByName", "albums.by_name"),
    ("alphabeticalByArtist", "albums.by_artist"),
];

const PAGE: u32 = 60;

fn list_type(name: &str) -> AlbumListType {
    match name {
        "recent" => AlbumListType::Recent,
        "frequent" => AlbumListType::Frequent,
        "highest" => AlbumListType::Highest,
        "starred" => AlbumListType::Starred,
        "random" => AlbumListType::Random,
        "alphabeticalByName" => AlbumListType::AlphabeticalByName,
        "alphabeticalByArtist" => AlbumListType::AlphabeticalByArtist,
        _ => AlbumListType::Newest,
    }
}

/// Tabs above the album grid; `active` is a list name, `genre:<g>` or `years`.
fn album_tabs(active: String) -> impl IntoView {
    let app = app();
    let genres = load(|c| async move { c.get_genres().await });
    let from = RwSignal::new(String::new());
    let to = RwSignal::new(String::new());
    let active2 = active.clone();
    view! {
        <div class="tabs">
            {LISTS
                .iter()
                .map(|(name, key)| {
                    let on = *name == active;
                    view! { <a class:active=on href=Route::Albums((*name).into()).hash()>{t(key)}</a> }
                })
                .collect_view()}
        </div>
        <div class="filters">
            <select on:change=move |ev| {
                let g = event_target_value(&ev);
                if !g.is_empty() {
                    app.go(Route::Genre(g));
                }
            }>
                <option value="">{t("albums.genre")}</option>
                {move || {
                    let cur = active2.strip_prefix("genre:").map(str::to_string);
                    genres.get().map(|mut gs| {
                        gs.sort_by_key(|g| g.value.to_lowercase());
                        gs.into_iter()
                            .map(|g| {
                                let sel = cur.as_deref() == Some(g.value.as_str());
                                view! { <option value=g.value.clone() selected=sel>{g.value.clone()}</option> }
                            })
                            .collect_view()
                    })
                }}
            </select>
            <form class="years" on:submit=move |ev| {
                ev.prevent_default();
                let a = from.get_untracked().parse::<i32>();
                let b = to.get_untracked().parse::<i32>();
                match (a, b) {
                    (Ok(a), Ok(b)) => app.go(Route::Years(a, b)),
                    (Ok(a), Err(_)) => app.go(Route::Years(a, a)),
                    _ => {}
                }
            }>
                <input type="number" placeholder=t("albums.from_year") prop:value=move || from.get()
                    on:input=move |ev| from.set(event_target_value(&ev))/>
                "–"
                <input type="number" placeholder=t("albums.to_year") prop:value=move || to.get()
                    on:input=move |ev| to.set(event_target_value(&ev))/>
                <button type="submit">{t("albums.show")}</button>
            </form>
        </div>
    }
}

/// An album grid that loads pages of `PAGE` with a "more" button.
fn paged_albums<F, Fut>(fetch: F) -> impl IntoView
where
    F: Fn(infrasonic_subsonic::Client, u32) -> Fut + Copy + Send + Sync + 'static,
    Fut: std::future::Future<Output = infrasonic_subsonic::Result<Vec<Album>>> + 'static,
{
    let app = app();
    let albums = RwSignal::new(None::<Vec<Album>>);
    let more = RwSignal::new(false);
    let fetch_page = move |offset: u32| {
        let Some(c) = app.client() else { return };
        app.call(fetch(c, offset), move |page: Vec<Album>| {
            more.set(page.len() as u32 == PAGE);
            albums.update(|a| a.get_or_insert_with(Vec::new).extend(page));
        });
    };
    Effect::new(move |_| {
        app.version.track();
        albums.set(None);
        fetch_page(0);
    });
    view! {
        {move || match albums.get() {
            None => loading(),
            Some(a) if a.is_empty() => view! { <p class="muted">{t("common.empty")}</p> }.into_any(),
            Some(a) => album_grid(&a).into_any(),
        }}
        <Show when=move || more.get()>
            <button class="more" on:click=move |_| {
                let n = albums.with_untracked(|a| a.as_ref().map_or(0, Vec::len)) as u32;
                fetch_page(n);
            }>{t("common.more")}</button>
        </Show>
    }
}

fn albums(list: String) -> impl IntoView {
    let ty = list_type(&list);
    view! {
        <h1>{t("nav.albums")}</h1>
        {album_tabs(list)}
        {paged_albums(move |c, off| async move { c.get_album_list2(ty, PAGE, off).await })}
    }
}

fn genre(g: String) -> impl IntoView {
    let name = StoredValue::new(g.clone());
    view! {
        <h1>{t("albums.genre")} ": " {g.clone()}</h1>
        {album_tabs(format!("genre:{g}"))}
        {paged_albums(move |c, off| async move {
            c.get_albums_by_genre(&name.get_value(), PAGE, off).await
        })}
    }
}

fn years(a: i32, b: i32) -> impl IntoView {
    let ty = AlbumListType::ByYear { from: a, to: b };
    view! {
        <h1>{t("nav.albums")} " " {if a == b { a.to_string() } else { format!("{a}–{b}") }}</h1>
        {album_tabs("years".into())}
        {paged_albums(move |c, off| async move { c.get_album_list2(ty, PAGE, off).await })}
    }
}

fn album(id: String) -> impl IntoView {
    let app = app();
    let data = load({
        let id = id.clone();
        move |c| {
            let id = id.clone();
            async move { c.get_album(&id).await }
        }
    });
    move || {
        let Some(a) = data.get() else {
            return loading();
        };
        let total: u64 = a
            .song
            .iter()
            .map(|s| u64::from(s.duration.unwrap_or(0)))
            .sum();
        let artist = a.artist_id.clone();
        let (id, name) = (a.id.clone(), a.display_name().to_string());
        view! {
            <header class="head">
                {cover(a.cover_art.clone(), 600, "big")}
                <div>
                    <p class="muted kind">{t("album.kind")}</p>
                    <h1>{a.display_name().to_string()}</h1>
                    <p>
                        <a on:click=move |_| {
                            if let Some(id) = artist.clone() {
                                app.go(Route::Artist(id));
                            }
                        }>{a.artist.clone().unwrap_or_default()}</a>
                    </p>
                    <p class="muted">
                        {a.year.map(|y| format!("{y} · ")).unwrap_or_default()}
                        {a.genre.clone().map(|g| format!("{g} · ")).unwrap_or_default()}
                        {songs_n(a.song.len())}
                        " · " {fmt_total(total)}
                    </p>
                    <div class="row">
                        {star_button(StarTarget::album(a.id.clone()), a.starred.is_some())}
                        {stars(a.id.clone(), a.user_rating.filter(|r| *r > 0))}
                        <button class="icon-btn" title=t("action.share") on:click=move |_| share(app, id.clone(), name.clone())>
                            {icon(Icon::Share)}
                        </button>
                    </div>
                    {song_actions(a.song.clone())}
                </div>
            </header>
            {song_list(a.song.clone(), ListOpts { show_track: true, ..ListOpts::default() })}
        }
        .into_any()
    }
}

// --- artists ---------------------------------------------------------------------------

fn artists() -> impl IntoView {
    let data = load(|c| async move { c.get_artists().await });
    view! {
        <h1>{t("nav.artists")}</h1>
        {move || {
            let Some(a) = data.get() else { return loading() };
            view! {
                <div class="letters">
                    {a.index.iter().map(|i| view! { <a href=format!("#/artists") on:click={
                        let n = i.name.clone();
                        move |ev: web_sys::MouseEvent| {
                            ev.prevent_default();
                            if let Some(el) = web_sys::window().and_then(|w| w.document()).and_then(|d| d.get_element_by_id(&format!("letter-{n}"))) {
                                el.scroll_into_view();
                            }
                        }
                    }>{i.name.clone()}</a> }).collect_view()}
                </div>
                {a.index.iter().map(|i| view! {
                    <h2 id=format!("letter-{}", i.name)>{i.name.clone()}</h2>
                    <div class="grid">{i.artist.iter().map(artist_card).collect_view()}</div>
                }).collect_view()}
            }
            .into_any()
        }}
    }
}

/// Last.fm biographies come with HTML links; keep the text only.
fn strip_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .trim()
        .to_string()
}

fn artist(id: String) -> impl IntoView {
    let app = app();
    let data = load({
        let id = id.clone();
        move |c| {
            let id = id.clone();
            async move { c.get_artist(&id).await }
        }
    });
    let info = load({
        let id = id.clone();
        move |c| {
            let id = id.clone();
            async move { c.get_artist_info2(&id, 12).await }
        }
    });
    let top = RwSignal::new(None::<Vec<Child>>);
    Effect::new(move |done: Option<bool>| {
        if done == Some(true) {
            return true;
        }
        let Some(name) = data.with(|d| d.as_ref().map(|a| a.name.clone())) else {
            return false;
        };
        if let Some(c) = app.client() {
            wasm_bindgen_futures::spawn_local(async move {
                // Not every server has top songs (Last.fm); the page works without.
                let songs = c.get_top_songs(&name, 10).await.unwrap_or_default();
                let _ = top.try_set(Some(songs));
            });
        }
        true
    });
    let radio = {
        let id = id.clone();
        move |_| {
            let Some(c) = app.client() else { return };
            let id = id.clone();
            app.call(
                async move { c.get_similar_songs2(&id, 50).await },
                move |songs: Vec<Child>| {
                    if songs.is_empty() {
                        app.info(t("artist.no_similar"));
                    } else {
                        app.player.play_songs(&songs, 0);
                    }
                },
            );
        }
    };
    move || {
        let Some(a) = data.get() else {
            return loading();
        };
        let radio = radio.clone();
        let image = info
            .with(|i| {
                i.as_ref()
                    .and_then(|i| i.large_image_url.clone().or(i.medium_image_url.clone()))
            })
            .filter(|u| !u.is_empty() && !u.contains("2a96cbd8b46e442fc41c2b86b821562f"));
        let mut albums = a.album.clone();
        albums.sort_by_key(|x| std::cmp::Reverse(x.year.unwrap_or(0)));
        view! {
            <header class="head">
                {match image {
                    Some(src) => view! { <img class="big round" src=src alt=""/> }.into_any(),
                    None => cover(a.cover_art.clone(), 600, "big round"),
                }}
                <div>
                    <p class="muted kind">{t("artist.kind")}</p>
                    <h1>{a.name.clone()}</h1>
                    <p class="muted">{tf("artist.albums_n", &[("n", &a.album.len())])}</p>
                    <div class="row">
                        {star_button(StarTarget::artist(a.id.clone()), a.starred.is_some())}
                        <button on:click=radio>{icon(Icon::Radio)} {t("artist.radio")}</button>
                    </div>
                    {move || info.get().and_then(|i| i.biography).map(|b| strip_tags(&b)).filter(|b| !b.is_empty()).map(|b| view! {
                        <details class="bio"><summary>{t("artist.bio")}</summary><p>{b}</p></details>
                    })}
                </div>
            </header>
            {move || top.get().filter(|s| !s.is_empty()).map(|s| view! {
                <h2>{t("artist.top")}</h2>
                {song_actions(s.clone())}
                {song_list(s, ListOpts { show_album: true, ..ListOpts::default() })}
            })}
            <h2>{t("nav.albums")}</h2>
            {album_grid(&albums)}
            {move || info.get().map(|i| i.similar_artist).filter(|s| !s.is_empty()).map(|s| view! {
                <h2>{t("artist.similar")}</h2>
                <div class="grid">{s.iter().map(artist_card).collect_view()}</div>
            })}
        }
        .into_any()
    }
}

// --- playlists -------------------------------------------------------------------------

fn playlists() -> impl IntoView {
    let app = app();
    let data = load(|c| async move { c.get_playlists().await });
    let name = RwSignal::new(String::new());
    let create = move |ev: web_sys::SubmitEvent| {
        ev.prevent_default();
        let n = name.get_untracked();
        if n.trim().is_empty() {
            return;
        }
        let Some(c) = app.client() else { return };
        app.call(
            async move { c.create_playlist(n.trim(), &[]).await },
            move |_| {
                name.set(String::new());
                app.reload();
            },
        );
    };
    view! {
        <h1>{t("nav.playlists")}</h1>
        <form class="row" on:submit=create>
            <input placeholder=t("playlist.new_name") prop:value=move || name.get()
                on:input=move |ev| name.set(event_target_value(&ev))/>
            <button type="submit">{t("playlist.create")}</button>
        </form>
        {move || {
            let Some(list) = data.get() else { return loading() };
            if list.is_empty() {
                return view! { <p class="muted">{t("common.empty")}</p> }.into_any();
            }
            view! {
                <div class="grid">
                    {list.iter().map(|p| view! {
                        <a class="card" href=Route::Playlist(p.id.clone()).hash()>
                            {cover(p.cover_art.clone(), 300, "cover")}
                            <span class="name">{p.name.clone()}</span>
                            <span class="muted sub">
                                {songs_n(p.song_count.unwrap_or(0) as usize)}
                                {p.owner.clone().map(|o| format!(" · {o}")).unwrap_or_default()}
                            </span>
                        </a>
                    }).collect_view()}
                </div>
            }
            .into_any()
        }}
    }
}

fn playlist(id: String) -> impl IntoView {
    let app = app();
    let id = StoredValue::new(id);
    let data = load(move |c| {
        let id = id.get_value();
        async move { c.get_playlist(&id).await }
    });
    let on_move = Callback::new(move |(from, to): (usize, usize)| {
        let Some(mut pl) = data.get_untracked() else {
            return;
        };
        if from >= pl.entry.len() || to >= pl.entry.len() {
            return;
        }
        let s = pl.entry.remove(from);
        pl.entry.insert(to, s);
        let ids: Vec<String> = pl.entry.iter().map(|e| e.id.clone()).collect();
        let Some(c) = app.client() else { return };
        app.call(
            async move { c.replace_playlist(&id.get_value(), &ids).await },
            move |()| data.set(Some(pl)),
        );
    });
    let on_remove = Callback::new(move |i: usize| {
        let Some(c) = app.client() else { return };
        app.call(
            async move {
                c.update_playlist(
                    &id.get_value(),
                    &PlaylistUpdate {
                        remove_indexes: vec![i],
                        ..PlaylistUpdate::default()
                    },
                )
                .await
            },
            move |()| app.reload(),
        );
    });
    let delete = move |_| {
        let ok = web_sys::window()
            .and_then(|w| w.confirm_with_message(&t("playlist.confirm_delete")).ok())
            .unwrap_or(false);
        if !ok {
            return;
        }
        let Some(c) = app.client() else { return };
        app.call(
            async move { c.delete_playlist(&id.get_value()).await },
            move |()| {
                app.go(Route::Playlists);
            },
        );
    };
    move || {
        let Some(p) = data.get() else {
            return loading();
        };
        let total: u64 = p
            .entry
            .iter()
            .map(|s| u64::from(s.duration.unwrap_or(0)))
            .sum();
        let name = p.name.clone();
        view! {
            <header class="head">
                {cover(p.cover_art.clone(), 600, "big")}
                <div>
                    <p class="muted kind">{t("playlist.kind")}</p>
                    <h1>{p.name.clone()}</h1>
                    <p class="muted">
                        {p.comment.clone().map(|c| format!("{c} · ")).unwrap_or_default()}
                        {songs_n(p.entry.len())} " · " {fmt_total(total)}
                    </p>
                    <div class="row">
                        <button class="icon-btn" title=t("action.share") on:click=move |_| share(app, id.get_value(), name.clone())>{icon(Icon::Share)}</button>
                        <button class="icon-btn" title=t("playlist.delete") on:click=delete>{icon(Icon::Delete)}</button>
                    </div>
                    {song_actions(p.entry.clone())}
                </div>
            </header>
            <p class="hint muted">{t("playlist.drag_hint")}</p>
            {song_list(p.entry.clone(), ListOpts {
                show_album: true,
                on_move: Some(on_move),
                on_remove: Some(on_remove),
                ..ListOpts::default()
            })}
        }
        .into_any()
    }
}

// --- search, favorites -----------------------------------------------------------------

fn search(q: String) -> impl IntoView {
    let query = StoredValue::new(q.clone());
    let data = load(move |c| {
        let q = query.get_value();
        async move {
            c.search3(
                &q,
                SearchOptions {
                    artist_count: 12,
                    album_count: 24,
                    song_count: 50,
                    ..SearchOptions::default()
                },
            )
            .await
        }
    });
    view! {
        <h1>{tf("search.title", &[("q", &q)])}</h1>
        {move || {
            let Some(r) = data.get() else { return loading() };
            if r.artist.is_empty() && r.album.is_empty() && r.song.is_empty() {
                return view! { <p class="muted">{t("search.nothing")}</p> }.into_any();
            }
            view! {
                {(!r.artist.is_empty()).then(|| view! {
                    <h2>{t("nav.artists")}</h2>
                    <div class="grid">{r.artist.iter().map(artist_card).collect_view()}</div>
                })}
                {(!r.album.is_empty()).then(|| view! { <h2>{t("nav.albums")}</h2> {album_grid(&r.album)} })}
                {(!r.song.is_empty()).then(|| view! {
                    <h2>{t("search.songs")}</h2>
                    {song_list(r.song.clone(), ListOpts { show_album: true, ..ListOpts::default() })}
                })}
            }
            .into_any()
        }}
    }
}

fn favorites() -> impl IntoView {
    let data = load(|c| async move { c.get_starred2().await });
    view! {
        <h1>{t("nav.favorites")}</h1>
        {move || {
            let Some(r) = data.get() else { return loading() };
            if r.artist.is_empty() && r.album.is_empty() && r.song.is_empty() {
                return view! { <p class="muted">{t("favorites.empty")}</p> }.into_any();
            }
            view! {
                {(!r.song.is_empty()).then(|| view! {
                    <h2>{t("search.songs")}</h2>
                    {song_actions(r.song.clone())}
                    {song_list(r.song.clone(), ListOpts { show_album: true, ..ListOpts::default() })}
                })}
                {(!r.album.is_empty()).then(|| view! { <h2>{t("nav.albums")}</h2> {album_grid(&r.album)} })}
                {(!r.artist.is_empty()).then(|| view! {
                    <h2>{t("nav.artists")}</h2>
                    <div class="grid">{r.artist.iter().map(artist_card).collect_view()}</div>
                })}
            }
            .into_any()
        }}
    }
}

// --- queue -----------------------------------------------------------------------------

fn queue() -> impl IntoView {
    let app = app();
    let p = app.player;
    let drag = StoredValue::new(None::<usize>);
    let over = RwSignal::new(None::<usize>);
    let save = move |_| {
        let ids: Vec<String> = p.queue.with_untracked(|q| {
            q.iter()
                .filter(|e| !e.track.id.starts_with("radio:"))
                .map(|e| e.track.id.clone())
                .collect()
        });
        app.picker.set(Some(ids));
    };
    let rows = move || {
        let current = p.snapshot.with(|s| s.current_index);
        p.queue
            .get()
            .into_iter()
            .enumerate()
            .map(|(i, e)| {
                let tr = e.track;
                let album = tr.album_id.clone();
                view! {
                    <tr
                        class:current=current == Some(i)
                        class:over=move || over.get() == Some(i)
                        draggable="true"
                        on:dragstart=move |ev| {
                            drag.set_value(Some(i));
                            if let Some(dt) = ev.data_transfer() {
                                let _ = dt.set_data("text/plain", &i.to_string());
                            }
                        }
                        on:dragover=move |ev| {
                            ev.prevent_default();
                            over.set(Some(i));
                        }
                        on:dragleave=move |_| over.set(None)
                        on:mousedown=no_select
                        on:dblclick=move |ev| {
                            if on_row(&ev) {
                                p.send(Command::PlayIndex(i));
                            }
                        }
                        on:drop=move |ev| {
                            ev.prevent_default();
                            over.set(None);
                            if let Some(from) = drag.get_value() && from != i {
                                p.send(Command::Move { from, to: i });
                            }
                            drag.set_value(None);
                        }
                    >
                        <td class="num muted">{i + 1}</td>
                        <td class="title">
                            <a on:click=move |_| p.send(Command::PlayIndex(i))>{tr.title.clone()}</a>
                            <div class="sub muted">
                                {tr.artist.clone()} " · "
                                <a on:click=move |_| {
                                    if let Some(a) = album.clone() {
                                        app.go(Route::Album(a));
                                    }
                                }>{tr.album.clone()}</a>
                            </div>
                        </td>
                        <td class="actions">
                            <button class="icon-btn" title=t("action.up") disabled=i == 0
                                on:click=move |_| p.send(Command::Move { from: i, to: i.saturating_sub(1) })>{icon(Icon::Up)}</button>
                            <button class="icon-btn" title=t("action.down")
                                on:click=move |_| p.send(Command::Move { from: i, to: i + 1 })>{icon(Icon::Down)}</button>
                            <button class="icon-btn" title=t("action.remove") on:click=move |_| p.send(Command::Remove(i))>{icon(Icon::Remove)}</button>
                        </td>
                        <td class="dur muted">{fmt_time(tr.duration.unwrap_or(0.0))}</td>
                    </tr>
                }
            })
            .collect_view()
    };
    let total = move || {
        p.queue.with(|q| {
            q.iter()
                .map(|e| e.track.duration.unwrap_or(0.0))
                .sum::<f64>()
        }) as u64
    };
    view! {
        <h1>{t("nav.queue")}</h1>
        <p class="muted">
            {move || songs_n(p.queue.with(Vec::len))} " · " {move || fmt_total(total())}
            {move || {
                let (cached, loading) = p.cache.get();
                (cached + loading > 0).then(|| format!(" · {}", tf("queue.cache", &[("n", &cached), ("m", &loading)])))
            }}
        </p>
        <div class="actions-row">
            <button on:click=move |_| p.send(Command::Shuffle)>{icon(Icon::Shuffle)} {t("action.shuffle")}</button>
            <button on:click=save>{icon(Icon::Save)} {t("queue.save")}</button>
            <button on:click=move |_| p.send(Command::Clear)>{icon(Icon::Delete)} {t("queue.clear")}</button>
        </div>
        <Show
            when=move || p.queue.with(|q| !q.is_empty())
            fallback=|| view! { <p class="muted">{t("queue.empty")}</p> }
        >
            <table class="songs"><tbody>{rows}</tbody></table>
        </Show>
    }
}

// --- radio, shares ---------------------------------------------------------------------

fn radio() -> impl IntoView {
    let app = app();
    let data = load(|c| async move { c.get_internet_radio_stations().await });
    let name = RwSignal::new(String::new());
    let url = RwSignal::new(String::new());
    let home = RwSignal::new(String::new());
    let add = move |ev: web_sys::SubmitEvent| {
        ev.prevent_default();
        let (n, u, h) = (
            name.get_untracked(),
            url.get_untracked(),
            home.get_untracked(),
        );
        if n.trim().is_empty() || u.trim().is_empty() {
            return;
        }
        let Some(c) = app.client() else { return };
        app.call(
            async move {
                let h = h.trim();
                c.create_internet_radio_station(n.trim(), u.trim(), (!h.is_empty()).then_some(h))
                    .await
            },
            move |()| {
                name.set(String::new());
                url.set(String::new());
                home.set(String::new());
                app.reload();
            },
        );
    };
    view! {
        <h1>{t("nav.radio")}</h1>
        {move || {
            let Some(list) = data.get() else { return loading() };
            if list.is_empty() {
                return view! { <p class="muted">{t("radio.empty")}</p> }.into_any();
            }
            view! {
                <table class="songs"><tbody>
                    {list.into_iter().map(|s| {
                        let (id, n, u) = (s.id.clone(), s.name.clone(), s.stream_url.clone());
                        let del = s.id.clone();
                        view! {
                            <tr>
                                <td class="title">
                                    <a on:click=move |_| app.player.play_radio(&id, &n, &u)>{s.name.clone()}</a>
                                    <div class="sub muted">{s.stream_url.clone()}</div>
                                </td>
                                <td class="actions">
                                    {s.home_page_url.as_deref().and_then(web_link).map(|h| view! {
                                        <a class="icon-btn" href=h target="_blank" rel="noopener" title=t("radio.home")>{icon(Icon::External)}</a>
                                    })}
                                    <button class="icon-btn" title=t("action.remove") on:click=move |_| {
                                        let Some(c) = app.client() else { return };
                                        let id = del.clone();
                                        app.call(async move { c.delete_internet_radio_station(&id).await }, move |()| app.reload());
                                    }>{icon(Icon::Remove)}</button>
                                </td>
                            </tr>
                        }
                    }).collect_view()}
                </tbody></table>
            }
            .into_any()
        }}
        <h2>{t("radio.add")}</h2>
        <form class="stack" on:submit=add>
            <input placeholder=t("radio.name") prop:value=move || name.get() on:input=move |ev| name.set(event_target_value(&ev))/>
            <input type="url" placeholder=t("radio.stream") prop:value=move || url.get() on:input=move |ev| url.set(event_target_value(&ev))/>
            <input type="url" placeholder=t("radio.home") prop:value=move || home.get() on:input=move |ev| home.set(event_target_value(&ev))/>
            <button type="submit">{t("radio.add")}</button>
        </form>
        <p class="hint muted">{t("radio.hint")}</p>
    }
}

fn shares() -> impl IntoView {
    let app = app();
    let data = load(|c| async move { c.get_shares().await });
    view! {
        <h1>{t("nav.shares")}</h1>
        <p class="hint muted">{t("shares.hint")}</p>
        {move || {
            let Some(list) = data.get() else { return loading() };
            if list.is_empty() {
                return view! { <p class="muted">{t("shares.empty")}</p> }.into_any();
            }
            view! {
                <table class="songs"><tbody>
                    {list.into_iter().map(|s| {
                        let url = s.url.clone();
                        let id = s.id.clone();
                        view! {
                            <tr>
                                <td class="title">
                                    <a href=web_link(&s.url) target="_blank" rel="noopener">
                                        {s.description.clone().filter(|d| !d.is_empty()).unwrap_or_else(|| s.url.clone())}
                                    </a>
                                    <div class="sub muted">
                                        {songs_n(s.entry.len())} " · "
                                        {tf("shares.visits", &[("n", &s.visit_count)])}
                                        {s.expires.clone().map(|e| format!(" · {} {}", t("shares.expires"), e.get(..10).unwrap_or(&e))).unwrap_or_default()}
                                    </div>
                                </td>
                                <td class="actions">
                                    <button class="icon-btn" title=t("action.copy") on:click=move |_| super::widgets::copy_text(app, url.clone())>{icon(Icon::Copy)}</button>
                                    <button class="icon-btn" title=t("action.remove") on:click=move |_| {
                                        let Some(c) = app.client() else { return };
                                        let id = id.clone();
                                        app.call(async move { c.delete_share(&id).await }, move |()| app.reload());
                                    }>{icon(Icon::Remove)}</button>
                                </td>
                            </tr>
                        }
                    }).collect_view()}
                </tbody></table>
            }
            .into_any()
        }}
    }
}

// --- settings --------------------------------------------------------------------------

fn select(
    label: &'static str,
    id: &'static str,
    options: Vec<(String, String)>,
    current: impl Fn(&Settings) -> String + Copy + Send + Sync + 'static,
    set: impl Fn(&mut Settings, String) + Copy + 'static,
    after: impl Fn(App, &Settings) + Copy + 'static,
) -> impl IntoView {
    let app = app();
    view! {
        <label for=id>{t(label)}</label>
        <select id=id on:change=move |ev| {
            let v = event_target_value(&ev);
            app.settings.update(|s| {
                set(s, v);
                s.save();
            });
            app.settings.with_untracked(|s| after(app, s));
        }>
            {options.into_iter().map(|(v, l)| {
                let v2 = v.clone();
                view! { <option value=v selected=move || app.settings.with(|s| current(s) == v2)>{l}</option> }
            }).collect_view()}
        </select>
    }
}

fn opts(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(v, k)| ((*v).to_string(), t(k)))
        .collect()
}

fn settings() -> impl IntoView {
    let app = app();
    let reload_page = |_: App, _: &Settings| {
        if let Some(w) = web_sys::window() {
            let _ = w.location().reload();
        }
    };
    let playback = |app: App, s: &Settings| {
        app.player.settings.set(playback_settings(s));
        app.player.set_replay_gain(replay_gain(s));
    };
    let bitrates: Vec<(String, String)> =
        std::iter::once(("0".to_string(), t("settings.original")))
            .chain([96, 128, 160, 192, 256, 320].map(|b| (b.to_string(), format!("{b} kbit/s"))))
            .collect();
    let profiles = move || {
        let current = app.profiles.with(|p| p.current().map(Profile::key));
        app.profiles.get().list.into_iter().map(|p| {
            let key = p.key();
            let is_current = current.as_deref() == Some(key.as_str());
            let p2 = p.clone();
            view! {
                <li class:current=is_current>
                    <button class="link" disabled=is_current on:click=move |_| {
                        app.player.send(Command::Clear);
                        app.login(p2.clone());
                    }>{format!("{} · {}", p.name, p.username)}</button>
                    <span class="muted sub">{p.url.clone()}</span>
                    <button class="icon-btn" title=t("action.remove") disabled=is_current on:click=move |_| {
                        app.profiles.update(|ps| {
                            ps.remove(&key);
                            ps.save();
                        });
                    }>{icon(Icon::Remove)}</button>
                </li>
            }
        }).collect_view()
    };
    let logout = move |_| {
        app.profiles.update(|ps| {
            if let Some(k) = ps.current().map(Profile::key) {
                ps.remove(&k);
            }
            ps.save();
        });
        app.logout();
    };
    let server = move || {
        app.info.get().map(|i| {
            format!(
                "{} {} · API {}{}",
                i.server_type.unwrap_or_else(|| "Subsonic".into()),
                i.server_version.unwrap_or_default(),
                i.version,
                if i.open_subsonic {
                    " · OpenSubsonic"
                } else {
                    ""
                }
            )
        })
    };
    let backend_now = move || match app.player.active.get() {
        Some(super::player::BackendKind::Engine) => t("settings.backend_engine"),
        Some(super::player::BackendKind::Element) => t("settings.backend_element"),
        None => "–".into(),
    };
    view! {
        <h1>{t("nav.settings")}</h1>
        <section class="settings">
            <h2>{t("settings.servers")}</h2>
            <p class="muted">{server}</p>
            <ul class="pick">{profiles}</ul>
            <div class="row">
                <button on:click=move |_| app.go(Route::Login)>{t("settings.add_server")}</button>
                <button class="secondary" on:click=logout>{t("settings.logout")}</button>
            </div>

            <h2>{t("settings.look")}</h2>
            {select("settings.theme", "theme", opts(&[("auto", "settings.auto"), ("light", "settings.light"), ("dark", "settings.dark")]),
                |s| s.theme.clone(), |s, v| s.theme = v, |_, s| apply_theme(&s.theme))}
            {select("settings.language", "language", vec![
                    ("auto".into(), t("settings.auto")), ("en".into(), "English".into()), ("de".into(), "Deutsch".into())],
                |s| s.language.clone(), |s, v| s.language = v, reload_page)}

            <h2>{t("settings.playback")}</h2>
            {select("settings.backend", "backend", opts(&[("auto", "settings.auto"), ("engine", "settings.backend_engine"), ("element", "settings.backend_element")]),
                |s| s.backend.clone(), |s, v| s.backend = v,
                |app, s| app.player.preferred.set(super::backend_kind(s)))}
            <p class="hint muted">{t("settings.backend_hint")} " " {t("settings.backend_now")} " " {backend_now}</p>
            {(!super::player::BackendKind::engine_available())
                .then(|| view! { <p class="hint warn">{t("settings.engine_needs_https")}</p> })}
            {select("settings.bitrate", "bitrate", bitrates,
                |s| s.max_bit_rate.to_string(), |s, v| s.max_bit_rate = v.parse().unwrap_or(0), playback)}
            {select("settings.format", "format", vec![
                    (String::new(), t("settings.original")), ("opus".into(), "Opus".into()),
                    ("mp3".into(), "MP3".into()), ("aac".into(), "AAC".into())],
                |s| s.format.clone(), |s, v| s.format = v, playback)}
            {select("settings.replay_gain", "replay-gain", opts(&[("auto", "settings.auto"), ("track", "settings.rg_track"), ("album", "settings.rg_album"), ("off", "settings.off")]),
                |s| s.replay_gain.clone(), |s, v| s.replay_gain = v, playback)}
            <label class="check">
                <input type="checkbox" prop:checked=move || app.settings.with(|s| s.sync_queue)
                    on:change=move |ev| app.settings.update(|s| { s.sync_queue = event_target_checked(&ev); s.save(); })/>
                {t("settings.sync_queue")}
            </label>
            <label class="check">
                <input type="checkbox" prop:checked=move || app.settings.with(|s| s.scrobble)
                    on:change=move |ev| app.settings.update(|s| { s.scrobble = event_target_checked(&ev); s.save(); })/>
                {t("settings.scrobble")}
            </label>

            <h2>{t("settings.about")}</h2>
            <p class="muted">
                {format!("infrasonic {} · GPL-3.0-only · ", env!("CARGO_PKG_VERSION"))}
                <a href="https://github.com/FionaPreroll/infrasonic" target="_blank" rel="noopener">"GitHub"</a>
                " · " <a on:click=move |_| app.help.set(true)>{t("help.title")}</a>
            </p>
        </section>
    }
}

// --- lyrics ----------------------------------------------------------------------------

/// Lyrics of the playing song: synced (OpenSubsonic) with the current line marked, or text.
fn lyrics() -> impl IntoView {
    let app = app();
    let p = app.player;
    let lines = RwSignal::new(None::<StructuredLyrics>);
    let song = move || {
        p.snapshot
            .with(|s| s.current.as_ref().map(|c| c.track.clone()))
    };
    Effect::new(move |prev: Option<Option<String>>| {
        let track = song();
        let id = track.as_ref().map(|t| t.id.clone());
        if prev.as_ref() == Some(&id) {
            return id;
        }
        lines.set(None);
        if let (Some(tr), Some(c)) = (track, app.client())
            && !tr.id.starts_with("radio:")
        {
            wasm_bindgen_futures::spawn_local(async move {
                let mut found = None;
                if c.supports("songLyrics")
                    && let Ok(list) = c.get_lyrics_by_song_id(&tr.id).await
                {
                    found = list.iter().find(|l| l.synced).or(list.first()).cloned();
                }
                if found.is_none()
                    && let Ok(l) = c.get_lyrics(&tr.artist, &tr.title).await
                    && !l.value.trim().is_empty()
                {
                    found = Some(StructuredLyrics {
                        line: l
                            .value
                            .lines()
                            .map(|v| infrasonic_subsonic::LyricsLine {
                                start: None,
                                value: v.to_string(),
                            })
                            .collect(),
                        ..StructuredLyrics::default()
                    });
                }
                let _ = lines.try_set(Some(found.unwrap_or_default()));
            });
        }
        id
    });
    // Index of the line being sung.
    let current = Memo::new(move |_| {
        let ms = (p.snapshot.with(|s| s.position) * 1000.0) as i64;
        lines.with(|l| {
            let l = l.as_ref()?;
            if !l.synced {
                return None;
            }
            let ms = ms + l.offset.unwrap_or(0);
            l.line
                .iter()
                .rposition(|x| x.start.is_some_and(|s| s <= ms))
        })
    });
    Effect::new(move |_| {
        if current.get().is_some()
            && let Some(el) = web_sys::window()
                .and_then(|w| w.document())
                .and_then(|d| d.query_selector(".lyrics .current").ok().flatten())
        {
            let opts = web_sys::ScrollIntoViewOptions::new();
            opts.set_block(web_sys::ScrollLogicalPosition::Center);
            opts.set_behavior(web_sys::ScrollBehavior::Smooth);
            el.scroll_into_view_with_scroll_into_view_options(&opts);
        }
    });
    view! {
        <h1>{t("nav.lyrics")}</h1>
        {move || song().map(|s| view! { <p class="muted">{format!("{} · {}", s.title, s.artist)}</p> })}
        {move || {
            if song().is_none() {
                return view! { <p class="muted">{t("lyrics.nothing_playing")}</p> }.into_any();
            }
            match lines.get() {
                None => loading(),
                Some(l) if l.line.is_empty() => view! { <p class="muted">{t("lyrics.none")}</p> }.into_any(),
                Some(l) => {
                    let synced = l.synced;
                    view! {
                        <div class="lyrics" class:synced=synced>
                            {l.line.into_iter().enumerate().map(|(i, x)| {
                                let start = x.start;
                                view! {
                                    <p
                                        class:current=move || current.get() == Some(i)
                                        on:click=move |_| {
                                            if let Some(ms) = start && synced {
                                                p.send(Command::Seek(ms as f64 / 1000.0));
                                                if p.snapshot.with_untracked(|s| s.state) == PlayState::Paused {
                                                    p.send(Command::Play);
                                                }
                                            }
                                        }
                                    >
                                        {if x.value.is_empty() { "\u{a0}".to_string() } else { x.value }}
                                    </p>
                                }
                            }).collect_view()}
                        </div>
                    }
                    .into_any()
                }
            }
        }}
    }
}
