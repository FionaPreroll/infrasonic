// SPDX-License-Identifier: GPL-3.0-only
//! The Kotlin interface of infrasonic, generated with [UniFFI](https://mozilla.github.io/uniffi-rs/).
//!
//! The Android app talks to one [`Infrasonic`] object: it connects to a server, browses the
//! library through async methods and controls the Rust playback engine. Playback events reach
//! the app through a [`PlayerListener`].
//!
//! Write the bindings with
//! `cargo run -p infrasonic-ffi --features bindgen --bin uniffi-bindgen -- generate --library <libinfrasonic_ffi.so> --language kotlin --out-dir <dir>`
//! (`android/build-rust.sh` does this and builds the libraries for all Android ABIs).

mod android;
mod app;
mod resume;
mod types;

pub use app::{Infrasonic, PlayerCommand, PlayerListener};
pub use types::*;

uniffi::setup_scaffolding!();

/// Send log messages to logcat (Android) at this level: `error`, `warn`, `info`, `debug`
/// or `trace`. Only the first call has an effect.
#[uniffi::export]
pub fn init_logging(level: String) {
    #[cfg(target_os = "android")]
    {
        use tracing_subscriber::layer::SubscriberExt as _;
        let filter: tracing_subscriber::filter::LevelFilter = level
            .parse()
            .unwrap_or(tracing_subscriber::filter::LevelFilter::INFO);
        if let Ok(layer) = tracing_android::layer("infrasonic") {
            let sub = tracing_subscriber::registry().with(layer).with(filter);
            let _ = tracing::subscriber::set_global_default(sub);
        }
    }
    #[cfg(not(target_os = "android"))]
    let _ = level;
}

/// Version of the Rust library, e.g. `0.1.0`.
#[uniffi::export]
pub fn core_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}
