// SPDX-License-Identifier: GPL-3.0-only
//! The page program (loaded by `www/index.html`).

fn main() {
    #[cfg(target_arch = "wasm32")]
    infrasonic_web::ui::start();
}
