// SPDX-License-Identifier: GPL-3.0-only
package io.github.fionapreroll.infrasonic.update

import android.content.Context
import android.content.Intent
import androidx.core.content.FileProvider
import androidx.core.content.pm.PackageInfoCompat
import io.github.fionapreroll.infrasonic.BuildConfig
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext
import java.io.ByteArrayOutputStream
import java.io.File
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URL
import java.security.MessageDigest
import kotlin.coroutines.coroutineContext

/**
 * New versions of the app, for the build from the release android-latest on GitHub (the store build
 * has none, see src/store/). The check reads version.json from the release; a newer build is
 * downloaded into the app's own cache, where no other app can change it, and handed to Android's
 * installer only if it has the size and SHA-256 that version.json names and is this app in that
 * build. The installer asks the user and takes it only if it is signed with the same key as the
 * installed app; an APK of another package would not be checked against that key, hence the check.
 */
object Updates {
    const val AVAILABLE = true

    private const val APK = "infrasonic.apk"
    private const val TIMEOUT_MS = 15_000

    /** The release's version.json, or null if it did not answer. */
    suspend fun latest(): Release? = withContext(Dispatchers.IO) {
        try {
            open("version.json").useConnection { c ->
                c.inputStream.use { input ->
                    val out = ByteArrayOutputStream()
                    val buf = ByteArray(1024)
                    while (true) {
                        val n = input.read(buf)
                        if (n < 0) break
                        out.write(buf, 0, n)
                        if (out.size() > Release.MAX_JSON) return@useConnection null
                    }
                    Release.parse(out.toString(Charsets.UTF_8.name()))
                }
            }
        } catch (e: IOException) {
            null
        }
    }

    /** Downloads and checks the APK of `release`; null if it failed or is not that release. */
    suspend fun download(context: Context, release: Release, progress: (Float) -> Unit): File? =
        withContext(Dispatchers.IO) {
            val dir = File(context.cacheDir, "updates")
            val file = File(dir, APK)
            dir.mkdirs()
            file.delete()
            val ok = try {
                fetchApk(file, release, progress)
            } catch (e: IOException) {
                false
            }
            if (ok && describes(context, file, release)) file else null.also { file.delete() }
        }

    /** Hands the checked APK to Android's installer, which asks the user. */
    fun install(context: Context, apk: File) {
        val uri = FileProvider.getUriForFile(context, context.packageName + ".updates", apk)
        val intent = Intent(Intent.ACTION_VIEW)
            .setDataAndType(uri, "application/vnd.android.package-archive")
            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
        context.startActivity(intent)
    }

    /** Writes exactly `release.size` bytes with the right SHA-256, or returns false. */
    private suspend fun fetchApk(file: File, release: Release, progress: (Float) -> Unit): Boolean =
        open(APK).useConnection { c ->
            val sha = MessageDigest.getInstance("SHA-256")
            var total = 0L
            var shown = -1
            c.inputStream.use { input ->
                file.outputStream().use { out ->
                    val buf = ByteArray(64 * 1024)
                    while (true) {
                        coroutineContext.ensureActive()
                        val n = input.read(buf)
                        if (n < 0) break
                        total += n
                        if (total > release.size) return@useConnection false
                        sha.update(buf, 0, n)
                        out.write(buf, 0, n)
                        val percent = (total * 100 / release.size).toInt()
                        if (percent != shown) {
                            shown = percent
                            progress(percent / 100f)
                        }
                    }
                }
            }
            total == release.size && sha.digest().joinToString("") { "%02x".format(it) } == release.sha256
        } ?: false

    private fun describes(context: Context, apk: File, release: Release): Boolean {
        val info = context.packageManager.getPackageArchiveInfo(apk.path, 0) ?: return false
        return info.packageName == context.packageName && PackageInfoCompat.getLongVersionCode(info) == release.build
    }

    /** A GET of a file of the release; GitHub answers with a redirect to its storage. */
    private fun open(name: String): HttpURLConnection {
        val c = URL(BuildConfig.UPDATES_URL + name).openConnection() as HttpURLConnection
        c.connectTimeout = TIMEOUT_MS
        c.readTimeout = TIMEOUT_MS
        c.useCaches = false
        c.instanceFollowRedirects = true
        return c
    }

    private inline fun <T> HttpURLConnection.useConnection(block: (HttpURLConnection) -> T?): T? =
        try {
            if (responseCode == HttpURLConnection.HTTP_OK) block(this) else null
        } finally {
            disconnect()
        }
}
