// SPDX-License-Identifier: GPL-3.0-only
//! Helpers for tests against real servers.
//!
//! Run `scripts/e2e-servers.sh cargo test -p infrasonic-e2e`; the script starts Navidrome and
//! gonic in Docker and sets `INFRASONIC_E2E_SERVERS=name=url;name=url`. Without that variable
//! every test returns early, so `cargo test --workspace` works without Docker.

use std::sync::Arc;

use infrasonic_engine::output::{NullOutputFactory, OutputSpec};
use infrasonic_subsonic::{Auth, Client};
use parking_lot::Mutex;

/// A server under test.
#[derive(Debug, Clone)]
pub struct Server {
    pub name: String,
    pub url: String,
}

impl Server {
    /// A client logged in as the admin user both test servers start with.
    pub fn client(&self) -> Client {
        Client::new(&self.url, self.auth()).expect("client")
    }

    pub fn auth(&self) -> Auth {
        Auth::Token {
            username: "admin".into(),
            password: "admin".into(),
        }
    }

    pub fn profile(&self) -> infrasonic_core::Profile {
        infrasonic_core::Profile {
            url: self.url.clone(),
            username: "admin".into(),
            password: Some("admin".into()),
            ..Default::default()
        }
    }
}

/// Servers from `INFRASONIC_E2E_SERVERS`. Empty if unset.
pub fn servers() -> Vec<Server> {
    let Ok(v) = std::env::var("INFRASONIC_E2E_SERVERS") else {
        eprintln!("INFRASONIC_E2E_SERVERS not set; skipping end-to-end test");
        return Vec::new();
    };
    v.split(';')
        .filter_map(|s| s.split_once('='))
        .map(|(n, u)| Server {
            name: n.trim().to_string(),
            url: u.trim().to_string(),
        })
        .collect()
}

/// Output format used by the tests.
pub const SPEC: OutputSpec = OutputSpec {
    sample_rate: 44_100,
    channels: 2,
};

/// A null output running `speed` times faster than real time.
pub fn null_output(speed: f64) -> Box<NullOutputFactory> {
    Box::new(NullOutputFactory::new(SPEC, speed))
}

/// A null output that records the samples.
pub fn capturing_output(speed: f64) -> (Box<NullOutputFactory>, Arc<Mutex<Vec<f32>>>) {
    let cap = Arc::new(Mutex::new(Vec::new()));
    (
        Box::new(NullOutputFactory::new(SPEC, speed).with_capture(cap.clone())),
        cap,
    )
}

/// Names of the synthetic library in `testdata/music`.
pub mod library {
    pub const ARTISTS: [&str; 3] = [
        "Black Noise Collective",
        "Infrasonic Test Ensemble",
        "The Codec Quartet",
    ];
    pub const SONGS: usize = 9;
    pub const FORMATS_ALBUM: &str = "Formats Live";
    pub const SINE_ALBUM: &str = "Sine Studies";
}
