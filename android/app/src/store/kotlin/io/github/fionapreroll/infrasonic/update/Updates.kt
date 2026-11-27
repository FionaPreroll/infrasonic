// SPDX-License-Identifier: GPL-3.0-only
package io.github.fionapreroll.infrasonic.update

import android.content.Context
import java.io.File

/**
 * The store build (Google Play, F-Droid) does not look for new versions of itself: the store brings
 * them. Nothing here reaches out, and the build has no permission to install apps (compare
 * src/github/).
 */
object Updates {
    const val AVAILABLE = false

    suspend fun latest(): Release? = null

    suspend fun download(context: Context, release: Release, progress: (Float) -> Unit): File? = null

    fun install(context: Context, apk: File) = Unit
}
