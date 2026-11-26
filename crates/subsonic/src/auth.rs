// SPDX-License-Identifier: GPL-3.0-only
//! Authentication parameters for Subsonic requests.

use md5::{Digest, Md5};

/// How the client authenticates against the server.
#[derive(Clone, PartialEq, Eq)]
pub enum Auth {
    /// Salted MD5 token authentication (Subsonic 1.13+). The default.
    Token { username: String, password: String },
    /// Legacy authentication: the password is sent hex encoded with every request.
    /// Needed for servers that store hashed passwords (e.g. some LDAP setups).
    Plain { username: String, password: String },
    /// OpenSubsonic `apiKeyAuthentication`.
    ApiKey { key: String },
}

impl std::fmt::Debug for Auth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Auth::Token { username, .. } => f
                .debug_struct("Token")
                .field("username", username)
                .finish_non_exhaustive(),
            Auth::Plain { username, .. } => f
                .debug_struct("Plain")
                .field("username", username)
                .finish_non_exhaustive(),
            Auth::ApiKey { .. } => f.debug_struct("ApiKey").finish_non_exhaustive(),
        }
    }
}

impl Auth {
    /// Username, if this auth method has one.
    pub fn username(&self) -> Option<&str> {
        match self {
            Auth::Token { username, .. } | Auth::Plain { username, .. } => Some(username),
            Auth::ApiKey { .. } => None,
        }
    }

    /// Query parameters that authenticate one request. Token auth uses a fresh salt each time.
    pub(crate) fn params(&self) -> Vec<(&'static str, String)> {
        match self {
            Auth::Token { username, password } => {
                let salt = new_salt();
                vec![
                    ("u", username.clone()),
                    ("t", token(password, &salt)),
                    ("s", salt),
                ]
            }
            Auth::Plain { username, password } => vec![
                ("u", username.clone()),
                ("p", format!("enc:{}", hex::encode(password.as_bytes()))),
            ],
            Auth::ApiKey { key } => vec![("apiKey", key.clone())],
        }
    }
}

/// `md5(password + salt)` as lowercase hex, as defined by the Subsonic API.
pub fn token(password: &str, salt: &str) -> String {
    let mut h = Md5::new();
    h.update(password.as_bytes());
    h.update(salt.as_bytes());
    hex::encode(h.finalize())
}

fn new_salt() -> String {
    let bytes: [u8; 8] = rand::random();
    hex::encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_matches_api_example() {
        // Example from http://www.subsonic.org/pages/api.jsp
        assert_eq!(
            token("sesame", "c19b2d"),
            "26719a1196d2a940705a59634eb18eab"
        );
    }

    #[test]
    fn plain_is_hex_encoded() {
        let a = Auth::Plain {
            username: "u".into(),
            password: "sesame".into(),
        };
        let p = a.params();
        assert_eq!(p[1], ("p", "enc:736573616d65".to_string()));
    }

    #[test]
    fn salts_differ_between_requests() {
        let a = Auth::Token {
            username: "u".into(),
            password: "pw".into(),
        };
        assert_ne!(a.params()[2].1, a.params()[2].1);
    }

    #[test]
    fn debug_does_not_leak_password() {
        let a = Auth::Token {
            username: "u".into(),
            password: "supersecret".into(),
        };
        assert!(!format!("{a:?}").contains("supersecret"));
    }
}
