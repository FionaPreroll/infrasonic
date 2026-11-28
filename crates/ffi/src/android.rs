// SPDX-License-Identifier: GPL-3.0-only
//! Android only: the Java VM and app context for the audio output, and the visualizer's Surface.
//!
//! cpal's AAudio host asks the Java `AudioManager` for devices and buffer sizes through
//! `ndk-context`, which panics if nobody set it. A plain Android app (no `NativeActivity`)
//! has to do that itself, once, before the first playback: `AndroidAudio.init(context)` in
//! Kotlin calls `Java_io_github_fionapreroll_infrasonic_core_AndroidAudio_nativeInit`.
//!
//! The visualizer renders into the `ANativeWindow` of a `SurfaceView`'s Surface, which
//! `VisualizerSurface` in Kotlin hands over with `nativeAttach` and takes back with
//! `nativeDetach` (before the Surface is destroyed). These are the only `unsafe` parts of the
//! visualizer: turning JNI pointers into a window; it renders in safe Rust.

use std::sync::{Arc, Weak};

use infrasonic_visualizer::Visualizer;
use parking_lot::RwLock;

/// The visualizer of the newest `Infrasonic` object, for the JNI entry points (they get no
/// handle; the app has one `Infrasonic` at a time).
static VISUALIZER: RwLock<Option<Weak<Visualizer>>> = RwLock::new(None);

pub(crate) fn set_visualizer(v: &Arc<Visualizer>) {
    *VISUALIZER.write() = Some(Arc::downgrade(v));
}

#[cfg_attr(not(target_os = "android"), allow(dead_code))]
fn visualizer() -> Option<Arc<Visualizer>> {
    VISUALIZER.read().as_ref().and_then(Weak::upgrade)
}

#[cfg(target_os = "android")]
mod jni {
    use std::sync::Once;

    use jni_sys::{JNIEnv, JavaVM, jclass, jint, jobject};
    use ndk::native_window::NativeWindow;
    use wgpu_handles::*;

    static INIT: Once = Once::new();

    /// `external fun nativeInit(context: Context)` in `io.github.fionapreroll.infrasonic.core.AndroidAudio`.
    ///
    /// # Safety
    /// Called by the Java VM with a valid environment and a `Context`.
    #[unsafe(no_mangle)]
    pub unsafe extern "system" fn Java_io_github_fionapreroll_infrasonic_core_AndroidAudio_nativeInit(
        env: *mut JNIEnv,
        _class: jclass,
        context: jobject,
    ) {
        INIT.call_once(|| unsafe {
            let fns = &**env;
            let mut vm: *mut JavaVM = std::ptr::null_mut();
            if (fns.v1_2.GetJavaVM)(env, &mut vm) != 0 || vm.is_null() {
                tracing::warn!("no Java VM; the audio output may fail");
                return;
            }
            // A global reference: the context outlives this call (the application context
            // lives as long as the process).
            let global = (fns.v1_1.NewGlobalRef)(env, context);
            ndk_context::initialize_android_context(vm.cast(), global.cast());
        });
    }

    /// `external fun nativeAttach(surface: Surface, width: Int, height: Int)` in
    /// `io.github.fionapreroll.infrasonic.visualizer.VisualizerSurface`: render into `surface`.
    ///
    /// # Safety
    /// Called by the Java VM with a valid environment and a `android.view.Surface`.
    #[unsafe(no_mangle)]
    pub unsafe extern "system" fn Java_io_github_fionapreroll_infrasonic_visualizer_VisualizerSurface_nativeAttach(
        env: *mut JNIEnv,
        _class: jclass,
        surface: jobject,
        width: jint,
        height: jint,
    ) {
        // SAFETY: `env` and `surface` come from the VM for this call; `from_surface` takes its
        // own reference on the ANativeWindow, which `NativeWindow` releases when dropped.
        let window = unsafe { NativeWindow::from_surface(env.cast(), surface.cast()) };
        let (Some(window), Some(v)) = (window, super::visualizer()) else {
            tracing::warn!("visualizer: no window or no visualizer to attach");
            return;
        };
        let size = |n: jint| u32::try_from(n).unwrap_or(1).max(1);
        v.attach(AndroidWindow(window), size(width), size(height));
    }

    /// `external fun nativeResize(width: Int, height: Int)`.
    ///
    /// # Safety
    /// Called by the Java VM.
    #[unsafe(no_mangle)]
    pub unsafe extern "system" fn Java_io_github_fionapreroll_infrasonic_visualizer_VisualizerSurface_nativeResize(
        _env: *mut JNIEnv,
        _class: jclass,
        width: jint,
        height: jint,
    ) {
        if let Some(v) = super::visualizer() {
            let size = |n: jint| u32::try_from(n).unwrap_or(1).max(1);
            v.resize(size(width), size(height));
        }
    }

    /// `external fun nativeDetach()`: returns once the window is no longer used, so the
    /// Surface may be destroyed after it.
    ///
    /// # Safety
    /// Called by the Java VM.
    #[unsafe(no_mangle)]
    pub unsafe extern "system" fn Java_io_github_fionapreroll_infrasonic_visualizer_VisualizerSurface_nativeDetach(
        _env: *mut JNIEnv,
        _class: jclass,
    ) {
        if let Some(v) = super::visualizer() {
            v.detach();
        }
    }

    mod wgpu_handles {
        use ndk::native_window::NativeWindow;
        use raw_window_handle::{
            DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle,
        };

        /// A Surface's window with the (trivial) Android display handle wgpu wants next to it.
        #[derive(Debug)]
        pub(crate) struct AndroidWindow(pub(crate) NativeWindow);

        impl HasWindowHandle for AndroidWindow {
            fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
                self.0.window_handle()
            }
        }

        impl HasDisplayHandle for AndroidWindow {
            fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
                Ok(DisplayHandle::android())
            }
        }
    }
}
