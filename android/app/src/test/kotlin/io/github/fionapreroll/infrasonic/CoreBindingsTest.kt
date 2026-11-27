// SPDX-License-Identifier: GPL-3.0-only
package io.github.fionapreroll.infrasonic

import io.github.fionapreroll.infrasonic.core.AlbumListKind
import io.github.fionapreroll.infrasonic.core.FfiException
import io.github.fionapreroll.infrasonic.core.Infrasonic
import io.github.fionapreroll.infrasonic.core.PlayerEvent
import io.github.fionapreroll.infrasonic.core.PlayerListener
import io.github.fionapreroll.infrasonic.core.ServerProfile
import io.github.fionapreroll.infrasonic.core.tokenAuth
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.TimeUnit

/**
 * The Kotlin bindings against real servers (the library for this machine through JNA).
 * Runs when INFRASONIC_E2E_SERVERS is set, see scripts/e2e-servers.sh.
 */
class CoreBindingsTest {
    @get:Rule
    val tmp = TemporaryFolder()

    private val servers: List<Pair<String, String>> =
        System.getenv("INFRASONIC_E2E_SERVERS").orEmpty().split(';').filter { '=' in it }
            .map { it.substringBefore('=') to it.substringAfter('=') }

    private class Recorder : PlayerListener {
        val events = CopyOnWriteArrayList<PlayerEvent>()
        override fun onEvent(event: PlayerEvent) {
            events += event
        }

        fun await(what: String, timeoutS: Long = 60, check: (List<PlayerEvent>) -> Boolean) {
            val end = System.nanoTime() + TimeUnit.SECONDS.toNanos(timeoutS)
            while (System.nanoTime() < end) {
                if (check(events)) return
                Thread.sleep(50)
            }
            fail("timed out waiting for $what: $events")
        }
    }

    private fun profile(url: String, password: String = "admin") =
        ServerProfile("test", "Test", url, tokenAuth("admin", password), false)

    @Test
    fun libraryPlaybackAndDownloads() = runBlocking {
        assumeTrue("INFRASONIC_E2E_SERVERS not set", servers.isNotEmpty())
        for ((name, url) in servers) {
            val dir = tmp.newFolder(name)
            val app = Infrasonic.withNullOutput("$dir/data", "$dir/cache", 20.0)
            val rec = Recorder()
            app.setListener(rec)

            try {
                app.testConnection(profile(url, "wrong"))
                fail("$name: wrong password accepted")
            } catch (e: FfiException.Auth) {
                // expected
            }

            val status = app.connect(profile(url))
            assertTrue("$name: $status", status.online)
            val albums = app.albumList(AlbumListKind.ByName, 50u, 0u)
            val sine = albums.find { it.name == "Sine Studies" } ?: error("$name: no sine album in $albums")
            val songs = app.album(sine.id).songs
            assertTrue("$name: ${songs.size} songs", songs.size >= 2)
            assertTrue(name, app.search("song", 20u, 0u).songs.isNotEmpty())

            // Play to the end at 20× speed; the songs come in order.
            app.play(songs, 0u)
            rec.await("queue end") { ev -> ev.any { it is PlayerEvent.QueueEnded } }
            val played = rec.events.filterIsInstance<PlayerEvent.TrackChanged>().mapNotNull { it.item?.song?.id }.distinct()
            assertEquals(name, songs.map { it.id }, played)

            // Download one song and find it among the pinned ones.
            assertEquals("$name: failed downloads", 0, app.pinAndWait(listOf(songs[0])).toInt())
            assertTrue(name, app.isPinned(songs[0].id))
            assertTrue(name, app.cacheUsage().pinnedBytes > 0u)
            app.unpin(listOf(songs[0].id))

            // Saved servers survive a restart.
            app.saveProfiles(listOf(profile(url)), "test")
            app.setListener(null)
            app.destroy()
            val again = Infrasonic("$dir/data", "$dir/cache")
            assertEquals(name, "test", again.activeProfileId())
            assertEquals(name, url, again.profiles().single().url)
            again.destroy()
        }
    }
}
