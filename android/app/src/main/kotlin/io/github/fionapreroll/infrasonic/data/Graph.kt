// SPDX-License-Identifier: GPL-3.0-only
package io.github.fionapreroll.infrasonic.data

import android.app.Application
import io.github.fionapreroll.infrasonic.BuildConfig
import io.github.fionapreroll.infrasonic.core.Infrasonic
import io.github.fionapreroll.infrasonic.core.initLogging
import io.github.fionapreroll.infrasonic.playback.PlayerController
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch

/** The app's long-lived objects. */
object Graph {
    lateinit var app: Application
        private set
    lateinit var core: Infrasonic
        private set
    lateinit var prefs: UiPrefs
        private set
    lateinit var servers: Servers
        private set
    lateinit var player: PlayerController
        private set
    lateinit var covers: Covers
        private set
    lateinit var network: NetworkQuality
        private set

    val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)

    @Volatile
    private var ready = false

    @Synchronized
    fun init(app: Application) {
        if (ready) return
        this.app = app
        initLogging(if (BuildConfig.DEBUG) "debug" else "info")
        core = Infrasonic(app.filesDir.resolve("core").path, app.cacheDir.resolve("core").path)
        prefs = UiPrefs(app)
        player = PlayerController(core)
        core.setListener(player)
        covers = Covers(app, core)
        servers = Servers(core, prefs, player)
        network = NetworkQuality(app, core, prefs, servers)
        ready = true
        scope.launch { servers.connectActive() }
    }
}
