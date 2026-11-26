# infrasonic

A music player for [Subsonic](http://www.subsonic.org/pages/api.jsp) and
[OpenSubsonic](https://opensubsonic.netlify.app/) servers such as Navidrome and gonic, written in
Rust. The first frontend is a terminal player and the successor of
[STMPS](https://github.com/spezifisch/stmps). A lightweight web player and an Android app will
use the same engine later.

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

## Crates

| Crate | |
|---|---|
| `crates/subsonic` | Async Subsonic/OpenSubsonic client (API 1.16.1 and newer, token, password and API key auth). Also builds for WebAssembly. |
| `crates/engine` | Playback engine: decoding with Symphonia and opus-decoder, rubato resampling, gapless queue, cpal output. Builds for WebAssembly and Android. |
| `crates/core` | Configuration and profiles, HTTP streaming into the engine, scrobbling, queue sync. |
| `crates/tui` | The `infrasonic` terminal player (ratatui). |
| `crates/e2e` | End-to-end and soak tests against real servers. |
| `crates/web-spike` | Proof that the engine's decoders run in the browser (groundwork for the web player). |
| `vendor/opus-decoder` | opus-decoder 0.1.1 with a speed fix and an overflow fix, until they are released upstream. |

## Development

```sh
cargo test --workspace                           # unit and mock-server tests
scripts/e2e-servers.sh cargo test -p infrasonic-e2e -p infrasonic --tests
                                                 # Navidrome and gonic in Docker, incl. headless TUI test
INFRASONIC_SOAK_SECS=600 scripts/e2e-servers.sh cargo test --release -p infrasonic-e2e --test soak -- --nocapture
scripts/fuzz.sh 60                               # all fuzz targets, needs nightly and cargo-fuzz
scripts/web-spike.sh [--serve]                   # engine in headless Chromium, needs wasm-bindgen-cli and Playwright
```

The test music in `testdata/music` is generated (see `testdata/README.md`). CI runs formatting,
clippy, tests on Linux, macOS and Windows, the end-to-end tests, WebAssembly and Android build
checks and `cargo deny`; fuzzing and a 30 minute soak run nightly.

## License

GPL-3.0-only, see [LICENSE](LICENSE). Dependencies are checked for compatible licences with
`cargo deny`.
