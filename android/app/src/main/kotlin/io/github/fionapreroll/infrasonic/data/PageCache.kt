// SPDX-License-Identifier: GPL-3.0-only
package io.github.fionapreroll.infrasonic.data

import android.os.SystemClock
import android.util.LruCache

/**
 * The content of recently shown pages, in memory only, so that going back or switching tabs
 * shows a page at once. Keys contain the server and whether it is used offline, so that nothing
 * from one server or mode shows up in another. Entries older than [FRESH_MS], and all entries
 * after [stale], are shown too, but loaded again in the background.
 */
class PageCache(lowRam: Boolean) {
    inner class Entry(val value: Any?, val time: Long) {
        val fresh get() = time > staleBefore && SystemClock.elapsedRealtime() - time < FRESH_MS
    }

    @Volatile
    private var staleBefore = 0L

    private val lru = LruCache<String, Entry>(if (lowRam) 15 else 40)

    fun key(server: String?, offline: Boolean, page: String) = "${server.orEmpty()}|${if (offline) "off" else "on"}|$page"

    fun get(key: String): Entry? = lru.get(key)

    fun put(key: String, value: Any?) {
        lru.put(key, Entry(value, SystemClock.elapsedRealtime()))
    }

    /**
     * Something changed on the server (a star, a playlist, ...): pages are still shown from
     * memory, but loaded again.
     */
    fun stale() {
        staleBefore = SystemClock.elapsedRealtime()
    }

    /** Pages in memory. */
    val size get() = lru.size()

    /** Keep only the most recent half (the system is short of memory). */
    fun trim() = lru.trimToSize(lru.maxSize() / 2)

    fun clear() = lru.evictAll()

    companion object {
        const val FRESH_MS = 5 * 60_000L
    }
}
