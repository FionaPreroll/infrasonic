// SPDX-License-Identifier: GPL-3.0-only
package io.github.fionapreroll.infrasonic.ui

import android.Manifest
import android.app.SearchManager
import android.content.ComponentName
import android.content.Intent
import android.provider.MediaStore
import android.content.pm.PackageManager
import android.os.Build
import android.content.Context
import android.os.Bundle
import android.widget.Toast
import androidx.activity.compose.setContent
import androidx.annotation.StringRes
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AppCompatActivity
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.core.content.ContextCompat
import androidx.core.content.edit
import androidx.lifecycle.lifecycleScope
import androidx.media3.session.MediaController
import androidx.media3.session.SessionToken
import com.google.common.util.concurrent.ListenableFuture
import io.github.fionapreroll.infrasonic.downloads.FavoriteDownloads
import io.github.fionapreroll.infrasonic.R
import io.github.fionapreroll.infrasonic.core.FfiException
import io.github.fionapreroll.infrasonic.data.Graph
import io.github.fionapreroll.infrasonic.playback.PlaybackService
import io.github.fionapreroll.infrasonic.playback.reason
import io.github.fionapreroll.infrasonic.playback.songsForQuery
import io.github.fionapreroll.infrasonic.update.AppUpdates
import io.github.fionapreroll.infrasonic.visualizer.Presets
import kotlinx.coroutines.launch
import io.github.fionapreroll.infrasonic.widget.PlayerWidget
import androidx.core.content.pm.ShortcutInfoCompat
import androidx.core.content.pm.ShortcutManagerCompat
import androidx.core.graphics.drawable.IconCompat
import io.github.fionapreroll.infrasonic.core.PlayerCommand

class MainActivity : AppCompatActivity() {
    private var controller: ListenableFuture<MediaController>? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        Graph.init(application)
        PlayerWidget.follow(this)
        askForNotifications()
        handle(intent)
        if (savedInstanceState == null) {
            AppUpdates.onStart(applicationContext)
            Presets.onStart(applicationContext)
            FavoriteDownloads.schedule(applicationContext)
            publishShortcuts()
        }
        setContent {
            val theme by Graph.prefs.theme.flow.collectAsState()
            val dynamic by Graph.prefs.dynamicColor.flow.collectAsState()
            InfrasonicTheme(theme, dynamic) { AppRoot() }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handle(intent)
    }

    /** Long-press shortcuts on the launcher icon (in the app's language, so set at each start). */
    private fun publishShortcuts() {
        fun shortcut(s: Shortcut, short: Int, long: Int, icon: Int) =
            ShortcutInfoCompat.Builder(this, s.name)
                .setShortLabel(getString(short))
                .setLongLabel(getString(long))
                .setIcon(IconCompat.createWithResource(this, icon))
                .setIntent(Intent(s.action, null, this, MainActivity::class.java))
                .build()
        val list = listOf(
            shortcut(Shortcut.Resume, R.string.shortcut_resume, R.string.shortcut_resume_long, R.drawable.ic_shortcut_play),
            shortcut(Shortcut.ShuffleAll, R.string.shortcut_shuffle, R.string.shortcut_shuffle, R.drawable.ic_shortcut_shuffle),
            shortcut(Shortcut.Favorites, R.string.shortcut_favorites, R.string.shortcut_favorites_long, R.drawable.ic_shortcut_favorite),
            shortcut(Shortcut.Search, R.string.shortcut_search, R.string.shortcut_search, R.drawable.ic_shortcut_search),
        )
        runCatching { ShortcutManagerCompat.setDynamicShortcuts(this, list) }
    }

    private fun handleShortcut(s: Shortcut) {
        ShortcutManagerCompat.reportShortcutUsed(this, s.name)
        val player = Graph.player
        if (s == Shortcut.Search) {
            openRoute.tryEmit(Routes.SEARCH)
            return
        }
        lifecycleScope.launch {
            Graph.servers.awaitUsable()
            when (s) {
                Shortcut.ShuffleAll -> runCatching { Graph.core.randomSongs(100u, null) }
                    .onSuccess { player.play(it) }.onFailure { toast(it) }
                Shortcut.Favorites -> runCatching { Graph.core.starred().songs }
                    .onSuccess { if (it.isEmpty()) toast(R.string.no_favorites) else player.playShuffled(it) }
                    .onFailure { toast(it) }
                Shortcut.Resume -> if (player.state.value?.current != null || player.restoreLocalQueue()) {
                    player.command(PlayerCommand.Play)
                }
                Shortcut.Search -> {}
            }
        }
    }

    /** "Play … on infrasonic" from the Assistant, and launcher shortcuts. */
    private fun handle(intent: Intent?) {
        Shortcut.from(intent?.action)?.let {
            handleShortcut(it)
            setIntent(Intent(this, MainActivity::class.java))
            return
        }
        if (intent?.action != MediaStore.INTENT_ACTION_MEDIA_PLAY_FROM_SEARCH) return
        val query = intent.getStringExtra(SearchManager.QUERY)
        lifecycleScope.launch {
            Graph.servers.awaitUsable()
            runCatching { songsForQuery(query) }
                .onSuccess { if (it.isEmpty()) toast(R.string.no_results) else Graph.player.play(it) }
                .onFailure { toast(it) }
        }
        setIntent(Intent(this, MainActivity::class.java))
    }

    /** Downloads show their progress in a notification (Android 13 and later ask first). */
    private fun askForNotifications() {
        if (Build.VERSION.SDK_INT < 33) return
        val permission = Manifest.permission.POST_NOTIFICATIONS
        if (ContextCompat.checkSelfPermission(this, permission) == PackageManager.PERMISSION_GRANTED) return
        val asked = getPreferences(MODE_PRIVATE)
        if (asked.getBoolean("asked_notifications", false)) return
        asked.edit { putBoolean("asked_notifications", true) }
        registerForActivityResult(ActivityResultContracts.RequestPermission()) {}.launch(permission)
    }

    override fun onStart() {
        super.onStart()
        // A connected controller keeps the playback service (and its notification) alive.
        controller = MediaController.Builder(this, SessionToken(this, ComponentName(this, PlaybackService::class.java))).buildAsync()
    }

    override fun onStop() {
        controller?.let { MediaController.releaseFuture(it) }
        controller = null
        Graph.player.saveLocalQueue()
        super.onStop()
    }
}

fun Context.toast(text: String) = Toast.makeText(this, text, Toast.LENGTH_LONG).show()

fun Context.toast(e: Throwable) = toast(if (e is FfiException) e.reason() else e.message ?: e.toString())

/** A toast with a text resource (read when shown, so it follows the current language). */
fun Context.toast(@StringRes text: Int, vararg args: Any) = toast(getString(text, *args))
