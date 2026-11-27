// SPDX-License-Identifier: GPL-3.0-only

/// Errors returned by the Subsonic client.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Transport level failure (DNS, TLS, connection, HTTP status). The URL is removed,
    /// because it carries the login data.
    #[error("HTTP request failed: {0}")]
    Http(reqwest::Error),
    /// The server answered with `status: failed`.
    #[error("server error {code}: {message}")]
    Api { code: i32, message: String },
    /// The response could not be decoded.
    #[error("unexpected response from {endpoint}: {source}")]
    Decode {
        endpoint: String,
        #[source]
        source: serde_json::Error,
    },
    /// The response was valid JSON but lacked the expected payload.
    #[error("response from {endpoint} has no `{key}` element")]
    MissingPayload { endpoint: String, key: String },
    /// The configured server URL is invalid.
    #[error("invalid server URL: {0}")]
    Url(#[from] url::ParseError),
}

impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        Error::Http(e.without_url())
    }
}

impl Error {
    /// The server does not implement the call (HTTP 501 or Subsonic error 0 with
    /// "not implemented"/"not supported").
    pub fn is_not_supported(&self) -> bool {
        match self {
            Error::Http(e) => e.status().is_some_and(|s| s.as_u16() == 501),
            Error::Api { code: 0, message } => {
                let m = message.to_lowercase();
                m.contains("not implemented") || m.contains("not supported")
            }
            _ => false,
        }
    }

    /// Subsonic error code 40/41/44: wrong credentials or token auth not supported.
    pub fn is_auth_error(&self) -> bool {
        matches!(self, Error::Api { code, .. } if (40..=44).contains(code))
    }

    /// Subsonic error code 70: the requested data was not found.
    pub fn is_not_found(&self) -> bool {
        matches!(self, Error::Api { code: 70, .. })
    }
}

/// Result alias for this crate.
pub type Result<T, E = Error> = std::result::Result<T, E>;
