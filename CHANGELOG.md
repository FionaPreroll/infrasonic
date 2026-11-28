# Changelog

All notable changes to infrasonic are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/). The terminal player (tags `v*`), the
web player (`web-v*`) and the Android app (`android-v*`) are released on their own; each line
says which part it is about. Nothing has been released yet, so everything so far is under
Unreleased. How versions are made is in "Changelog and versions" in the README.

## [Unreleased]

### Added

- Terminal player: the successor of STMPS, with the STMPS keys and pages for browsing by folder
  or tag, the queue, playlists, search and the log; server profiles with the password from the
  system keyring, a command or the environment; import of an existing STMPS configuration;
  system media keys (MPRIS, Now Playing, SMTC) and cover art in Kitty, iTerm2 and Sixel
  terminals.
- Terminal player: a library page with album lists (newest, recently played, most played, top
  rated, A–Z, by year), genres, starred songs, bookmarks and the songs pinned for offline use.
- Terminal player: a lyrics page whose synchronised lyrics follow the music.
- Terminal player: a command palette (`:`) with every action of the page and commands such as
  `volume 40`, `seek 1:30`, `replaygain album` or `theme light`.
- Terminal player: configurable keys, themes and single colours, ratings (`*`), pinning for
  offline use (`O`), bookmarks (`b`), switching profiles (`F2`), and German texts.
- Terminal player: starts offline with the pinned songs when the server cannot be reached, or
  with `--offline`.
- All players: decoding of MP3, FLAC, Vorbis, Opus, AAC/ALAC and WAV/AIFF in Rust, without mpv
  or any other external player.
- All players: gapless playback for every format, also across sample rate changes, and AAC
  encoder delay and padding from MP4 edit lists.
- All players: ReplayGain (off, track, album, auto) from the server or the file tags, with
  preamp, fallback gain and clipping protection.
- All players: a size-limited audio cache that downloads the next songs ahead, songs pinned for
  offline use, scrobbling and syncing the queue with the server.
- All players: a ten band equalizer with a preamp and automatic headroom; changing it while
  playing, pausing, skipping and seeking no longer click.
- All players: crossfade that keeps albums gapless, playback speed from 0.5× to 3× that keeps
  the pitch, pausing after the current song, and resuming long tracks where they were left.
- `infrasonic-headless`: a player without user interface, also for `adb shell` on Android.
- Web player: a static site with the engine in WebAssembly, feeding an AudioWorklet for gapless
  playback, and the browser's audio element on iPhone and iPad and for internet radio.
- Web player: albums with sorting, genres and years, artist pages with biography, top songs and
  similar artists, playlists with drag and drop, the queue, synced lyrics, favorites, ratings,
  search, internet radio and shares; the queue is saved on the server, several servers as
  profiles, light and dark theme, English and German, Media Session, keyboard shortcuts, and it
  installs as an app (PWA).
- Web player: published on GitHub Pages and as the container image
  `ghcr.io/fionapreroll/infrasonic-web` (`:snapshot` follows `main`), with Compose examples, one
  of them behind Tailscale with HTTPS.
- Web player: works over plain HTTP in a home network, with the audio element instead of the
  AudioWorklet.
- Web player: the mouse wheel over the volume slider changes the volume, and the speaker icon
  mutes; one set of icons that looks the same in every browser.
- Web player: a double click on a song in a list or in the queue plays it right away.
- Android: the app, with the library (album lists, artists, genres, folders, search, favorites,
  ratings, playlists, lyrics, podcasts, bookmarks, shares, the server jukebox and library scan),
  playback through the Rust engine with notification, lock screen, Bluetooth resume, Android
  Auto with voice search and a home screen widget, downloads for offline use with bit rates for
  Wi-Fi, mobile data and downloads, several servers, Material You themes, and English and German.
- Android: the GitHub build looks for new versions of itself if the user agrees, and installs
  them; the latest build is the release `android-latest`. The store build for Google Play and
  F-Droid has no update check.
- Android: an equalizer page with 13 presets, own presets and settings per output device.
- Android: a sleep timer with fade-out, swiping on the mini player, playback speed kept apart
  for music and spoken word, crossfade and resume settings, reordering the queue by dragging,
  start page rows (recently played, often played, newest, random), launcher shortcuts,
  automatic downloads of favorites over Wi-Fi, and synced lyrics from LRC.
- Android: tapping a tab again scrolls to the top; tapping search again focuses the field.
- Android: Settings › Appearance › Page transitions turns the page animation off.
- Android: the equalizer shows its frequency response as a graph with a point per band to drag,
  and three knobs for bass, mids and treble.
- Android: releases are checked to be reproducible and come with checksums and build
  provenance; the signing key is used only in a job of its own.
- Android: pages shown before open instantly from a memory cache and refresh in the background;
  the cover cache has a size limit (default 250 MB), and a new Storage section in the settings
  shows and limits what songs, covers and downloads use.
- Android: every long list gets a fast scroller (library, playlists, search, queue, podcasts,
  bookmarks, shares, jukebox): a thumb on the right edge to drag anywhere in the list, with the
  current letter in a bubble for artists, genres, folders and albums by name or artist, and the
  section for search results.
- Android: a visualizer (the button in the player) shows MilkDrop presets that follow what
  plays, without the microphone permission. 24 presets come with the app; the full Cream of
  the Crop collection (about 9,000 presets, 31 MB) is downloaded only on request in Settings ›
  Visualizer, where a check for newer collections can also be switched on.

### Changed

- All players: the songs after the playing one are downloaded one at a time, once the playing
  song is in, so prefetching never competes with playback.
- All players: playback goes on through changes of the output device and through network drops;
  a broken download resumes where it stopped.
- Android: page transitions take 250 ms instead of 700 ms.
- All players: "Accept self-signed certificate" trusts exactly the certificate the server
  presented first, by its fingerprint, instead of turning off every TLS check; a new
  certificate has to be trusted by hand.
- All players: responses from the server are read only up to a size limit, so a broken or
  hostile server cannot exhaust memory, and errors no longer contain login data.
- Terminal player: the configuration file with credentials is private before anything is
  written to it.
- Web player: links from the server only count as links if they are http or https; a
  Content-Security-Policy allows only the player's own scripts, and the container image forbids
  framing the player.
- Android: the cover provider is no longer open to other apps, and the media session gives
  library, search and what plays only to trusted apps; others may only play, pause and skip.

### Fixed

- All players: a paused player stays paused when the output device goes away, and pause works
  again at once; the Android app no longer hangs on "buffering" when a Bluetooth speaker is
  switched off.
- All players: when the output device goes away during playback, the new one goes on exactly
  where the old one stopped, instead of repeating the last moment.
- Android: playback no longer crashes the app when it starts.
- Android: the library loads after the first setup and at every start, instead of staying on
  "not connected".
- Android: saving the first server no longer leaves the app on "Connecting" for good.
- Android: the version code of the GitHub build counts every commit again, so the update check
  offers new builds.
- Android: covers of different servers with the same id no longer mix up.
