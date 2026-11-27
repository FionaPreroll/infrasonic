// SPDX-License-Identifier: GPL-3.0-only
//! Android only: hand the Java VM and the app context to the audio output.
//!
//! cpal's AAudio host asks the Java `AudioManager` for devices and buffer sizes through
//! `ndk-context`, which panics if nobody set it. A plain Android app (no `NativeActivity`)
//! has to do that itself, once, before the first playback: `AndroidAudio.init(context)` in
//! Kotlin calls `Java_io_github_fionapreroll_infrasonic_core_AndroidAudio_nativeInit`.

#[cfg(target_os = "android")]
mod jni {
    use std::sync::Once;

    use jni_sys::{JNIEnv, JavaVM, jclass, jobject};

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
}
