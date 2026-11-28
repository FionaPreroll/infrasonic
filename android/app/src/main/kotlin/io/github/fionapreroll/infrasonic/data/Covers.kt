// SPDX-License-Identifier: GPL-3.0-only
package io.github.fionapreroll.infrasonic.data

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import android.util.LruCache
import io.github.fionapreroll.infrasonic.BuildConfig
import io.github.fionapreroll.infrasonic.core.Infrasonic
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import java.io.File
import java.security.MessageDigest
import java.util.concurrent.atomic.AtomicLong

/**
 * Cover art from the server, through the Rust client (so self-signed servers work too), with a
 * memory and a disk cache. [CoverProvider] serves the same files to the notification, Android
 * Auto and the widget.
 *
 * Memory: decoded bitmaps up to 1/8 of the app's heap (1/16 on devices with little RAM), never
 * larger than asked for, even if the server sends a bigger image. Disk: files per server, id and
 * size, up to [limitMb]; the ones shown longest ago are deleted first. A size that is not on
 * disk yet is made from a bigger one of the same cover if there is one.
 */
class Covers(
    context: Context,
    private val core: Infrasonic,
    private val server: () -> String?,
    private val limitMb: () -> Int,
    lowRam: Boolean,
) {
    private val dir = File(context.cacheDir, "art")
    private val memory = object : LruCache<String, Bitmap>((Runtime.getRuntime().maxMemory() / (if (lowRam) 16 else 8) / 1024).toInt()) {
        override fun sizeOf(key: String, value: Bitmap) = value.byteCount / 1024
    }
    private val locks = Array(16) { Mutex() }
    private val io = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    /** Bytes on disk; -1 until counted. */
    private val used = AtomicLong(-1)
    private val evicting = Mutex()

    init {
        io.launch {
            // Before covers were kept per server (no limit, ids of two servers could mix up).
            File(context.cacheDir, "covers").deleteRecursively()
            used.set(dirFiles().sumOf { it.length() })
            evict()
        }
    }

    private fun name(id: String): String =
        MessageDigest.getInstance("SHA-256").digest("${server().orEmpty()}\u0000$id".toByteArray())
            .joinToString("") { "%02x".format(it) }.take(32)

    private fun key(id: String, size: Int) = "${server().orEmpty()}\u0000${id}_$size"

    private fun fileFor(id: String, size: Int) = File(dir, "${name(id)}_$size")

    private fun dirFiles(): List<File> = dir.listFiles()?.filter { it.isFile && !it.name.endsWith(".part") }.orEmpty()

    private fun usable(f: File) = f.isFile && f.length() > 0

    /** A file of this cover in [size] or bigger; marks it as used. */
    private fun onDisk(id: String, size: Int): File? {
        val f = fileFor(id, size).takeIf(::usable)
            ?: SIZES.filter { it > size }.firstNotNullOfOrNull { s -> fileFor(id, s).takeIf(::usable) }
            ?: return null
        f.setLastModified(System.currentTimeMillis())
        return f
    }

    fun cached(id: String, size: Int): Bitmap? = memory.get(key(id, size))

    /** An image file of at least [size] pixels, downloading it if needed; null if the server has none. */
    suspend fun file(id: String, size: Int): File? = withContext(Dispatchers.IO) {
        onDisk(id, size)?.let { return@withContext it }
        locks[(id.hashCode() and 0x7fffffff) % locks.size].withLock {
            onDisk(id, size)?.let { return@withLock it }
            val bytes = runCatching { core.coverArt(id, size.toUInt()) }.getOrNull()
            if (bytes == null || bytes.isEmpty()) return@withLock null
            val f = fileFor(id, size)
            dir.mkdirs()
            val tmp = File(dir, f.name + ".part")
            tmp.writeBytes(bytes)
            if (!tmp.renameTo(f)) {
                tmp.delete()
                return@withLock null
            }
            if (used.get() >= 0) used.addAndGet(f.length())
            io.launch { evict() }
            f
        }
    }

    fun fileBlocking(id: String, size: Int): File? = runBlocking { file(id, size) }

    suspend fun bitmap(id: String, size: Int): Bitmap? {
        cached(id, size)?.let { return it }
        val f = file(id, size) ?: return null
        return withContext(Dispatchers.IO) {
            decode(f, size)?.also { memory.put(key(id, size), it) }
        }
    }

    /** Delete the files shown longest ago until the cache is below 90 % of its limit. */
    suspend fun evict() = evicting.withLock {
        val limit = limitMb().toLong() * 1024 * 1024
        if (used.get() in 0..limit) return@withLock
        withContext(Dispatchers.IO) { used.set(trimDir(dirFiles(), limit)) }
    }

    /** Bytes of cover files on disk. */
    fun diskBytes(): Long = used.get().coerceAtLeast(0)

    /** Bytes of decoded covers in memory. */
    fun memoryBytes(): Long = memory.size().toLong() * 1024

    /** Keep half of the covers in memory (the system is short of memory). */
    fun trimMemory() = memory.trimToSize(memory.maxSize() / 2)

    fun clearMemory() = memory.evictAll()

    fun clear() {
        memory.evictAll()
        dir.deleteRecursively()
        used.set(0)
    }

    companion object {
        const val AUTHORITY = BuildConfig.APPLICATION_ID + ".covers"

        /** Sizes the app asks for; a smaller one is made from a bigger one on disk. */
        val SIZES = listOf(96, 128, 300, 512, 800, 1024)

        /** Where all covers live; read access is granted for this prefix. */
        val ROOT: Uri by lazy { Uri.Builder().scheme("content").authority(AUTHORITY).build() }

        /** A content URI for a cover, for media notifications and Android Auto. */
        fun uri(id: String?, size: Int = 512): Uri? =
            id?.let { Uri.Builder().scheme("content").authority(AUTHORITY).appendPath(it).appendQueryParameter("size", "$size").build() }

        /**
         * Delete the [files] used longest ago (oldest modification time) until they are below
         * 90 % of [limit] bytes, if they are above it. Returns the bytes left.
         */
        fun trimDir(files: List<File>, limit: Long): Long {
            val sorted = files.map { Triple(it, it.lastModified(), it.length()) }.sortedBy { it.second }
            var total = sorted.sumOf { it.third }
            if (total <= limit) return total
            for ((f, _, len) in sorted) {
                if (total <= limit * 9 / 10) break
                if (f.delete()) total -= len
            }
            return total
        }

        /** Decode [f] scaled down by powers of two to about [size] pixels. */
        fun decode(f: File, size: Int): Bitmap? {
            val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
            BitmapFactory.decodeFile(f.path, bounds)
            if (bounds.outWidth <= 0 || bounds.outHeight <= 0) return null
            return BitmapFactory.decodeFile(f.path, BitmapFactory.Options().apply { inSampleSize = sampleSize(bounds.outWidth, bounds.outHeight, size) })
        }

        /** The biggest power of two that keeps the smaller side at [size] pixels or more. */
        fun sampleSize(width: Int, height: Int, size: Int): Int {
            var s = 1
            while (minOf(width, height) / (s * 2) >= size) s *= 2
            return s
        }
    }
}
