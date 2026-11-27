// SPDX-License-Identifier: GPL-3.0-only
package io.github.fionapreroll.infrasonic.core

import android.content.Context

/**
 * Hands the Java VM and the app context to the Rust audio output (crates/ffi/src/android.rs),
 * which asks the AudioManager about devices and buffer sizes. Call once before playback.
 */
object AndroidAudio {
    init {
        System.loadLibrary("infrasonic_ffi")
    }

    fun init(context: Context) = nativeInit(context.applicationContext)

    @JvmStatic
    private external fun nativeInit(context: Context)
}
