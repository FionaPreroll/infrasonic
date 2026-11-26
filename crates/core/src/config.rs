// SPDX-License-Identifier: GPL-3.0-only
//! Configuration file handling, server profiles and the import of STMPS' `stmp.toml`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use infrasonic_subsonic::Auth;

/// Errors while loading or resolving the configuration.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("cannot read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("cannot write {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid configuration in {path}: {message}")]
    Parse { path: PathBuf, message: String },
    #[error("no server profile configured; create {0} or pass a server URL")]
    NoProfile(PathBuf),
    #[error("unknown profile `{0}`")]
    UnknownProfile(String),
    #[error("profile `{0}` has no URL")]
    MissingUrl(String),
    #[error(
        "no password for profile `{0}`: set `password_command`, store it in the keyring or set INFRASONIC_PASSWORD"
    )]
    MissingPassword(String),
    #[error("password command for profile `{profile}` failed: {message}")]
    PasswordCommand { profile: String, message: String },
}

/// How a profile authenticates.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthMethod {
    /// Salted token (default).
    #[default]
    Token,
    /// Legacy hex encoded password (STMPS `plaintext = true`).
    Plain,
    /// OpenSubsonic API key.
    ApiKey,
}

/// One Subsonic server account.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Profile {
    pub url: String,
    pub username: String,
    /// Plain text password. Prefer `password_command` or the system keyring.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    /// Shell command whose first output line is the password, e.g. `pass show music`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password_command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    pub auth: AuthMethod,
}

/// Whether browsing uses the folder structure or ID3 tags.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BrowseMode {
    #[default]
    Folders,
    Tags,
}

/// Which ReplayGain value to apply.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReplayGainSetting {
    #[default]
    Off,
    Track,
    Album,
    /// Album gain while an album plays in order, track gain otherwise.
    Auto,
}

impl From<ReplayGainSetting> for infrasonic_engine::ReplayGainMode {
    fn from(s: ReplayGainSetting) -> Self {
        match s {
            ReplayGainSetting::Off => Self::Off,
            ReplayGainSetting::Track => Self::Track,
            ReplayGainSetting::Album => Self::Album,
            ReplayGainSetting::Auto => Self::Auto,
        }
    }
}

/// Playback settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PlaybackConfig {
    /// 0–100.
    pub volume: u8,
    /// Submit plays to the server (last.fm / ListenBrainz via the server).
    pub scrobble: bool,
    /// Max bit rate in kbit/s, 0 = original quality.
    pub max_bit_rate: u32,
    /// Transcoding format when `max_bit_rate` is set, e.g. `opus` or `mp3`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// Save the queue to the server when quitting.
    pub save_queue_on_exit: bool,
    /// Number of songs added by "random songs".
    pub random_songs: u32,
    /// Seconds for the seek keys.
    pub seek_step: u32,
    /// Output device name (default device if unset).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    /// Loudness normalisation.
    pub replay_gain: ReplayGainSetting,
    /// Added to the ReplayGain value, in dB.
    pub replay_gain_preamp: f32,
    /// Gain for songs without ReplayGain values, in dB.
    pub replay_gain_fallback: f32,
    /// Lower the gain where it would make a song clip.
    pub prevent_clipping: bool,
    /// Size of the song cache in MiB; 0 turns caching off.
    pub cache_mb: u64,
    /// Songs after the next one that are downloaded into the cache in advance.
    pub preload_songs: u32,
}

impl PlaybackConfig {
    /// ReplayGain settings for the engine.
    pub fn replay_gain_config(&self) -> infrasonic_engine::ReplayGainConfig {
        infrasonic_engine::ReplayGainConfig {
            mode: self.replay_gain.into(),
            preamp_db: self.replay_gain_preamp,
            fallback_db: self.replay_gain_fallback,
            prevent_clipping: self.prevent_clipping,
        }
    }
}

impl Default for PlaybackConfig {
    fn default() -> Self {
        Self {
            volume: 80,
            scrobble: true,
            max_bit_rate: 0,
            format: None,
            save_queue_on_exit: true,
            random_songs: 50,
            seek_step: 10,
            device: None,
            replay_gain: ReplayGainSetting::Off,
            replay_gain_preamp: 0.0,
            replay_gain_fallback: 0.0,
            prevent_clipping: true,
            cache_mb: 1024,
            preload_songs: 2,
        }
    }
}

/// User interface settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UiConfig {
    /// Frames of the loading spinner.
    pub spinner: String,
    pub cover_art: bool,
    pub mouse: bool,
    pub browse_mode: BrowseMode,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            spinner: "▉▊▋▌▍▎▏▎▍▌▋▊▉".into(),
            cover_art: true,
            mouse: true,
            browse_mode: BrowseMode::Folders,
        }
    }
}

/// The whole configuration file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_profile: Option<String>,
    pub profiles: BTreeMap<String, Profile>,
    pub playback: PlaybackConfig,
    pub ui: UiConfig,
}

/// Locations of infrasonic's files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub config_file: PathBuf,
    pub cache_dir: PathBuf,
    /// Pinned songs and other data that must not be wiped like a cache.
    pub data_dir: PathBuf,
    pub log_file: PathBuf,
}

impl Paths {
    /// Platform defaults: XDG directories on Linux and macOS, `%APPDATA%` on Windows.
    pub fn platform() -> Self {
        let (config, cache, data, state) = match etcetera::choose_base_strategy() {
            Ok(b) => {
                use etcetera::BaseStrategy;
                let state = b.state_dir().unwrap_or_else(|| b.data_dir());
                (b.config_dir(), b.cache_dir(), b.data_dir(), state)
            }
            Err(_) => {
                let here = PathBuf::from(".");
                (here.clone(), here.clone(), here.clone(), here)
            }
        };
        Self {
            config_file: config.join("infrasonic").join("config.toml"),
            cache_dir: cache.join("infrasonic"),
            data_dir: data.join("infrasonic"),
            log_file: state.join("infrasonic").join("infrasonic.log"),
        }
    }

    /// Where STMPS looked for its configuration.
    pub fn stmps_candidates() -> Vec<PathBuf> {
        let mut v = Vec::new();
        if let Ok(b) = etcetera::choose_base_strategy() {
            use etcetera::BaseStrategy;
            v.push(b.config_dir().join("stmp").join("stmp.toml"));
            v.push(b.home_dir().join(".config").join("stmp").join("stmp.toml"));
        }
        if let Ok(exe) = std::env::current_exe()
            && let Some(dir) = exe.parent()
        {
            v.push(dir.join("stmp.toml"));
        }
        v.dedup();
        v
    }
}

impl Config {
    /// Parse TOML text.
    pub fn parse(text: &str, origin: &Path) -> Result<Self, ConfigError> {
        toml::from_str(text).map_err(|e| ConfigError::Parse {
            path: origin.to_path_buf(),
            message: e.to_string(),
        })
    }

    /// Load from a file. A missing file yields the default configuration.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        match std::fs::read_to_string(path) {
            Ok(t) => Self::parse(&t, path),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(source) => Err(ConfigError::Read {
                path: path.to_path_buf(),
                source,
            }),
        }
    }

    /// Serialize to TOML.
    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).unwrap_or_default()
    }

    /// Write to a file, creating parent directories. On Unix the file is only readable by the
    /// owner because it may contain credentials.
    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        let err = |source| ConfigError::Write {
            path: path.to_path_buf(),
            source,
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(err)?;
        }
        std::fs::write(path, self.to_toml()).map_err(err)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(err)?;
        }
        Ok(())
    }

    /// Convert an STMPS `stmp.toml` into a configuration with one profile named `stmps`.
    pub fn from_stmps(text: &str, origin: &Path) -> Result<Self, ConfigError> {
        #[derive(Deserialize, Default)]
        #[serde(default)]
        struct Stmp {
            auth: StmpAuth,
            server: StmpServer,
            client: StmpClient,
            ui: StmpUi,
        }
        #[derive(Deserialize, Default)]
        #[serde(default)]
        struct StmpAuth {
            username: String,
            password: Option<String>,
            plaintext: bool,
        }
        #[derive(Deserialize, Default)]
        #[serde(default)]
        struct StmpServer {
            host: String,
            scrobble: bool,
        }
        #[derive(Deserialize, Default)]
        #[serde(default)]
        struct StmpClient {
            #[serde(rename = "random-songs")]
            random_songs: Option<u32>,
        }
        #[derive(Deserialize, Default)]
        #[serde(default)]
        struct StmpUi {
            spinner: Option<String>,
        }
        let s: Stmp = toml::from_str(text).map_err(|e| ConfigError::Parse {
            path: origin.to_path_buf(),
            message: e.to_string(),
        })?;
        let mut c = Config::default();
        c.profiles.insert(
            "stmps".into(),
            Profile {
                url: s.server.host,
                username: s.auth.username,
                password: s.auth.password,
                auth: if s.auth.plaintext {
                    AuthMethod::Plain
                } else {
                    AuthMethod::Token
                },
                ..Profile::default()
            },
        );
        c.default_profile = Some("stmps".into());
        c.playback.scrobble = s.server.scrobble;
        if let Some(n) = s.client.random_songs {
            c.playback.random_songs = n;
        }
        if let Some(sp) = s.ui.spinner.filter(|s| !s.is_empty()) {
            c.ui.spinner = sp;
        }
        Ok(c)
    }

    /// Pick a profile: the named one, else `default_profile`, else the only one.
    pub fn profile(&self, name: Option<&str>) -> Result<(String, Profile), ConfigError> {
        let name = match name.or(self.default_profile.as_deref()) {
            Some(n) => n.to_string(),
            None if self.profiles.len() == 1 => {
                self.profiles.keys().next().cloned().unwrap_or_default()
            }
            None => return Err(ConfigError::NoProfile(PathBuf::new())),
        };
        self.profiles
            .get(&name)
            .cloned()
            .map(|p| (name.clone(), p))
            .ok_or(ConfigError::UnknownProfile(name))
    }
}

/// Build a profile from a URL like `https://user:pass@host:port/path` (STMPS CLI syntax).
pub fn profile_from_url(url: &str) -> Result<Profile, ConfigError> {
    let mut u = url::Url::parse(url).map_err(|e| ConfigError::Parse {
        path: PathBuf::from("<command line>"),
        message: format!("invalid server URL `{url}`: {e}"),
    })?;
    let username = percent_decode(u.username());
    let password = u.password().map(percent_decode);
    let _ = u.set_username("");
    let _ = u.set_password(None);
    Ok(Profile {
        url: u.to_string().trim_end_matches('/').to_string(),
        username,
        password,
        ..Profile::default()
    })
}

fn percent_decode(s: &str) -> String {
    url::form_urlencoded::parse(format!("x={s}").as_bytes())
        .next()
        .map(|(_, v)| v.into_owned())
        .unwrap_or_else(|| s.to_string())
}

/// Keyring service name used for passwords.
pub const KEYRING_SERVICE: &str = "infrasonic";

/// Keyring user name for a profile: `username@url`.
pub fn keyring_user(profile: &Profile) -> String {
    format!("{}@{}", profile.username, profile.url)
}

/// Store a password in the system keyring.
#[cfg(feature = "keyring")]
pub fn store_password(profile: &Profile, password: &str) -> Result<(), String> {
    keyring::Entry::new(KEYRING_SERVICE, &keyring_user(profile))
        .and_then(|e| e.set_password(password))
        .map_err(|e| e.to_string())
}

fn keyring_password(profile: &Profile) -> Option<String> {
    #[cfg(feature = "keyring")]
    {
        keyring::Entry::new(KEYRING_SERVICE, &keyring_user(profile))
            .and_then(|e| e.get_password())
            .ok()
    }
    #[cfg(not(feature = "keyring"))]
    {
        let _ = profile;
        None
    }
}

fn run_password_command(name: &str, cmd: &str) -> Result<String, ConfigError> {
    let fail = |message: String| ConfigError::PasswordCommand {
        profile: name.to_string(),
        message,
    };
    #[cfg(windows)]
    let out = std::process::Command::new("cmd").args(["/C", cmd]).output();
    #[cfg(not(windows))]
    let out = std::process::Command::new("sh").args(["-c", cmd]).output();
    let out = out.map_err(|e| fail(e.to_string()))?;
    if !out.status.success() {
        return Err(fail(format!(
            "exit status {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(text.lines().next().unwrap_or("").to_string())
}

/// Resolve the credentials of a profile. Order: API key, `password_command`,
/// `INFRASONIC_PASSWORD`, keyring, `password` in the file.
pub fn resolve_auth(name: &str, p: &Profile) -> Result<Auth, ConfigError> {
    if p.url.trim().is_empty() {
        return Err(ConfigError::MissingUrl(name.into()));
    }
    if p.auth == AuthMethod::ApiKey || (p.username.is_empty() && p.api_key.is_some()) {
        let key = p
            .api_key
            .clone()
            .or_else(|| std::env::var("INFRASONIC_API_KEY").ok())
            .ok_or_else(|| ConfigError::MissingPassword(name.into()))?;
        return Ok(Auth::ApiKey { key });
    }
    let password = if let Some(cmd) = &p.password_command {
        run_password_command(name, cmd)?
    } else if let Ok(pw) = std::env::var("INFRASONIC_PASSWORD") {
        pw
    } else if let Some(pw) = keyring_password(p) {
        pw
    } else if let Some(pw) = &p.password {
        pw.clone()
    } else {
        return Err(ConfigError::MissingPassword(name.into()));
    };
    let username = p.username.clone();
    Ok(match p.auth {
        AuthMethod::Plain => Auth::Plain { username, password },
        _ => Auth::Token { username, password },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_roundtrips() {
        let mut c = Config::default();
        c.profiles.insert(
            "home".into(),
            Profile {
                url: "https://music.example.com".into(),
                username: "me".into(),
                password_command: Some("pass show music".into()),
                ..Profile::default()
            },
        );
        let text = c.to_toml();
        let back = Config::parse(&text, Path::new("x")).expect("parse");
        assert_eq!(back, c);
    }

    #[test]
    fn unknown_keys_are_rejected() {
        let e = Config::parse("[playback]\nvolumee = 3\n", Path::new("cfg")).expect_err("typo");
        assert!(e.to_string().contains("volumee"), "{e}");
    }

    #[test]
    fn imports_stmps_config() {
        let text = r#"
[auth]
username = 'admin'
password = 'password'
plaintext = true

[server]
host = 'https://your-subsonic-host.tld'
scrobble = true

[client]
random-songs = 25

[ui]
spinner = '|/-\'
"#;
        let c = Config::from_stmps(text, Path::new("stmp.toml")).expect("import");
        let (name, p) = c.profile(None).expect("profile");
        assert_eq!(name, "stmps");
        assert_eq!(p.url, "https://your-subsonic-host.tld");
        assert_eq!(p.auth, AuthMethod::Plain);
        assert_eq!(p.password.as_deref(), Some("password"));
        assert!(c.playback.scrobble);
        assert_eq!(c.playback.random_songs, 25);
        assert_eq!(c.ui.spinner, "|/-\\");
    }

    #[test]
    fn profile_selection() {
        let mut c = Config::default();
        assert!(matches!(c.profile(None), Err(ConfigError::NoProfile(_))));
        c.profiles.insert("a".into(), Profile::default());
        assert_eq!(c.profile(None).expect("only one").0, "a");
        c.profiles.insert("b".into(), Profile::default());
        assert!(c.profile(None).is_err());
        c.default_profile = Some("b".into());
        assert_eq!(c.profile(None).expect("default").0, "b");
        assert!(matches!(
            c.profile(Some("zzz")),
            Err(ConfigError::UnknownProfile(_))
        ));
    }

    #[test]
    fn url_with_credentials() {
        let p = profile_from_url("https://al%40ice:p%3Ass@host:4533/music").expect("url");
        assert_eq!(p.username, "al@ice");
        assert_eq!(p.password.as_deref(), Some("p:ss"));
        assert_eq!(p.url, "https://host:4533/music");
    }

    #[test]
    fn password_command_and_api_key() {
        let p = Profile {
            url: "http://x".into(),
            username: "u".into(),
            password_command: Some("echo secret; echo ignored".into()),
            ..Profile::default()
        };
        #[cfg(not(windows))]
        assert_eq!(
            resolve_auth("p", &p).expect("auth"),
            Auth::Token {
                username: "u".into(),
                password: "secret".into()
            }
        );
        let k = Profile {
            url: "http://x".into(),
            api_key: Some("k".into()),
            auth: AuthMethod::ApiKey,
            ..Profile::default()
        };
        assert_eq!(
            resolve_auth("k", &k).expect("auth"),
            Auth::ApiKey { key: "k".into() }
        );
        let missing = Profile {
            url: String::new(),
            ..Profile::default()
        };
        assert!(matches!(
            resolve_auth("m", &missing),
            Err(ConfigError::MissingUrl(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn saved_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("sub/config.toml");
        Config::default().save(&path).expect("save");
        let mode = std::fs::metadata(&path).expect("meta").permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(Config::load(&path).expect("load"), Config::default());
    }
}
