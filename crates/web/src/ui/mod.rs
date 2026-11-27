// SPDX-License-Identifier: GPL-3.0-only
//! The page: Leptos components, client-side routing on the URL hash (works on GitHub Pages
//! without server rewrites), the player bar, keyboard shortcuts, Media Session, scrobbling
//! and the server queue sync.

mod icons;
mod pages;
pub mod player;
mod widgets;

use infrasonic_engine::{Command, PlayState, ReplayGainConfig, ReplayGainMode, Snapshot};
use infrasonic_subsonic::{Client, ServerInfo, StarTarget};
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

use crate::i18n::{t, tf};
use crate::store::{Profile, Profiles, Settings};
use player::{BackendKind, PlaybackSettings, Player};

/// Where the page is (`#/album/123` and so on).
#[derive(Debug, Clone, PartialEq)]
pub enum Route {
    Login,
    Albums(String),
    Genre(String),
    Years(i32, i32),
    Album(String),
    Artists,
    Artist(String),
    Playlists,
    Playlist(String),
    Search(String),
    Queue,
    Favorites,
    Radio,
    Shares,
    Settings,
    Lyrics,
}

fn decode(s: &str) -> String {
    js_sys::decode_uri_component(s)
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_else(|| s.to_string())
}

fn encode(s: &str) -> String {
    js_sys::encode_uri_component(s).into()
}

impl Route {
    pub fn parse(hash: &str) -> Self {
        let h = hash.trim_start_matches('#').trim_start_matches('/');
        let (head, rest) = h.split_once('/').unwrap_or((h, ""));
        let rest = decode(rest);
        match head {
            "albums" if !rest.is_empty() => Self::Albums(rest),
            "genre" => Self::Genre(rest),
            "years" => {
                let (a, b) = rest.split_once('-').unwrap_or((&rest, &rest));
                match (a.parse(), b.parse()) {
                    (Ok(a), Ok(b)) => Self::Years(a, b),
                    _ => Self::Albums("newest".into()),
                }
            }
            "album" => Self::Album(rest),
            "artists" => Self::Artists,
            "artist" => Self::Artist(rest),
            "playlists" => Self::Playlists,
            "playlist" => Self::Playlist(rest),
            "search" => Self::Search(rest),
            "queue" => Self::Queue,
            "favorites" => Self::Favorites,
            "radio" => Self::Radio,
            "shares" => Self::Shares,
            "settings" => Self::Settings,
            "lyrics" => Self::Lyrics,
            "login" => Self::Login,
            _ => Self::Albums("newest".into()),
        }
    }

    pub fn hash(&self) -> String {
        match self {
            Self::Login => "#/login".into(),
            Self::Albums(l) => format!("#/albums/{}", encode(l)),
            Self::Genre(g) => format!("#/genre/{}", encode(g)),
            Self::Years(a, b) => format!("#/years/{a}-{b}"),
            Self::Album(id) => format!("#/album/{}", encode(id)),
            Self::Artists => "#/artists".into(),
            Self::Artist(id) => format!("#/artist/{}", encode(id)),
            Self::Playlists => "#/playlists".into(),
            Self::Playlist(id) => format!("#/playlist/{}", encode(id)),
            Self::Search(q) => format!("#/search/{}", encode(q)),
            Self::Queue => "#/queue".into(),
            Self::Favorites => "#/favorites".into(),
            Self::Radio => "#/radio".into(),
            Self::Shares => "#/shares".into(),
            Self::Settings => "#/settings".into(),
            Self::Lyrics => "#/lyrics".into(),
        }
    }
}

/// Everything the components share; cheap to copy.
#[derive(Debug, Clone, Copy)]
pub struct App {
    pub client: RwSignal<Option<Client>, LocalStorage>,
    pub info: RwSignal<Option<ServerInfo>>,
    pub profiles: RwSignal<Profiles>,
    pub settings: RwSignal<Settings>,
    pub route: RwSignal<Route>,
    pub player: Player,
    /// Status line: text and whether it is an error.
    pub toast: RwSignal<Option<(String, bool)>>,
    /// Songs waiting for the "add to playlist" dialog.
    pub picker: RwSignal<Option<Vec<String>>>,
    pub help: RwSignal<bool>,
    /// Bumped after changes on the server so that lists load again.
    pub version: RwSignal<u32>,
}

pub fn app() -> App {
    expect_context::<App>()
}

impl App {
    /// The client of the current server. Pages render only while logged in.
    pub fn client(&self) -> Option<Client> {
        self.client.get_untracked()
    }

    pub fn go(&self, r: Route) {
        if let Some(w) = web_sys::window() {
            let _ = w.location().set_hash(&r.hash());
        }
        self.route.set(r);
    }

    pub fn info(&self, text: impl Into<String>) {
        self.toast.set(Some((text.into(), false)));
    }

    pub fn error(&self, text: impl Into<String>) {
        self.toast.set(Some((text.into(), true)));
    }

    pub fn reload(&self) {
        self.version.update(|v| *v += 1);
    }

    /// Run a server call; report errors, then `ok` with the result.
    pub fn call<T: 'static, F>(&self, fut: F, ok: impl FnOnce(T) + 'static)
    where
        F: std::future::Future<Output = infrasonic_subsonic::Result<T>> + 'static,
    {
        let me = *self;
        wasm_bindgen_futures::spawn_local(async move {
            match fut.await {
                Ok(v) => ok(v),
                Err(e) => me.error(e.to_string()),
            }
        });
    }

    pub fn star(&self, target: StarTarget, star: bool, done: impl FnOnce() + 'static) {
        let Some(c) = self.client() else {
            return;
        };
        let me = *self;
        self.call(
            async move {
                if star {
                    c.star(&target).await
                } else {
                    c.unstar(&target).await
                }
            },
            move |()| {
                me.info(if star {
                    t("status.starred")
                } else {
                    t("status.unstarred")
                });
                done();
            },
        );
    }

    /// Connect with `p`; on success it becomes the current profile.
    pub fn login(&self, p: Profile) {
        let client = match Client::builder(&p.url, p.auth())
            .client_name("infrasonic-web")
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                self.error(e.to_string());
                return;
            }
        };
        let me = *self;
        wasm_bindgen_futures::spawn_local(async move {
            match client.connect().await {
                Ok(info) => {
                    me.profiles.update(|ps| {
                        ps.activate(p);
                        ps.save();
                    });
                    me.player.set_client(Some(client.clone()));
                    me.client.set(Some(client.clone()));
                    me.info.set(Some(info));
                    me.toast.set(None);
                    if me.route.get_untracked() == Route::Login {
                        me.go(Route::Albums("newest".into()));
                    }
                    if me.settings.get_untracked().sync_queue {
                        restore_queue(me, client).await;
                    }
                }
                Err(e) => {
                    me.error(tf("error.connect", &[("e", &e)]));
                    if me.route.get_untracked() != Route::Login {
                        me.go(Route::Login);
                    }
                }
            }
        });
    }

    pub fn logout(&self) {
        self.player.send(Command::Clear);
        self.client.set(None);
        self.player.set_client(None);
        self.info.set(None);
        self.go(Route::Login);
    }
}

/// Load the queue saved on the server, without starting playback.
async fn restore_queue(app: App, client: Client) {
    if !app.player.queue.with_untracked(|q| q.is_empty()) {
        return;
    }
    if let Ok(Some(pq)) = client.get_play_queue().await
        && !pq.entry.is_empty()
    {
        let start = pq
            .current
            .as_ref()
            .and_then(|c| pq.entry.iter().position(|e| &e.id == c))
            .unwrap_or(0);
        let pos = pq.position.unwrap_or(0) as f64 / 1000.0;
        app.player.play_tracks(&pq.entry, start, pos, false);
        app.info(tf("status.restored", &[("n", &pq.entry.len())]));
    }
}

pub fn replay_gain(s: &Settings) -> ReplayGainConfig {
    ReplayGainConfig {
        mode: match s.replay_gain.as_str() {
            "off" => ReplayGainMode::Off,
            "track" => ReplayGainMode::Track,
            "album" => ReplayGainMode::Album,
            _ => ReplayGainMode::Auto,
        },
        ..ReplayGainConfig::default()
    }
}

pub fn playback_settings(s: &Settings) -> PlaybackSettings {
    PlaybackSettings {
        max_bit_rate: s.max_bit_rate,
        format: (!s.format.is_empty()).then(|| s.format.clone()),
        replay_gain: replay_gain(s),
    }
}

fn backend_kind(s: &Settings) -> BackendKind {
    match s.backend.as_str() {
        "engine" => BackendKind::Engine,
        "element" => BackendKind::Element,
        _ => BackendKind::auto(),
    }
}

fn browser_languages() -> Vec<String> {
    web_sys::window()
        .map(|w| {
            w.navigator()
                .languages()
                .iter()
                .filter_map(|v| v.as_string())
                .collect()
        })
        .unwrap_or_default()
}

/// Apply `theme` (`auto`, `light`, `dark`) to the document.
pub fn apply_theme(theme: &str) {
    if let Some(el) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.document_element())
    {
        let _ = el.set_attribute("data-theme", theme);
    }
}

/// Entry point of the page program.
pub fn start() {
    console_error_panic_hook::set_once();
    let settings = Settings::load();
    crate::i18n::set_language(&settings.language, &browser_languages());
    apply_theme(&settings.theme);
    if let Some(d) = web_sys::window().and_then(|w| w.document())
        && let Some(el) = d.document_element()
    {
        let _ = el.set_attribute("lang", if crate::i18n::is_german() { "de" } else { "en" });
    }
    register_service_worker();
    leptos::mount::mount_to_body(Root);
}

fn register_service_worker() {
    let Some(w) = web_sys::window() else {
        return;
    };
    // Not on localhost during development, where a stale cache gets in the way.
    let host = w.location().hostname().unwrap_or_default();
    if host == "127.0.0.1" || host == "localhost" {
        return;
    }
    // Service workers need a secure context (HTTPS or localhost); over plain HTTP in a home
    // network `navigator.serviceWorker` does not exist and the player runs without one.
    let nav = w.navigator();
    if !w.is_secure_context()
        || !js_sys::Reflect::has(&nav, &"serviceWorker".into()).unwrap_or(false)
    {
        return;
    }
    let _ = nav.service_worker().register("sw.js");
}

#[component]
fn Root() -> impl IntoView {
    let settings = Settings::load();
    let player = Player::new(backend_kind(&settings), playback_settings(&settings));
    player.snapshot.update(|s| s.volume = settings.volume);
    let hash = web_sys::window()
        .and_then(|w| w.location().hash().ok())
        .unwrap_or_default();
    let app = App {
        client: RwSignal::new_local(None),
        info: RwSignal::new(None),
        profiles: RwSignal::new(Profiles::load()),
        settings: RwSignal::new(settings),
        route: RwSignal::new(Route::parse(&hash)),
        player,
        toast: RwSignal::new(None),
        picker: RwSignal::new(None),
        help: RwSignal::new(false),
        version: RwSignal::new(0),
    };
    provide_context(app);

    // Back and forward buttons.
    let on_hash = Closure::<dyn FnMut()>::new(move || {
        let h = web_sys::window()
            .and_then(|w| w.location().hash().ok())
            .unwrap_or_default();
        let r = Route::parse(&h);
        if app.route.get_untracked() != r {
            app.route.set(r);
        }
    });
    if let Some(w) = web_sys::window() {
        w.set_onhashchange(Some(on_hash.as_ref().unchecked_ref()));
    }
    on_hash.forget();

    // Log in with the saved profile, or a server preset by the deployment.
    match app.profiles.get_untracked().current().cloned() {
        Some(p) => app.login(p),
        None => app.route.set(Route::Login),
    }

    keyboard(app);
    media_session(app);
    scrobbling(app);
    queue_sync(app);

    // Messages from the player go to the status line.
    Effect::new(move |_| {
        if let Some(m) = app.player.message.get() {
            app.error(m);
        }
    });
    // Toasts go away after a while.
    Effect::new(move |_| {
        if app.toast.get().is_some() {
            let cb = Closure::once_into_js(move || app.toast.set(None));
            if let Some(w) = web_sys::window() {
                let _ = w.set_timeout_with_callback_and_timeout_and_arguments_0(
                    cb.unchecked_ref(),
                    6000,
                );
            }
        }
    });

    let logged_in = move || app.client.with(|c| c.is_some());
    view! {
        <Show
            when=logged_in
            fallback=move || view! { <pages::Login/> }
        >
            <div class="layout">
                <widgets::Sidebar/>
                <main id="main">
                    {move || pages::page(app.route.get())}
                </main>
                <widgets::PlayerBar/>
            </div>
            <widgets::PlaylistPicker/>
            <widgets::Help/>
        </Show>
        <widgets::Toast/>
    }
}

fn keyboard(app: App) {
    let handler =
        Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(move |e: web_sys::KeyboardEvent| {
            // Not while typing in a field.
            if let Some(t) = e.target()
                && let Ok(el) = t.dyn_into::<web_sys::HtmlElement>()
            {
                let tag = el.tag_name();
                if matches!(tag.as_str(), "INPUT" | "TEXTAREA" | "SELECT") {
                    if e.key() == "Escape" {
                        let _ = el.blur();
                    }
                    return;
                }
            }
            if e.ctrl_key() || e.meta_key() || e.alt_key() {
                return;
            }
            let p = app.player;
            let snap = p.snapshot.get_untracked();
            let handled = match (e.key().as_str(), e.shift_key()) {
                (" ", _) => {
                    p.send(Command::TogglePause);
                    true
                }
                ("ArrowRight", false) => {
                    p.send(Command::SeekBy(10.0));
                    true
                }
                ("ArrowLeft", false) => {
                    p.send(Command::SeekBy(-10.0));
                    true
                }
                ("ArrowRight", true) => {
                    p.send(Command::Next);
                    true
                }
                ("ArrowLeft", true) => {
                    p.send(Command::Previous);
                    true
                }
                ("=" | "+", _) => {
                    p.send(Command::SetVolume(snap.volume.saturating_add(5).min(100)));
                    true
                }
                ("-", _) => {
                    p.send(Command::SetVolume(snap.volume.saturating_sub(5)));
                    true
                }
                ("l" | "L", _) => {
                    widgets::toggle_star_current(app);
                    true
                }
                ("r", _) => {
                    p.send(Command::SetRepeat(snap.repeat.cycle()));
                    true
                }
                ("q", _) => {
                    app.go(Route::Queue);
                    true
                }
                ("y", _) => {
                    app.go(Route::Lyrics);
                    true
                }
                ("/", _) => {
                    widgets::focus_search();
                    true
                }
                ("?", _) => {
                    app.help.update(|h| *h = !*h);
                    true
                }
                ("Escape", _) => {
                    app.help.set(false);
                    app.picker.set(None);
                    true
                }
                _ => false,
            };
            if handled {
                e.prevent_default();
            }
        });
    if let Some(w) = web_sys::window() {
        let _ = w.add_event_listener_with_callback("keydown", handler.as_ref().unchecked_ref());
    }
    handler.forget();
}

/// Lock screen, media keys and the notification show the playing song. Uses the API
/// through reflection, since web-sys only binds it behind an unstable flag.
fn media_session(app: App) {
    use js_sys::{Function, Reflect};
    let Some(ms) = web_sys::window()
        .and_then(|w| Reflect::get(&w.navigator(), &"mediaSession".into()).ok())
        .filter(|v| v.is_object())
    else {
        return;
    };
    let p = app.player;
    type Handler = Box<dyn Fn(JsValue)>;
    let actions: [(&str, Handler); 7] = [
        ("play", Box::new(move |_| p.send(Command::Play))),
        ("pause", Box::new(move |_| p.send(Command::Pause))),
        ("nexttrack", Box::new(move |_| p.send(Command::Next))),
        (
            "previoustrack",
            Box::new(move |_| p.send(Command::Previous)),
        ),
        (
            "seekforward",
            Box::new(move |_| p.send(Command::SeekBy(10.0))),
        ),
        (
            "seekbackward",
            Box::new(move |_| p.send(Command::SeekBy(-10.0))),
        ),
        (
            "seekto",
            Box::new(move |d: JsValue| {
                if let Some(t) = Reflect::get(&d, &"seekTime".into())
                    .ok()
                    .and_then(|v| v.as_f64())
                {
                    p.send(Command::Seek(t));
                }
            }),
        ),
    ];
    if let Ok(set) =
        Reflect::get(&ms, &"setActionHandler".into()).and_then(|f| f.dyn_into::<Function>())
    {
        for (a, f) in actions {
            let cb = Closure::<dyn Fn(JsValue)>::new(f);
            // Browsers throw for actions they do not know.
            let _ = set.call2(&ms, &a.into(), cb.as_ref());
            cb.forget();
        }
    }
    let ms = StoredValue::new_local(ms);
    Effect::new(move |prev: Option<Option<String>>| {
        let snap = p.snapshot.get();
        let id = snap.current.as_ref().map(|c| c.track.id.clone());
        ms.with_value(|ms| {
            if prev.as_ref() != Some(&id) {
                update_metadata(app, ms, &snap);
            }
            let state = match snap.state {
                PlayState::Playing | PlayState::Buffering => "playing",
                PlayState::Paused => "paused",
                PlayState::Stopped => "none",
            };
            let _ = Reflect::set(ms, &"playbackState".into(), &state.into());
        });
        id
    });
}

fn update_metadata(app: App, ms: &JsValue, snap: &Snapshot) {
    use js_sys::{Array, Object, Reflect};
    let Some(cur) = &snap.current else {
        let _ = Reflect::set(ms, &"metadata".into(), &JsValue::NULL);
        set_title(None);
        return;
    };
    let t = &cur.track;
    let init = Object::new();
    let _ = Reflect::set(&init, &"title".into(), &t.title.as_str().into());
    let _ = Reflect::set(&init, &"artist".into(), &t.artist.as_str().into());
    let _ = Reflect::set(&init, &"album".into(), &t.album.as_str().into());
    if let (Some(c), Some(art)) = (app.client(), &t.cover_art) {
        let img = Object::new();
        let _ = Reflect::set(
            &img,
            &"src".into(),
            &c.cover_art_url(art, Some(512)).to_string().into(),
        );
        let _ = Reflect::set(&img, &"sizes".into(), &"512x512".into());
        let _ = Reflect::set(&init, &"artwork".into(), &Array::of1(&img));
    }
    if let Some(ctor) = web_sys::window()
        .and_then(|w| Reflect::get(&w, &"MediaMetadata".into()).ok())
        .and_then(|f| f.dyn_into::<js_sys::Function>().ok())
        && let Ok(md) = Reflect::construct(&ctor, &Array::of1(&init))
    {
        let _ = Reflect::set(ms, &"metadata".into(), &md);
    }
    set_title(Some(&format!("{} · {}", t.title, t.artist)));
}

fn set_title(song: Option<&str>) {
    if let Some(d) = web_sys::window().and_then(|w| w.document()) {
        d.set_title(&match song {
            Some(s) => format!("{s} · infrasonic"),
            None => "infrasonic".into(),
        });
    }
}

/// "Now playing" when a song starts; a scrobble after half of it or four minutes.
fn scrobbling(app: App) {
    let submitted = StoredValue::new(None::<u64>);
    let announced = StoredValue::new(None::<u64>);
    Effect::new(move |_| {
        let snap = app.player.snapshot.get();
        if !app.settings.with_untracked(|s| s.scrobble) {
            return;
        }
        let Some(cur) = snap.current else {
            return;
        };
        if cur.track.id.starts_with("radio:") {
            return;
        }
        let Some(c) = app.client() else {
            return;
        };
        if snap.state == PlayState::Playing && announced.get_value() != Some(cur.uid) {
            announced.set_value(Some(cur.uid));
            let (c, id) = (c.clone(), cur.track.id.clone());
            wasm_bindgen_futures::spawn_local(async move {
                let _ = c.scrobble(&id, false, None).await;
            });
        }
        let d = snap.duration.or(cur.track.duration).unwrap_or(0.0);
        let needed = (d / 2.0).min(240.0);
        if d > 30.0 && snap.position >= needed && submitted.get_value() != Some(cur.uid) {
            submitted.set_value(Some(cur.uid));
            let id = cur.track.id.clone();
            let now = js_sys::Date::now() as u64;
            wasm_bindgen_futures::spawn_local(async move {
                let _ = c.scrobble(&id, true, Some(now)).await;
            });
        }
    });
}

/// Save the queue on the server when it or the song changes (at most every 10 s), and
/// when the page goes away.
fn queue_sync(app: App) {
    let last = StoredValue::new(0.0f64);
    let save = move || {
        if !app.settings.with_untracked(|s| s.sync_queue) {
            return;
        }
        let Some(c) = app.client() else {
            return;
        };
        let ids: Vec<String> = app.player.queue.with_untracked(|q| {
            q.iter()
                .filter(|e| !e.track.id.starts_with("radio:"))
                .map(|e| e.track.id.clone())
                .collect()
        });
        if ids.is_empty() {
            return;
        }
        let snap = app.player.snapshot.get_untracked();
        let current = snap.current.map(|c| c.track.id);
        let pos = (snap.position * 1000.0) as u64;
        wasm_bindgen_futures::spawn_local(async move {
            let _ = c.save_play_queue(&ids, current.as_deref(), Some(pos)).await;
        });
    };
    Effect::new(move |_| {
        let _ = app.player.queue.with(|q| q.len());
        let _ = app
            .player
            .snapshot
            .with(|s| s.current.as_ref().map(|c| c.uid));
        let now = js_sys::Date::now();
        if now - last.get_value() > 10_000.0 {
            last.set_value(now);
            save();
        }
    });
    let on_hide = Closure::<dyn FnMut()>::new(save);
    if let Some(w) = web_sys::window() {
        let _ = w.add_event_listener_with_callback("pagehide", on_hide.as_ref().unchecked_ref());
    }
    on_hide.forget();
}
