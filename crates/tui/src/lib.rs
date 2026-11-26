// SPDX-License-Identifier: GPL-3.0-only
//! The infrasonic terminal player. The binary in `main.rs` is a thin wrapper; the pieces live
//! here so that tests can drive the UI headless.

pub mod i18n;

pub mod app;
pub mod cli;
pub mod cover;
pub mod entry;
pub mod keys;
pub mod list;
pub mod logging;
pub mod media;
pub mod pages;
pub mod palette;
pub mod setup;
pub mod theme;
pub mod ui;

pub use app::{App, Msg, Page};
