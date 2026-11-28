// SPDX-License-Identifier: GPL-3.0-only
//! Music visualizer: MilkDrop presets rendered by the [Lactea](https://github.com/FionaPreroll/lactea) engine from
//! the audio infrasonic plays.
//!
//! The samples come from the playback engine's [`Scope`] (no microphone, no screen capture), so the picture follows
//! exactly what is heard. A [`Visualizer`] renders on a thread of its own into a window the host hands over with
//! [`Visualizer::attach`] (an Android `Surface`), and stops when it is detached. Presets are read from collection
//! directories ([`Visualizer::set_collections`]): the starter presets the app ships and an optional downloaded
//! bundle.
//!
//! Presets are code. The Lactea fork bounds what one can do (nesting depth, expression loops and memory, shader
//! loops, image decodes); [`Limits::phone`] tightens those bounds for phones, and a preset that fails to load or
//! exceeds them is skipped.

mod playlist;
mod render;

use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use infrasonic_engine::scope::Scope;
use parking_lot::Mutex;
pub use playlist::Collections;
use wgpu::rwh::{HasDisplayHandle, HasWindowHandle};

/// How the visualizer renders.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// Frames per second; the display's own rate caps it.
    pub fps: u32,
    /// Seconds a preset stays before the next one blends in; 0 = until changed by hand.
    pub preset_seconds: f32,
    /// Seconds the blend into the next preset takes; 0 cuts.
    pub transition_seconds: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            fps: 30,
            preset_seconds: 30.0,
            transition_seconds: 2.5,
        }
    }
}

/// Bounds on what a preset may use; see the Lactea fork's `docs/security.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// `loop()` / `while()` iterations per frame.
    pub loop_budget: u64,
    /// Expression memory per preset, in bytes.
    pub memory_budget: usize,
    /// Loop iterations per shader invocation (one pixel).
    pub shader_loop_budget: u32,
    /// Bytes one texture decode may allocate.
    pub texture_bytes: u64,
}

impl Limits {
    /// For phones: about 70 times what the busiest preset of the Cream of the Crop collection runs in a frame, four
    /// times the most expression memory one holds, and textures up to 4096 x 4096 (the collection's are far
    /// smaller). The preset bundles are curated against the same loop and memory limits.
    pub const fn phone() -> Self {
        Self {
            loop_budget: 1 << 20,
            memory_budget: 32 << 20,
            shader_loop_budget: 4096,
            texture_bytes: 64 << 20,
        }
    }
}

impl Default for Limits {
    fn default() -> Self {
        Self::phone()
    }
}

/// What the render thread and the host share.
#[derive(Debug, Default)]
pub(crate) struct Shared {
    pub(crate) collections: Collections,
    pub(crate) settings: Settings,
    pub(crate) limits: Limits,
    /// Bumped on every change of the fields above; the render thread picks changes up between frames.
    pub(crate) generation: u64,
    /// Name of the preset on screen.
    pub(crate) current: Option<String>,
    /// Presets the collections hold.
    pub(crate) preset_count: usize,
    /// Why the last attach could not render, if it could not.
    pub(crate) error: Option<String>,
    /// Bumped on every attach and detach: only the render thread of the current attach reports here, not one that
    /// outlived its detach (see [`DETACH_WAIT`]).
    pub(crate) session: u64,
}

/// Requests to the render thread.
#[derive(Debug)]
pub(crate) enum Request {
    Resize(u32, u32),
    Next,
    Previous,
    Stop,
}

/// The visualizer of one playback engine. Cheap to share; methods return at once.
#[derive(Debug)]
pub struct Visualizer {
    scope: Arc<Scope>,
    shared: Arc<Mutex<Shared>>,
    running: Mutex<Option<Running>>,
}

#[derive(Debug)]
struct Running {
    requests: Sender<Request>,
    thread: JoinHandle<()>,
    /// Disconnects when the render thread ends (its sender is dropped, on a panic too).
    done: Receiver<()>,
}

/// Stack of the render thread. Lactea's limits keep preset loading and evaluation within 1 MiB; the rest is margin
/// for the GPU driver.
const RENDER_STACK: usize = 8 << 20;

/// How long [`Visualizer::detach`] waits for the render thread. Android calls it on the main thread right before the
/// Surface goes away, and a thread blocked in the driver (waiting for a buffer of that very Surface) only comes free
/// once it has gone: waiting longer would freeze the app. A thread that is still running afterwards keeps its own
/// reference to the window, finds the Surface abandoned (an error, not undefined behaviour) and stops at its next
/// frame.
const DETACH_WAIT: Duration = Duration::from_secs(1);

impl Visualizer {
    /// A visualizer fed from `scope` (see [`infrasonic_engine::scope::ScopedOutputFactory`]).
    pub fn new(scope: Arc<Scope>) -> Self {
        Self {
            scope,
            shared: Arc::default(),
            running: Mutex::new(None),
        }
    }

    /// Read presets (and their textures) from these collections. Takes effect with the next preset.
    pub fn set_collections(&self, collections: Collections) {
        let mut s = self.shared.lock();
        s.preset_count = collections.presets().len();
        s.collections = collections;
        s.generation += 1;
    }

    pub fn set_settings(&self, settings: Settings) {
        let mut s = self.shared.lock();
        s.settings = settings;
        s.generation += 1;
    }

    pub fn set_limits(&self, limits: Limits) {
        let mut s = self.shared.lock();
        s.limits = limits;
        s.generation += 1;
    }

    /// Start rendering into `window`, `width` x `height` pixels; a window attached before is detached first.
    pub fn attach<W>(&self, window: W, width: u32, height: u32)
    where
        W: HasWindowHandle + HasDisplayHandle + std::fmt::Debug + Send + Sync + 'static,
    {
        self.detach();
        let (tx, rx) = crossbeam_channel::unbounded();
        let (done_tx, done) = crossbeam_channel::bounded::<()>(0);
        let shared = Arc::clone(&self.shared);
        let scope = Arc::clone(&self.scope);
        let session = {
            let mut s = shared.lock();
            s.session += 1;
            s.session
        };
        let spawned = std::thread::Builder::new()
            .name("infrasonic-visualizer".into())
            .stack_size(RENDER_STACK)
            .spawn(move || {
                let _done = done_tx;
                let report = Arc::clone(&shared);
                // wgpu panics on some driver and handle failures; that must end in an error the app shows, not in a
                // thread that is silently gone.
                let run = AssertUnwindSafe(|| {
                    render::run(window, width, height, scope, shared, session, rx)
                });
                if let Err(panic) = std::panic::catch_unwind(run) {
                    let why = panic
                        .downcast_ref::<&str>()
                        .map(|s| (*s).to_owned())
                        .or_else(|| panic.downcast_ref::<String>().cloned())
                        .unwrap_or_default();
                    let mut s = report.lock();
                    if s.session == session {
                        s.current = None;
                        s.error = Some(format!("the renderer failed: {why}"));
                    }
                }
            });
        match spawned {
            Ok(thread) => {
                *self.running.lock() = Some(Running {
                    requests: tx,
                    thread,
                    done,
                })
            }
            Err(e) => {
                self.shared.lock().error = Some(format!("cannot start the render thread: {e}"))
            }
        }
    }

    /// The window changed its size.
    pub fn resize(&self, width: u32, height: u32) {
        self.request(Request::Resize(width, height));
    }

    /// Stop rendering and let go of the window. Waits for the render thread to stop, at most [`DETACH_WAIT`].
    pub fn detach(&self) {
        let running = self.running.lock().take();
        let Some(r) = running else { return };
        {
            // The thread no longer reports, even if it ends only after the wait below.
            let mut s = self.shared.lock();
            s.session += 1;
            s.current = None;
        }
        let _ = r.requests.send(Request::Stop);
        match r.done.recv_timeout(DETACH_WAIT) {
            Err(RecvTimeoutError::Disconnected) | Ok(()) => {
                if r.thread.join().is_err() {
                    tracing::error!("visualizer thread panicked");
                }
            }
            // Left to stop on its own: it holds its own window reference and no longer reports.
            Err(RecvTimeoutError::Timeout) => {
                tracing::warn!(
                    "visualizer thread still busy after {DETACH_WAIT:?}; it stops on its own"
                )
            }
        }
    }

    /// Blend to the next preset (a random one, unless it was gone back to before).
    pub fn next(&self) {
        self.request(Request::Next);
    }

    /// Back to the preset shown before.
    pub fn previous(&self) {
        self.request(Request::Previous);
    }

    /// Name of the preset on screen (its file name without extension).
    pub fn current_preset(&self) -> Option<String> {
        self.shared.lock().current.clone()
    }

    /// Presets in the collections.
    pub fn preset_count(&self) -> usize {
        self.shared.lock().preset_count
    }

    /// Why rendering stopped or could not start, if it did.
    pub fn error(&self) -> Option<String> {
        self.shared.lock().error.clone()
    }

    fn request(&self, r: Request) {
        if let Some(running) = self.running.lock().as_ref() {
            let _ = running.requests.send(r);
        }
    }
}

impl Drop for Visualizer {
    fn drop(&mut self) {
        self.detach();
    }
}

/// A collection directory: `presets/` with `.milk` files (in subfolders too) and an optional `textures/`.
pub fn collection_dirs(root: impl Into<PathBuf>) -> (PathBuf, PathBuf) {
    let root = root.into();
    (root.join("presets"), root.join("textures"))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Instant;

    use wgpu::rwh::{DisplayHandle, HandleError, WindowHandle};

    use super::*;

    /// A window whose first handle request blocks until the test lets it go, like a driver waiting for a buffer,
    /// and then fails. `gone` tells when the render thread has dropped it, that is, ended.
    #[derive(Debug)]
    struct Stuck {
        release: Receiver<()>,
        blocked: AtomicBool,
        gone: Arc<AtomicBool>,
    }

    impl Stuck {
        /// The window, the sender that lets it go (dropping it does) and its `gone` flag.
        fn new() -> (Self, Sender<()>, Arc<AtomicBool>) {
            let (tx, release) = crossbeam_channel::bounded(0);
            let gone = Arc::new(AtomicBool::new(false));
            let window = Self {
                release,
                blocked: AtomicBool::new(false),
                gone: Arc::clone(&gone),
            };
            (window, tx, gone)
        }

        fn block(&self) -> HandleError {
            if !self.blocked.swap(true, Ordering::AcqRel) {
                let _ = self.release.recv();
            }
            HandleError::Unavailable
        }
    }

    impl Drop for Stuck {
        fn drop(&mut self) {
            self.gone.store(true, Ordering::Release);
        }
    }

    impl HasWindowHandle for Stuck {
        fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
            Err(self.block())
        }
    }

    impl HasDisplayHandle for Stuck {
        fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
            Err(self.block())
        }
    }

    fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !done() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn detach_does_not_wait_for_a_stuck_render_thread() {
        let v = Visualizer::new(Scope::new());
        let (window, release, gone) = Stuck::new();
        v.attach(window, 64, 64);
        v.detach();
        assert!(
            !gone.load(Ordering::Acquire),
            "detach returned only after the render thread ended"
        );
        // Once free, the thread fails, after its detach: that is no error of the visualizer.
        drop(release);
        wait_until("the render thread to end", || gone.load(Ordering::Acquire));
        assert_eq!(v.error(), None);
        // A window that fails at once is reported (as a failed surface, or as wgpu's panic about the missing display
        // handle, depending on the platform).
        let (window, release, _) = Stuck::new();
        drop(release);
        v.attach(window, 64, 64);
        wait_until("the error", || v.error().is_some());
        v.detach();
    }
}
