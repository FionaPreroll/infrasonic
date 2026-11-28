# Notes for Claude

The README says what infrasonic is, how to run each part and how to develop it: the terminal
player (`crates/tui`), the web player (`crates/web`), the Android app (`android/` on top of
`crates/ffi`) and the crates they share (`crates/subsonic`, `crates/engine`, `crates/core`).
`testdata/README.md` says how the test music is made. These rules hold in every session:

## Working with Fiona

- Answer Fiona in German. Code, comments in code, READMEs, the changelog and commit messages
  are in English.
- What Fiona decided stays decided until she decides otherwise.

## Branches

- During the initial development, until Fiona says otherwise, commit straight to `main`, no
  branches and no pull requests. This holds even when the session suggests another branch
  name, such as `claude/…`.
- Afterwards, every feature or fix gets its own branch off `origin/main`, named
  `feature/<name_in_snake_case>` or `fix/<name_in_snake_case>`, and one pull request against
  `main` (see "Branches and pull requests" in the README), also when the session suggests
  another branch name. Then nobody pushes to `main`.
- Several sessions may work at the same time. Before every push, `git fetch origin main` and
  rebase onto it (no merge commits), tell the other sessions that touch the repository, and
  push one after another. Who changes a file another session is working on says so first.
- `main` must stay green. After a push, follow its CI run; a red `main` is fixed before
  anything else. `ci.yml` cancels the run of the previous push, so pushes in quick succession
  only get the last one checked.
- One commit per change that belongs together, with a subject that starts with the part it
  touches (`Engine:`, `TUI:`, `Web player:`, `Android:`, `CI:`, `Tests:`) and a body that says
  what changed for users and why.

## Changelog

- A change that users notice adds itself to `CHANGELOG.md`, under `## [Unreleased]`, in the
  section it belongs to (`### Added`, `### Changed`, `### Fixed` or `### Removed`), in the same
  commit (later: the same pull request). Write it for users, in English, starting with the part
  it is about: `Terminal player:`, `Web player:`, `Android:`, or `All players:` for the engine
  and the shared crates.
- Changes only developers notice (tests, tooling, CI, refactoring) need no entry.
- A new version is made only when asked: see "Changelog and versions" in the README.

## Code

- Before every push, run the checks of the part that changed, as CI runs them (`ci.yml`):
  `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` and
  `cargo test --workspace`. After a change to the client, the core, the TUI or the FFI also the
  end-to-end tests against Navidrome and gonic (`scripts/e2e-servers.sh …`, see the README).
  After a change to the web player `scripts/web.sh` and its browser test. After a change in
  `android/` `android/gradlew -p android :app:assembleGithubDebug :app:lintGithubDebug
  :app:lintStoreDebug :app:testGithubDebugUnitTest`; the emulator test only runs in CI. After
  changing `Cargo.toml` or `Cargo.lock`, also `cargo deny check`.
- No `unsafe`: the workspace forbids it, except `crates/ffi`, whose generated scaffolding needs
  it. Nothing that comes from a server, a file or the network may bring a player down: no
  `unwrap`, `expect` or `panic!` on it outside tests.
- Every parser of outside input (Subsonic responses, audio files, configuration, streams) has
  limits for size and count. The fuzz targets are in `fuzz/` (`scripts/fuzz.sh`) and run
  nightly in `scheduled.yml`; whatever they find becomes a test.
- New code comes with tests, and no test is switched off to get CI green. Tests use the
  generated music in `testdata/` and the Navidrome and gonic containers, never a real server,
  a real account or a secret. Timing in tests waits for a condition instead of sleeping a fixed
  time; CI runners are slow.
- Every text users see is in English and German: `crates/tui/locales/`, `crates/web/locales/`,
  and for the app only `android/tools/strings.py`, which writes both `strings.xml` files.
- Android dependencies are checked against `android/gradle/verification-metadata.xml`. After
  changing one, run `scripts/gradle-verification.sh` and commit the file with the change. The
  signing key never meets Gradle or a build script: only the signing jobs in the environment
  `android-release` see it.
- Actions are pinned to a commit and Docker base images to a digest; Dependabot keeps them
  current.
- `vendor/opus-decoder` carries two fixes until upstream releases them; then it goes.
- Dependencies come from crates.io, with `Cargo.lock` committed. Their licences must fit
  GPL-3.0-only (`deny.toml`). Prefer crates in pure Rust.

## What we don't need

People read this code, so rather 20 lines less than one more case:

- No fallback for errors that do not happen; not panicking on outside input is enough.
- The project's own files (the build, the test music, the locales) are not outside input.
- A message in the log that breaks nothing is no reason for code.
