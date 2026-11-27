// SPDX-License-Identifier: GPL-3.0-only
package io.github.fionapreroll.infrasonic.data

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import android.util.LruCache
import io.github.fionapreroll.infrasonic.BuildConfig
import io.github.fionapreroll.infrasonic.core.Infrasonic
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import java.io.File
import java.security.MessageDigest

/**
 * Cover art from the server, through the Rust client (so self-signed servers work too), with a
 * memory and a disk cache. [CoverProvider] serves the same files to the notification, Android
 * Auto and the widget.
 */
class Covers(context: Context, private val core: Infrasonic) {
    private val dir = File(context.cacheDir, "covers")
    private val memory = object : LruCache<String, Bitmap>((Runtime.getRuntime().maxMemory() / 8 / 1024).toInt()) {
        override fun sizeOf(key: String, value: Bitmap) = value.byteCount / 1024
    }
    private val locks = Array(16) { Mutex() }

    private fun key(id: String, size: Int) = "${id}_$size"

    private fun fileFor(id: String, size: Int): File {
        val h = MessageDigest.getInstance("SHA-256").digest(id.toByteArray())
            .joinToString("") { "%02x".format(it) }.take(32)
        return File(dir, "${h}_$size")
    }

    fun cached(id: String, size: Int): Bitmap? = memory.get(key(id, size))

    /** The image file, downloading it if needed; null if the server has none. */
    suspend fun file(id: String, size: Int): File? = withContext(Dispatchers.IO) {
        val f = fileFor(id, size)
        if (f.isFile && f.length() > 0) return@withContext f
        locks[(id.hashCode() and 0x7fffffff) % locks.size].withLock {
            if (f.isFile && f.length() > 0) return@withLock f
            val bytes = runCatching { core.coverArt(id, size.toUInt()) }.getOrNull()
            if (bytes == null || bytes.isEmpty()) return@withLock null
            dir.mkdirs()
            val tmp = File(dir, f.name + ".part")
            tmp.writeBytes(bytes)
            tmp.renameTo(f)
            f
        }
    }

    fun fileBlocking(id: String, size: Int): File? = runBlocking { file(id, size) }

    suspend fun bitmap(id: String, size: Int): Bitmap? {
        cached(id, size)?.let { return it }
        val f = file(id, size) ?: return null
        return withContext(Dispatchers.IO) {
            BitmapFactory.decodeFile(f.path)?.also { memory.put(key(id, size), it) }
        }
    }

    fun clear() {
        memory.evictAll()
        dir.deleteRecursively()
    }

    companion object {
        const val AUTHORITY = BuildConfig.APPLICATION_ID + ".covers"

        /** A content URI for a cover, for media notifications, Android Auto and widgets. */
        fun uri(id: String?, size: Int = 512): Uri? =
            id?.let { Uri.Builder().scheme("content").authority(AUTHORITY).appendPath(it).appendQueryParameter("size", "$size").build() }
    }
}
