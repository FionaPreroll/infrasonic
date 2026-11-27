// SPDX-License-Identifier: GPL-3.0-only
package io.github.fionapreroll.infrasonic.playback

import io.github.fionapreroll.infrasonic.ui.Shortcut
import io.github.fionapreroll.infrasonic.ui.currentLyricsLine
import io.github.fionapreroll.infrasonic.ui.dropTarget
import io.github.fionapreroll.infrasonic.ui.swipeStep
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class ComfortTest {
    @Test
    fun speedIsRememberedForMusicAndSpokenWord() {
        val applied = mutableListOf<Float>()
        val saved = mutableMapOf<Boolean, Float>()
        val p = SpeedPolicy(1f, 1.5f, save = { k, s -> saved[k] = s }, apply = { applied += it })
        p.onSong(spoken = false)
        p.onSong(spoken = true)
        p.onSong(spoken = true)
        p.set(2f)
        p.onSong(spoken = false)
        p.set(0.1f)
        p.onSong(spoken = true)
        assertEquals(listOf(1f, 1.5f, 2f, 1f, 0.5f, 2f), applied)
        assertEquals(mapOf(true to 2f, false to 0.5f), saved)
        assertEquals(2f, p.speed.value)
        assertTrue(p.spoken.value)
    }

    @Test
    fun spokenWord() {
        assertTrue(isSpoken("podcast", null, 300.0, 15))
        assertTrue(isSpoken(null, "Hörbuch", 200.0, 15))
        assertTrue(isSpoken("music", null, 16 * 60.0, 15))
        assertFalse(isSpoken("music", "Rock", 14 * 60.0, 15))
        assertTrue("resume off still knows long tracks", isSpoken(null, null, 20 * 60.0, 0))
        assertFalse(isSpoken(null, null, null, 15))
    }

    @Test
    fun speedLabels() {
        assertEquals("1×", speedLabel(1f))
        assertEquals("2×", speedLabel(2f))
        assertTrue(speedLabel(1.25f).matches(Regex("1[.,]25×")))
        assertTrue(speedLabel(1.5f).matches(Regex("1[.,]5×")))
    }

    private class FakePlayer : SleepPlayer {
        var level = 80
        val volumes = mutableListOf<Int>()
        var paused = 0
        val afterCurrent = mutableListOf<Boolean>()
        override fun volume() = level
        override fun setVolume(volume: Int) {
            level = volume
            volumes += volume
        }
        override fun pause() {
            paused++
        }
        override fun pauseAfterCurrent(on: Boolean) {
            afterCurrent += on
        }
    }

    @Test
    fun sleepTimerFadesOutPausesAndRestoresTheVolume() {
        val scope = TestScope()
        val player = FakePlayer()
        val t = SleepTimer(scope, player, now = { scope.testScheduler.currentTime }, fadeMs = 20_000)
        t.start(10)
        assertEquals(10 * 60_000L, t.remaining())
        scope.advanceTimeBy(9 * 60_000L + 39_000)
        scope.runCurrent()
        assertTrue("no fade yet", player.volumes.isEmpty())
        scope.advanceTimeBy(2_000)
        scope.runCurrent()
        assertTrue("fading", player.volumes.isNotEmpty() && player.volumes.last() < 80)
        assertEquals(0, player.paused)
        scope.advanceTimeBy(20_000)
        scope.runCurrent()
        assertEquals(1, player.paused)
        assertEquals("the volume falls steadily", player.volumes.dropLast(1), player.volumes.dropLast(1).sortedDescending())
        assertEquals(0, player.volumes[player.volumes.size - 2])
        assertEquals("and is back for the next time", 80, player.level)
        assertEquals(SleepMode.Off, t.mode.value)
        assertNull(t.remaining())
    }

    @Test
    fun sleepTimerCanBeCancelledAndReplaced() {
        val scope = TestScope()
        val player = FakePlayer()
        val t = SleepTimer(scope, player, now = { scope.testScheduler.currentTime }, fadeMs = 1_000)
        t.start(5)
        scope.advanceTimeBy(60_000)
        t.start(1)
        scope.advanceTimeBy(30_000)
        t.cancel()
        scope.advanceTimeBy(10 * 60_000L)
        scope.runCurrent()
        assertEquals(0, player.paused)
        assertTrue(player.volumes.isEmpty())
        assertEquals(SleepMode.Off, t.mode.value)
    }

    @Test
    fun sleepAtTheEndOfTheSong() {
        val scope = TestScope()
        val player = FakePlayer()
        val t = SleepTimer(scope, player, now = { scope.testScheduler.currentTime })
        t.endOfSong()
        assertEquals(SleepMode.EndOfSong, t.mode.value)
        t.onPaused()
        assertEquals(SleepMode.Off, t.mode.value)
        assertEquals(listOf(true), player.afterCurrent)
        t.endOfSong()
        t.cancel()
        assertEquals("turned off in the engine too", listOf(true, true, false), player.afterCurrent)
        t.endOfSong()
        t.start(10)
        assertEquals("a timer replaces it", listOf(true, true, false, true, false), player.afterCurrent)
    }

    @Test
    fun favoritesPlan() {
        val p = planFavorites(
            starred = setOf("a", "b", "c"),
            pinned = setOf("a", "x", "y"),
            auto = setOf("a", "y", "z"),
        )
        assertEquals(setOf("b", "c"), p.pin)
        assertEquals("by hand stays, automatic and no longer starred goes", setOf("y"), p.unpin)
        assertEquals(setOf("a", "b", "c"), p.auto)
        val again = planFavorites(setOf("a", "b", "c"), setOf("a", "b", "c", "x"), p.auto)
        assertTrue(again.pin.isEmpty() && again.unpin.isEmpty())
    }

    @Test
    fun gestures() {
        assertEquals(0, swipeStep(20f, 1000f, 100f))
        assertEquals(1, swipeStep(-400f, 1000f, 0f))
        assertEquals(-1, swipeStep(400f, 1000f, 0f))
        assertEquals("a flick", 1, swipeStep(-100f, 1000f, -2000f))
        assertEquals("too short even if fast", 0, swipeStep(-20f, 1000f, -3000f))

        assertEquals(3, dropTarget(3, 20f, 100f, 10))
        assertEquals(5, dropTarget(3, 180f, 100f, 10))
        assertEquals(0, dropTarget(3, -900f, 100f, 10))
        assertEquals(9, dropTarget(3, 2000f, 100f, 10))
        assertEquals(3, dropTarget(3, 500f, 0f, 10))
    }

    @Test
    fun lyricsLine() {
        val starts = listOf(1000L, 3000L, null, 5000L)
        assertEquals(-1, currentLyricsLine(starts, 500))
        assertEquals(0, currentLyricsLine(starts, 1000))
        assertEquals(1, currentLyricsLine(starts, 4999))
        assertEquals(3, currentLyricsLine(starts, 60_000))
        assertEquals(-1, currentLyricsLine(emptyList(), 10))
    }

    @Test
    fun shortcuts() {
        for (s in Shortcut.entries) assertEquals(s, Shortcut.from(s.action))
        assertNull(Shortcut.from("android.intent.action.MAIN"))
        assertNull(Shortcut.from(null))
    }
}
