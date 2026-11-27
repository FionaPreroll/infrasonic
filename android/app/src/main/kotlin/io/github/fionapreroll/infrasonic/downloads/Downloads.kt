// SPDX-License-Identifier: GPL-3.0-only
package io.github.fionapreroll.infrasonic.downloads

import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.Context
import android.content.pm.ServiceInfo
import android.os.Build
import androidx.core.app.NotificationCompat
import androidx.work.Constraints
import androidx.work.CoroutineWorker
import androidx.work.ExistingWorkPolicy
import androidx.work.ForegroundInfo
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import androidx.work.workDataOf
import io.github.fionapreroll.infrasonic.R
import io.github.fionapreroll.infrasonic.core.Song
import io.github.fionapreroll.infrasonic.data.Graph
import io.github.fionapreroll.infrasonic.playback.SongRegistry
import java.util.UUID

/**
 * Downloads for offline use ("pinning") as WorkManager jobs, so that they continue in the
 * background, wait for Wi-Fi if wanted and survive restarts. Items are `song:<id>`,
 * `album:<id>`, `playlist:<id>`, `artist:<id>` or `folder:<id>`.
 */
object Downloads {
    const val CHANNEL = "downloads"
    private const val KEY_ITEMS = "items"

    fun songs(context: Context, songs: List<Song>) {
        songs.forEach(SongRegistry::put)
        enqueue(context, songs.map { "song:${it.id}" })
    }

    fun enqueue(context: Context, items: List<String>) {
        if (items.isEmpty()) return
        val wifiOnly = Graph.prefs.downloadOnWifiOnly.value
        // WorkManager limits input data to 10 KB.
        items.chunked(200).forEach { chunk ->
            val req = OneTimeWorkRequestBuilder<PinWorker>()
                .setInputData(workDataOf(KEY_ITEMS to chunk.toTypedArray()))
                .setConstraints(
                    Constraints.Builder()
                        .setRequiredNetworkType(if (wifiOnly) NetworkType.UNMETERED else NetworkType.CONNECTED)
                        .setRequiresStorageNotLow(true)
                        .build(),
                )
                .addTag(CHANNEL)
                .build()
            WorkManager.getInstance(context)
                .enqueueUniqueWork("pin-${UUID.randomUUID()}", ExistingWorkPolicy.KEEP, req)
        }
    }

    fun cancelAll(context: Context) = WorkManager.getInstance(context).cancelAllWorkByTag(CHANNEL)

    internal fun items(params: WorkerParameters) = params.inputData.getStringArray(KEY_ITEMS)?.toList().orEmpty()

    internal fun ensureChannel(context: Context) {
        val nm = context.getSystemService(NotificationManager::class.java)
        if (nm.getNotificationChannel(CHANNEL) == null) {
            nm.createNotificationChannel(
                NotificationChannel(CHANNEL, context.getString(R.string.downloads), NotificationManager.IMPORTANCE_LOW),
            )
        }
    }
}

class PinWorker(context: Context, private val params: WorkerParameters) : CoroutineWorker(context, params) {
    override suspend fun getForegroundInfo(): ForegroundInfo {
        Downloads.ensureChannel(applicationContext)
        val n = NotificationCompat.Builder(applicationContext, Downloads.CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(applicationContext.getString(R.string.downloading))
            .setOngoing(true)
            .setSilent(true)
            .build()
        return if (Build.VERSION.SDK_INT >= 29) {
            ForegroundInfo(NOTIFICATION_ID, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)
        } else {
            ForegroundInfo(NOTIFICATION_ID, n)
        }
    }

    override suspend fun doWork(): Result {
        Graph.init(applicationContext as android.app.Application)
        runCatching { setForeground(getForegroundInfo()) }
        if (!Graph.servers.awaitUsable()) return Result.retry()
        val core = Graph.core
        val songs = Downloads.items(params).flatMap { item ->
            val id = item.substringAfter(':')
            runCatching {
                when (item.substringBefore(':')) {
                    "song" -> listOf(SongRegistry.get(id) ?: core.song(id))
                    "album" -> core.album(id).songs
                    "playlist" -> core.playlist(id).songs
                    "artist" -> core.artistSongs(id)
                    "folder" -> core.folderSongs(id)
                    else -> emptyList()
                }
            }.getOrElse { return Result.retry() }
        }
        val failed = runCatching { core.pinAndWait(songs) }.getOrElse { return Result.retry() }
        return if (failed > 0u && runAttemptCount < 3) Result.retry() else Result.success()
    }

    private companion object {
        const val NOTIFICATION_ID = 2
    }
}
