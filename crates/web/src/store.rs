// SPDX-License-Identifier: GPL-3.0-only
//! Server profiles and settings in the browser's `localStorage`. Passwords are never stored:
//! a profile keeps an API key or a salted token.

use infrasonic_subsonic::Auth;
use serde::{Deserialize, Serialize};

#[cfg(target_arch = "wasm32")]
const PROFILES: &str = "infrasonic.profiles";
#[cfg(target_arch = "wasm32")]
const SETTINGS: &str = "infrasonic.settings";

/// A saved server login.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    pub url: String,
    pub username: String,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub salt: Option<String>,
    /// Legacy servers that need the password in each request (hex encoded).
    #[serde(default)]
    pub plain: Option<String>,
}

impl Profile {
    /// A profile from what the login form asks for.
    pub fn new(url: &str, username: &str, password: &str, api_key: &str, legacy: bool) -> Self {
        let url = normalize_url(url);
        let name = url::Url::parse(&url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
            .unwrap_or_else(|| url.clone());
        let mut p = Profile {
            name,
            url,
            username: username.trim().to_string(),
            api_key: None,
            token: None,
            salt: None,
            plain: None,
        };
        if !api_key.trim().is_empty() {
            p.api_key = Some(api_key.trim().to_string());
        } else if legacy {
            p.plain = Some(password.to_string());
        } else if let Auth::SaltedToken { token, salt, .. } = Auth::salted(&p.username, password) {
            p.token = Some(token);
            p.salt = Some(salt);
        }
        p
    }

    pub fn auth(&self) -> Auth {
        if let Some(k) = &self.api_key {
            return Auth::ApiKey { key: k.clone() };
        }
        if let Some(pw) = &self.plain {
            return Auth::Plain {
                username: self.username.clone(),
                password: pw.clone(),
            };
        }
        Auth::SaltedToken {
            username: self.username.clone(),
            token: self.token.clone().unwrap_or_default(),
            salt: self.salt.clone().unwrap_or_default(),
        }
    }

    /// Profiles are told apart by server and user.
    pub fn key(&self) -> String {
        format!("{}|{}", self.url, self.username)
    }
}

/// `music.example.org` → `https://music.example.org`; trailing slashes go.
pub fn normalize_url(url: &str) -> String {
    let u = url.trim().trim_end_matches('/');
    if u.contains("://") {
        u.to_string()
    } else {
        format!("https://{u}")
    }
}

/// All saved profiles; the first is the one in use.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Profiles {
    pub list: Vec<Profile>,
}

impl Profiles {
    /// Put `p` first (replacing an older entry for the same server and user).
    pub fn activate(&mut self, p: Profile) {
        self.list.retain(|x| x.key() != p.key());
        self.list.insert(0, p);
    }

    pub fn remove(&mut self, key: &str) {
        self.list.retain(|x| x.key() != key);
    }

    pub fn current(&self) -> Option<&Profile> {
        self.list.first()
    }
}

/// Settings of the player.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// `auto`, `light` or `dark`.
    pub theme: String,
    /// `auto`, `en` or `de`.
    pub language: String,
    /// `auto`, `engine` or `element`.
    pub backend: String,
    pub max_bit_rate: u32,
    /// Transcoding target, empty for the original files.
    pub format: String,
    /// `off`, `track`, `album` or `auto`.
    pub replay_gain: String,
    pub volume: u8,
    /// Restore the queue saved on the server and save it while playing.
    pub sync_queue: bool,
    pub scrobble: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "auto".into(),
            language: "auto".into(),
            backend: "auto".into(),
            max_bit_rate: 0,
            format: String::new(),
            replay_gain: "auto".into(),
            volume: 80,
            sync_queue: true,
            scrobble: true,
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

#[cfg(target_arch = "wasm32")]
fn load<T: for<'de> Deserialize<'de> + Default>(key: &str) -> T {
    storage()
        .and_then(|s| s.get_item(key).ok().flatten())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

#[cfg(target_arch = "wasm32")]
fn save<T: Serialize>(key: &str, v: &T) {
    if let (Some(s), Ok(t)) = (storage(), serde_json::to_string(v)) {
        let _ = s.set_item(key, &t);
    }
}

#[cfg(target_arch = "wasm32")]
impl Profiles {
    pub fn load() -> Self {
        load(PROFILES)
    }
    pub fn save(&self) {
        save(PROFILES, self);
    }
}

#[cfg(target_arch = "wasm32")]
impl Settings {
    pub fn load() -> Self {
        load(SETTINGS)
    }
    pub fn save(&self) {
        save(SETTINGS, self);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_keep_no_password() {
        let p = Profile::new("music.example.org/", "me", "secret", "", false);
        assert_eq!(p.url, "https://music.example.org");
        assert_eq!(p.name, "music.example.org");
        let json = serde_json::to_string(&p).expect("json");
        assert!(!json.contains("secret"), "{json}");
        match p.auth() {
            Auth::SaltedToken {
                username,
                token,
                salt,
            } => {
                assert_eq!(username, "me");
                assert_eq!(token, infrasonic_subsonic::token("secret", &salt));
            }
            a => panic!("{a:?}"),
        }
        let k = Profile::new("http://h:4533", "me", "", "KEY", false);
        assert_eq!(k.auth(), Auth::ApiKey { key: "KEY".into() });
    }

    #[test]
    fn activating_moves_a_profile_first() {
        let mut ps = Profiles::default();
        let a = Profile::new("a", "u", "p", "", false);
        let b = Profile::new("b", "u", "p", "", false);
        ps.activate(a.clone());
        ps.activate(b.clone());
        ps.activate(a.clone());
        assert_eq!(ps.list.len(), 2);
        assert_eq!(ps.current().map(|p| p.name.as_str()), Some("a"));
        ps.remove(&a.key());
        assert_eq!(ps.current(), Some(&b));
        let s: Settings = serde_json::from_str("{\"theme\":\"dark\"}").expect("settings");
        assert_eq!(s.theme, "dark");
        assert_eq!(s.volume, 80);
    }
}
