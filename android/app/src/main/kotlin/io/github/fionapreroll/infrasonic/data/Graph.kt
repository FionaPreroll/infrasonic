// SPDX-License-Identifier: GPL-3.0-only
package io.github.fionapreroll.infrasonic.data

import android.app.ActivityManager
import android.app.Application
import android.content.ComponentCallbacks2
import android.content.Context
import androidx.core.content.edit
import io.github.fionapreroll.infrasonic.BuildConfig
import io.github.fionapreroll.infrasonic.core.EqualizerSettings
import io.github.fionapreroll.infrasonic.core.Infrasonic
import io.github.fionapreroll.infrasonic.core.initLogging
import io.github.fionapreroll.infrasonic.playback.AudioRoutes
import io.github.fionapreroll.infrasonic.playback.EqStore
import io.github.fionapreroll.infrasonic.playback.EqualizerManager
import io.github.fionapreroll.infrasonic.core.PlayerCommand
import io.github.fionapreroll.infrasonic.playback.PlayerController
import io.github.fionapreroll.infrasonic.playback.SleepPlayer
import io.github.fionapreroll.infrasonic.playback.SleepTimer
import io.github.fionapreroll.infrasonic.playback.SpeedPolicy
import io.github.fionapreroll.infrasonic.playback.isSpoken
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
    lateinit var pages: PageCache
        private set
    lateinit var network: NetworkQuality
        private set
    lateinit var equalizer: EqualizerManager
        private set
    lateinit var routes: AudioRoutes
        private set
    lateinit var speed: SpeedPolicy
        private set
    lateinit var sleep: SleepTimer
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
        val lowRam = app.getSystemService(ActivityManager::class.java)?.isLowRamDevice == true
        covers = Covers(app, core, server = { servers.activeId.value }, limitMb = { prefs.coverCacheMb.value }, lowRam = lowRam)
        pages = PageCache(lowRam)
        servers = Servers(core, prefs, player)
        network = NetworkQuality(app, core, prefs, servers)
        val eqPrefs = app.getSharedPreferences("equalizer", Context.MODE_PRIVATE)
        val eqStore = object : EqStore {
            override fun load() = eqPrefs.getString("state", null)
            override fun save(json: String) = eqPrefs.edit { putString("state", json) }
        }
        equalizer = EqualizerManager(eqStore, { l -> core.setEqualizer(l?.let { EqualizerSettings(it.preamp, it.gains) }) })
        routes = AudioRoutes(app) { equalizer.onDevice(it) }
        routes.start()
        speed = SpeedPolicy(
            prefs.speedMusic.value,
            prefs.speedSpoken.value,
            save = { spoken, s -> (if (spoken) prefs.speedSpoken else prefs.speedMusic).value = s },
            apply = { core.setSpeed(it) },
        )
        sleep = SleepTimer(
            scope,
            object : SleepPlayer {
                override fun volume() = core.state()?.volume?.toInt() ?: core.settings().volume.toInt()
                override fun setVolume(volume: Int) = player.command(PlayerCommand.SetVolume(volume.coerceIn(0, 100).toUByte()))
                override fun pause() = player.command(PlayerCommand.Pause)
                override fun pauseAfterCurrent(on: Boolean) {
                    runCatching { core.pauseAfterCurrent(on) }
                }
            },
        )
        player.onTrack = { item ->
            val s = item?.song
            val long = core.settings().resumeAfterMinutes.toInt()
            speed.onSong(s != null && isSpoken(s.mediaType, s.genre, s.duration?.toDouble(), long))
        }
        player.onPaused = { sleep.onPaused() }
        ready = true
        scope.launch { servers.connectActive() }
    }

    /** Free memory when the system asks for it (see [android.content.ComponentCallbacks2]). */
    fun trimMemory(level: Int) {
        if (!ready) return
        @Suppress("DEPRECATION")
        when {
            level >= ComponentCallbacks2.TRIM_MEMORY_COMPLETE || level == ComponentCallbacks2.TRIM_MEMORY_RUNNING_CRITICAL -> {
                covers.clearMemory()
                pages.clear()
            }
            level >= ComponentCallbacks2.TRIM_MEMORY_BACKGROUND || level == ComponentCallbacks2.TRIM_MEMORY_RUNNING_LOW -> {
                covers.trimMemory()
                pages.trim()
            }
        }
    }
}
