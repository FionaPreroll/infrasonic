// SPDX-License-Identifier: GPL-3.0-only
package io.github.fionapreroll.infrasonic

import android.app.Application
import io.github.fionapreroll.infrasonic.core.AndroidAudio
import io.github.fionapreroll.infrasonic.data.Graph

class App : Application() {
    override fun onCreate() {
        super.onCreate()
        AndroidAudio.init(this)
        Graph.init(this)
    }
}
