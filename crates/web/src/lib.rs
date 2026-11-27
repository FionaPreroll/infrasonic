// SPDX-License-Identifier: GPL-3.0-only
//! The infrasonic web player. Two WebAssembly programs come from this crate: the page
//! (`src/bin/web.rs`, Leptos) and the audio worker (`src/bin/worker.rs`) that runs the engine
//! in pump mode and feeds an AudioWorklet. They talk through [`protocol`].

pub mod i18n;
pub mod protocol;
pub mod sources;
pub mod store;

#[cfg(target_arch = "wasm32")]
pub mod ui;
#[cfg(target_arch = "wasm32")]
pub mod worker;
