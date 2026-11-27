// SPDX-License-Identifier: GPL-3.0-only
package io.github.fionapreroll.infrasonic.downloads

import android.content.Context
import androidx.work.Constraints
import androidx.work.CoroutineWorker
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.ExistingWorkPolicy
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import io.github.fionapreroll.infrasonic.data.Graph
import io.github.fionapreroll.infrasonic.playback.planFavorites
import java.util.concurrent.TimeUnit

/**
 * "Download favorites": keeps the starred songs downloaded, over Wi-Fi only. Runs twice a day,
 * right after the setting is turned on and after a song is starred or unstarred.
 */
object FavoriteDownloads {
    private const val PERIODIC = "favorites"
    private const val NOW = "favorites-now"

    private fun constraints() = Constraints.Builder()
        .setRequiredNetworkType(NetworkType.UNMETERED)
        .setRequiresStorageNotLow(true)
        .build()

    /**
     * Start or stop the regular sync, at the app's start and after the setting changed
     * ([now]: also sync right away).
     */
    fun schedule(context: Context, now: Boolean = false) {
        val wm = WorkManager.getInstance(context)
        if (!Graph.prefs.autoFavorites.value) {
            wm.cancelUniqueWork(PERIODIC)
            return
        }
        wm.enqueueUniquePeriodicWork(
            PERIODIC,
            ExistingPeriodicWorkPolicy.KEEP,
            PeriodicWorkRequestBuilder<FavoritesWorker>(12, TimeUnit.HOURS).setConstraints(constraints()).build(),
        )
        if (now) syncSoon(context)
    }

    /** Favorites changed: sync once the device is on Wi-Fi. */
    fun syncSoon(context: Context) {
        if (!Graph.prefs.autoFavorites.value) return
        WorkManager.getInstance(context).enqueueUniqueWork(
            NOW,
            ExistingWorkPolicy.REPLACE,
            OneTimeWorkRequestBuilder<FavoritesWorker>()
                .setConstraints(constraints())
                .setInitialDelay(10, TimeUnit.SECONDS)
                .build(),
        )
    }
}

class FavoritesWorker(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {
    override suspend fun doWork(): Result {
        Graph.init(applicationContext as android.app.Application)
        val prefs = Graph.prefs
        if (!prefs.autoFavorites.value) return Result.success()
        if (!Graph.servers.awaitUsable()) return Result.retry()
        val core = Graph.core
        val starred = runCatching { core.starred().songs }.getOrElse { return Result.retry() }
        val pinned = core.pinned().map { it.id }.toSet()
        val plan = planFavorites(starred.map { it.id }.toSet(), pinned, prefs.autoPinned.value)
        if (plan.unpin.isNotEmpty()) runCatching { core.unpin(plan.unpin.toList()) }
        // Note them first, so that an interrupted run still knows what it started.
        prefs.autoPinned.value = plan.auto
        val songs = starred.filter { it.id in plan.pin }
        if (songs.isNotEmpty()) {
            val failed = runCatching { core.pinAndWait(songs) }.getOrElse { return Result.retry() }
            if (failed > 0u && runAttemptCount < 3) return Result.retry()
        }
        Graph.player.notifyPinned()
        return Result.success()
    }
}
