// SPDX-License-Identifier: GPL-3.0-only
//! The audio worker program (loaded by `www/worker.js`).

fn main() {
    #[cfg(target_arch = "wasm32")]
    infrasonic_web::worker::start();
}
