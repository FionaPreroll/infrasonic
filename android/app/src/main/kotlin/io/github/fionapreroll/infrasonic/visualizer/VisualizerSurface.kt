// SPDX-License-Identifier: GPL-3.0-only
package io.github.fionapreroll.infrasonic.visualizer

import android.view.Surface
import android.view.SurfaceHolder

/**
 * Hands a SurfaceView's Surface to the Rust visualizer (crates/ffi/src/android.rs), which renders into it on a thread
 * of its own. [surfaceDestroyed] returns only once Rust no longer uses the Surface, as Android requires.
 */
object VisualizerSurface : SurfaceHolder.Callback {
    init {
        System.loadLibrary("infrasonic_ffi")
    }

    private var attached = false

    override fun surfaceCreated(holder: SurfaceHolder) = Unit

    override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) {
        if (attached) {
            nativeResize(width, height)
        } else {
            nativeAttach(holder.surface, width, height)
            attached = true
        }
    }

    override fun surfaceDestroyed(holder: SurfaceHolder) {
        if (attached) {
            nativeDetach()
            attached = false
        }
    }

    @JvmStatic
    private external fun nativeAttach(surface: Surface, width: Int, height: Int)

    @JvmStatic
    private external fun nativeResize(width: Int, height: Int)

    @JvmStatic
    private external fun nativeDetach()
}
