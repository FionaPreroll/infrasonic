// SPDX-License-Identifier: GPL-3.0-only
package io.github.fionapreroll.infrasonic.playback

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlin.math.roundToInt

/** Speeds offered in the player. */
val SPEEDS = listOf(0.75f, 1f, 1.1f, 1.25f, 1.5f, 1.75f, 2f, 2.5f, 3f)

/** Whether a song is spoken word: a podcast episode, an audio book, or simply very long. */
fun isSpoken(mediaType: String?, genre: String?, durationSecs: Double?, longMinutes: Int): Boolean {
    if (mediaType.equals("podcast", ignoreCase = true)) return true
    val g = genre.orEmpty().lowercase()
    if (listOf("podcast", "audiobook", "audio book", "hörbuch", "hörspiel", "spoken").any { it in g }) return true
    val minutes = if (longMinutes > 0) longMinutes else 15
    return durationSecs != null && durationSecs >= minutes * 60
}

/**
 * Playback speed, remembered separately for music and for spoken word, so that a podcast at
 * 1.5x does not make the next album play fast. [apply] tells the core; it is called only when
 * the speed changes.
 */
class SpeedPolicy(
    music: Float,
    spoken: Float,
    private val save: (spoken: Boolean, speed: Float) -> Unit,
    private val apply: (Float) -> Unit,
) {
    private var speeds = mutableMapOf(false to music, true to spoken)
    private var spokenNow = false
    private var applied = Float.NaN

    private val _speed = MutableStateFlow(music)
    /** The speed of what plays now. */
    val speed: StateFlow<Float> = _speed.asStateFlow()

    private val _spoken = MutableStateFlow(false)
    val spoken: StateFlow<Boolean> = _spoken.asStateFlow()

    /** A new song plays. */
    @Synchronized
    fun onSong(spoken: Boolean) {
        spokenNow = spoken
        push()
    }

    /** The user picked a speed for what plays now (and the same kind of songs). */
    @Synchronized
    fun set(speed: Float) {
        val s = speed.coerceIn(0.5f, 3f)
        speeds[spokenNow] = s
        save(spokenNow, s)
        push()
    }

    private fun push() {
        val s = speeds.getValue(spokenNow)
        _speed.value = s
        _spoken.value = spokenNow
        if (s != applied) {
            applied = s
            apply(s)
        }
    }
}

/** "1.25×", "2×". */
fun speedLabel(s: Float): String {
    val r = (s * 100).roundToInt()
    val text = if (r % 100 == 0) "${r / 100}" else if (r % 10 == 0) "%.1f".format(s) else "%.2f".format(s)
    return "$text×"
}

/** What the sleep timer does. */
sealed interface SleepMode {
    data object Off : SleepMode
    /** Pause at [endsAt] (milliseconds of the timer's clock), after fading out. */
    data class At(val endsAt: Long) : SleepMode
    /** Pause when the current song ends. */
    data object EndOfSong : SleepMode
}

/** What the sleep timer needs from the player. */
interface SleepPlayer {
    fun volume(): Int
    fun setVolume(volume: Int)
    fun pause()
    fun pauseAfterCurrent(on: Boolean)
}

/**
 * Sleep timer: after some minutes the volume fades out over [fadeMs] and playback pauses (the
 * volume is put back for the next time), or playback pauses when the song ends. Pausing by
 * hand ends the "end of song" mode; a timer keeps running.
 */
class SleepTimer(
    private val scope: CoroutineScope,
    private val player: SleepPlayer,
    private val now: () -> Long = System::currentTimeMillis,
    private val fadeMs: Long = 20_000,
) {
    private val _mode = MutableStateFlow<SleepMode>(SleepMode.Off)
    val mode: StateFlow<SleepMode> = _mode.asStateFlow()
    private var job: Job? = null

    fun start(minutes: Int) {
        cancel()
        val endsAt = now() + minutes * 60_000L
        _mode.value = SleepMode.At(endsAt)
        job = scope.launch {
            val fadeFrom = endsAt - fadeMs
            delay((fadeFrom - now()).coerceAtLeast(0))
            val volume = player.volume()
            val steps = 20
            for (i in 1..steps) {
                player.setVolume((volume * (steps - i) / steps.toFloat()).roundToInt())
                delay(fadeMs / steps)
            }
            player.pause()
            player.setVolume(volume)
            _mode.value = SleepMode.Off
        }
    }

    fun endOfSong() {
        cancel()
        player.pauseAfterCurrent(true)
        _mode.value = SleepMode.EndOfSong
    }

    fun cancel() {
        val wasEndOfSong = _mode.value == SleepMode.EndOfSong
        job?.cancel()
        job = null
        _mode.value = SleepMode.Off
        if (wasEndOfSong) player.pauseAfterCurrent(false)
    }

    /** Playback paused or stopped (by the engine at the end of the song, or by hand). */
    fun onPaused() {
        if (_mode.value == SleepMode.EndOfSong) _mode.value = SleepMode.Off
    }

    /** Milliseconds left, for the display. */
    fun remaining(): Long? = (_mode.value as? SleepMode.At)?.let { (it.endsAt - now()).coerceAtLeast(0) }
}

/** Which favorites to download and which automatic downloads to remove. */
data class FavoritePlan(val pin: Set<String>, val unpin: Set<String>, val auto: Set<String>)

/**
 * Downloads for "download favorites": starred songs that are not downloaded yet get
 * downloaded; songs this downloaded before and that are no longer starred are removed.
 * Songs downloaded by hand are never removed. [auto] are the songs downloaded automatically.
 */
fun planFavorites(starred: Set<String>, pinned: Set<String>, auto: Set<String>): FavoritePlan {
    val pin = starred - pinned
    val unpin = (auto - starred) intersect pinned
    return FavoritePlan(pin, unpin, (auto intersect starred intersect pinned) + pin)
}
