# infrasonic

A music player for [Subsonic](http://www.subsonic.org/pages/api.jsp) and
[OpenSubsonic](https://opensubsonic.netlify.app/) servers such as Navidrome and gonic, written in
Rust. It comes as a terminal player, the successor of
[STMPS](https://github.com/spezifisch/stmps), and as a web player that runs the same engine in
the browser, and as an Android app.

infrasonic plays only what your server streams. It does not need mpv, VLC or any other external
player: decoding (MP3, FLAC, Vorbis, Opus, AAC/ALAC, WAV/AIFF), resampling, gapless playback and
output are done in Rust.

## Terminal player

```sh
cargo install --locked --path crates/tui     # or download a release build
infrasonic                                   # first start asks for your server
infrasonic https://user:password@music.example.org   # STMPS-style one-off connection
```

| Page | What it does |
|---|---|
| 1 Browser | Artists on the left, folders or albums and songs on the right. `--tags` browses by ID3 tags instead of folders. |
| 2 Queue | The play queue with details and cover art of the playing song. Reorder, remove, shuffle, save as playlist, load the queue saved on the server. |
| 3 Playlists | Your playlists and their songs: play, queue, create, delete, remove songs. |
| 4 Search | Server search with artists, albums and songs side by side. |
| 5 Log | What happened, including errors from the server. |
| 6 Library | Album lists (recently added, most played, top rated, A–Z, by year…), genres, starred songs, bookmarks and the songs pinned for offline use. |
| 7 Lyrics | Lyrics of the playing song. Synchronised lyrics (OpenSubsonic) follow the music. |

Press `?` for every key. The defaults follow STMPS: `p` or Space play/pause, `<`/`>` previous and
next track, `,`/`.` seek, `-`/`=` volume, `Enter` play, `a` add to queue, `i` play next, `y` star,
`A` add to a playlist, `r` add random songs, `S` similar songs, `/` filter any list, `q` quit.
Lists also take `j`/`k`, `g`/`G`, PgUp/PgDn and the mouse. In the queue, `K`/`J` move a song.
New in infrasonic: `*` rates a song (0–5 stars), `O` keeps songs, albums or playlists for offline
use, `b` bookmarks the position in the playing song, and `F2` switches between server profiles.

`:` opens the command palette. Type to filter every action of the current page, or enter a
command: `volume 40`, `seek 1:30`, `replaygain album`, `bitrate 192 opus`, `theme light`,
`language de`, `profile work`, `years 1990-1999`, `cache` (or `cache clear`).

Without a connection to the server (or with `--offline`), infrasonic starts with the pinned songs
and plays them from the local cache.

Also included: scrobbling (Last.fm rules), gapless playback for every format (also across sample
rate changes), ReplayGain from the server or the file tags, a persistent audio cache with
preloading of the next songs, queue sync with the
server (restored on start, saved on quit), system media keys (MPRIS on Linux, Now Playing on
macOS, SMTC on Windows) and cover art in Kitty, iTerm2 and Sixel terminals (half blocks
elsewhere).

### Configuration

The configuration lives in `~/.config/infrasonic/config.toml` on Linux and macOS and in
`%APPDATA%\infrasonic\config.toml` on Windows. An existing STMPS `stmp.toml` is imported on the
first start, or with `infrasonic --import-stmps [FILE]`.

```toml
default_profile = "home"

[profiles.home]
url = "https://music.example.org"
username = "me"
# Pick one: the system keyring (infrasonic --store-password home), a command, or INFRASONIC_PASSWORD.
password_command = "pass show music.example.org"
# auth = "token" (default), "plain" (legacy servers) or "apikey" with api_key = "…"

[playback]
volume = 80
scrobble = true
max_bit_rate = 0          # kbit/s; 0 streams the original file
save_queue_on_exit = true
random_songs = 50
seek_step = 10
# device = "…"            # see infrasonic --list-devices
replay_gain = "auto"      # off, track, album, or auto (album gain when the queue plays an album)
replay_gain_preamp = 0.0  # dB
replay_gain_fallback = 0.0  # dB for songs without ReplayGain data
prevent_clipping = true
cache_mb = 1024           # audio cache; pinned songs do not count and are never evicted
preload_songs = 2         # songs downloaded ahead of the playing one

[ui]
cover_art = true
mouse = true
browse_mode = "folders"   # or "tags"
language = "auto"         # en, de, or auto (from LANG)
theme = "default"         # default, light, mono, or a file themes/NAME.toml next to this file

[ui.colors]               # single colours on top of the theme: names, "#rrggbb" or 0–255
# accent = "#88c0d0"      # also dim, error, warn, playing, header, selected_fg, selected_bg

[keys]                    # "context.action" = key or list; see the help (?) for the names
# "queue.remove" = ["x", "Del"]
# "global.toggle_pause" = "Ctrl-t"
```

Contexts are `global`, `list`, `browser`, `queue`, `playlists`, `search` and `library`; action
names are the ones the command palette accepts, such as `toggle_pause`, `star`, `pin` or
`page_6`. Keys look like `x`, `X`, `Ctrl-x`, `Alt-x`, `Enter`, `Space`, `Tab`, `Del`, `PgUp` or
`F5`. Conflicting bindings are reported in the log.

Other options: `--profile NAME`, `--list`, `--add-profile`, `--config FILE`, `--log-file FILE`,
`--offline`.
The log goes to `~/.local/state/infrasonic/infrasonic.log`; `INFRASONIC_LOG=debug` makes it
verbose.

Building on Linux needs the ALSA and D-Bus headers (`libasound2-dev libdbus-1-dev pkg-config` on
Debian and Ubuntu).

## Web player

The web player is a static site: the page (Leptos) and the engine run as WebAssembly, the engine
in a Web Worker that feeds an AudioWorklet, so albums play gapless with exact ReplayGain. Where
that does not work well (iPhone and iPad, browsers without AudioWorklet) and for internet radio,
the browser's own audio element plays instead; the setting can be changed by hand.

It covers what you use Navidrome's web UI for as a listener: albums with covers and sorting
(newest, recently played, most played, top rated, favorites, random, A–Z, by genre and years),
artist pages with biography, top songs and similar artists, playlists with drag and drop, the
queue, synced lyrics, favorites and ratings, search, internet radio and shares. Besides that:
the queue is saved on the server and picked up on other devices, several servers can be saved
as profiles, light and dark theme, English and German, Media Session (lock screen, media keys)
and keyboard shortcuts (`?` shows them). It installs as an app (PWA) and works offline as far as
the player itself goes. The first visit transfers about 0.6 MB; the audio worker (0.8 MB) loads
with the first song.

Logins stay in the browser: with an API key (OpenSubsonic) no password is stored at all,
otherwise a salted token. The player only talks to your server, which must allow cross-origin
requests (Navidrome and gonic do). A player served over HTTPS, like the one on GitHub Pages,
can only reach servers over HTTPS: browsers block plain `http://` requests from an HTTPS page,
so a server at `http://192.168.…` in your home network needs a self-hosted player (below) or
HTTPS. Over plain HTTP the browser allows no AudioWorklet, so the player falls back to the
browser's audio element (no gapless playback, no offline start); HTTPS or `localhost` gets the
full player.

**Container image** (amd64 and arm64, static-web-server, runs as a non-root user). `:latest` is
the latest release, `:snapshot` follows `main`.

```sh
docker run -d -p 8080:8080 -e INFRASONIC_SERVER=https://music.example.org \
  ghcr.io/fionapreroll/infrasonic-web
```

Compose examples are in [`crates/web/docker`](crates/web/docker):
[`compose.yml`](crates/web/docker/compose.yml) serves the player on port 8080,
[`compose.tailscale.yml`](crates/web/docker/compose.tailscale.yml) serves it at
`https://infrasonic.<tailnet>.ts.net` through a Tailscale sidecar with a certificate.
Copy [`example.env`](crates/web/docker/example.env) to `.env` next to them for the server
address, the image tag (`snapshot` follows `main`) and the Tailscale settings.

| Variable | |
|---|---|
| `INFRASONIC_SERVER` | Server address filled in on the login page (the field is hidden then). |
| `INFRASONIC_BASE_PATH` | Serve under a path, e.g. `/music` behind a reverse proxy at `https://example.org/music/`. |

**Any web server**: `scripts/web.sh` builds the site into `target/web` (needs the
`wasm32-unknown-unknown` target and `wasm-bindgen-cli` in the version of `Cargo.lock`);
`scripts/web.sh --serve` serves it on http://127.0.0.1:8098. Every push to `main` that changes
the player deploys it to GitHub Pages (https://fionapreroll.github.io/infrasonic/) and publishes
`:snapshot`; tags `web-v*` publish `:<version>` and `:latest`.

## Crates

| Crate | |
|---|---|
| `crates/subsonic` | Async Subsonic/OpenSubsonic client (API 1.16.1 and newer, token, password and API key auth). Also builds for WebAssembly. |
| `crates/engine` | Playback engine: decoding with Symphonia and opus-decoder, rubato resampling, gapless queue, cpal output. Builds for WebAssembly and Android. |
| `crates/core` | Configuration and profiles, HTTP streaming into the engine, audio cache and pinning, scrobbling, queue sync. |
| `crates/tui` | The `infrasonic` terminal player (ratatui). |
| `crates/headless` | `infrasonic-headless`, a player without UI, also for `adb shell` on Android. |
| `crates/ffi` | Kotlin interface of the core for the Android app (UniFFI): sessions, library, playback, downloads, settings and servers. |
| `crates/e2e` | End-to-end and soak tests against real servers. |
| `crates/web` | The web player: page and audio worker (WebAssembly), assets in `www/`, container image in `docker/`. |
| `vendor/opus-decoder` | opus-decoder 0.1.1 with a speed fix and an overflow fix, until they are released upstream. |

## Android app

`android/` holds the app (Kotlin, Jetpack Compose, Media3). The same Rust engine plays on the
device through `crates/ffi` (UniFFI bindings). It has the library, playlists, search, lyrics,
podcasts, bookmarks, shares and the server jukebox; downloads for offline use with separate bit
rates for Wi-Fi, mobile data and downloads; notification, lock screen, Bluetooth resume, Android
Auto with voice search and a home screen widget; several servers, self-signed certificates,
Material You themes and English and German texts. Android 8 or newer.

```sh
android/gradlew -p android :app:assembleDebug -Pinfrasonic.abis=arm64-v8a   # needs the SDK, NDK 29, cargo-ndk
scripts/e2e-servers.sh android/gradlew -p android :app:testDebugUnitTest   # Kotlin bindings against Navidrome and gonic
scripts/android-device-test.sh                 # UI test on a running emulator, with screenshots
```

`scripts/android-sdk.sh` installs the SDK packages, the pinned NDK, the Rust targets and
cargo-ndk. Texts live in `android/tools/strings.py`, which writes both `strings.xml` files.

Releases: a tag `android-vX.Y.Z` (matching `appVersion` in `android/app/build.gradle.kts`;
versionCode is `X*10000 + Y*100 + Z`) builds the signed APK and AAB, rebuilds the APK unsigned
in another directory and checks that both match apart from the signature, then publishes them
with `SHA256SUMS` and build provenance (`gh attestation verify`). With the secret
`PLAY_SERVICE_ACCOUNT_JSON` it also uploads to the Google Play internal track. Store texts are in
`fastlane/metadata/android`, the F-Droid recipe draft in `android/fdroid`.

`infrasonic-headless` still runs the engine without UI from `adb shell`
(`scripts/android-play.sh`), handy for testing audio on a device.

## Development

```sh
cargo test --workspace                           # unit and mock-server tests
scripts/e2e-servers.sh cargo test -p infrasonic-e2e -p infrasonic -p infrasonic-headless --tests
                                                 # Navidrome and gonic in Docker, incl. headless TUI test
INFRASONIC_SOAK_SECS=600 scripts/e2e-servers.sh cargo test --release -p infrasonic-e2e --test soak -- --nocapture
scripts/fuzz.sh 60                               # all fuzz targets, needs nightly and cargo-fuzz
scripts/web.sh && python3 -m http.server 8098 --bind 127.0.0.1 --directory target/web &
scripts/e2e-servers.sh node crates/web/tests/browser.mjs http://127.0.0.1:8098/ chromium [--mobile]
                                                 # web player in a browser (chromium, firefox, webkit), needs Playwright
```

The test music in `testdata/music` is generated (see `testdata/README.md`). CI runs formatting,
clippy, tests on Linux, macOS and Windows, the end-to-end tests, the web player in Chromium, Firefox, WebKit and mobile Chromium, the Android
app (build, lint, Kotlin tests, emulator UI test) and `cargo deny`; fuzzing and a 30 minute soak run nightly.

## License

GPL-3.0-only, see [LICENSE](LICENSE). Dependencies are checked for compatible licences with
`cargo deny`.
