// SPDX-License-Identifier: GPL-3.0-only
package io.github.fionapreroll.infrasonic.playback

import android.os.Handler
import android.os.Looper
import io.github.fionapreroll.infrasonic.core.FfiException
import io.github.fionapreroll.infrasonic.core.Infrasonic
import io.github.fionapreroll.infrasonic.core.PlayState
import io.github.fionapreroll.infrasonic.core.PlayerCommand
import io.github.fionapreroll.infrasonic.core.PlayerEvent
import io.github.fionapreroll.infrasonic.core.PlayerListener
import io.github.fionapreroll.infrasonic.core.PlayerState
import io.github.fionapreroll.infrasonic.core.QueueItem
import io.github.fionapreroll.infrasonic.core.Song
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import java.util.concurrent.CopyOnWriteArrayList

/**
 * The app's view of the Rust player: state as flows for the UI, change callbacks for the media
 * session, and the commands. Events from Rust arrive on a background thread and are handled on
 * the main thread.
 */
class PlayerController(private val core: Infrasonic) : PlayerListener {
    private val main = Handler(Looper.getMainLooper())

    private val _state = MutableStateFlow<PlayerState?>(null)
    val state: StateFlow<PlayerState?> = _state.asStateFlow()

    private val _queue = MutableStateFlow<List<QueueItem>>(emptyList())
    val queue: StateFlow<List<QueueItem>> = _queue.asStateFlow()

    /** Seconds; updated about twice per second while playing. */
    private val _position = MutableStateFlow(0.0)
    val position: StateFlow<Double> = _position.asStateFlow()
    private var positionAt = System.nanoTime()

    private val _messages = MutableSharedFlow<String>(extraBufferCapacity = 16)
    val messages: SharedFlow<String> = _messages.asSharedFlow()

    /** Incremented when a song finished downloading (refresh "downloaded" marks). */
    private val _pinned = MutableStateFlow(0)
    val pinned: StateFlow<Int> = _pinned.asStateFlow()

    /** After removing downloads. */
    fun notifyPinned() {
        _pinned.value++
    }

    private val listeners = CopyOnWriteArrayList<() -> Unit>()

    /** Called on the main thread after every change except position updates. */
    fun addListener(l: () -> Unit) = listeners.add(l)
    fun removeListener(l: () -> Unit) = listeners.remove(l)

    override fun onEvent(event: PlayerEvent) {
        main.post { handle(event) }
    }

    private fun handle(e: PlayerEvent) {
        when (e) {
            is PlayerEvent.Position -> {
                _position.value = e.position
                positionAt = System.nanoTime()
                _state.value = _state.value?.copy(position = e.position, duration = e.duration)
                return
            }
            is PlayerEvent.Pinned -> {
                _pinned.value++
                return
            }
            is PlayerEvent.Scrobbled -> return
            is PlayerEvent.Error -> _messages.tryEmit(e.message)
            is PlayerEvent.QueueChanged, is PlayerEvent.TrackChanged -> _queue.value = core.queue()
            else -> {}
        }
        refreshState()
        if (e is PlayerEvent.StateChanged && e.state == PlayState.PAUSED || e is PlayerEvent.TrackChanged) saveLocalQueue()
        listeners.forEach { it() }
    }

    private fun refreshState() {
        val s = core.state()
        _state.value = s
        if (s != null) {
            _position.value = s.position
            positionAt = System.nanoTime()
        }
    }

    /** Read everything again (after connecting). */
    fun refresh() {
        _queue.value = core.queue()
        refreshState()
        listeners.forEach { it() }
    }

    /** The position now, extrapolated between updates while playing. */
    fun positionNow(): Double {
        val s = _state.value ?: return 0.0
        if (s.state != PlayState.PLAYING) return _position.value
        val p = _position.value + (System.nanoTime() - positionAt) / 1e9
        return s.duration?.let { p.coerceAtMost(it) } ?: p
    }

    val isPlaying get() = _state.value?.state.let { it == PlayState.PLAYING || it == PlayState.BUFFERING }

    private fun guard(block: () -> Unit) {
        try {
            block()
        } catch (e: FfiException) {
            _messages.tryEmit(e.reason())
        }
    }

    fun play(songs: List<Song>, start: Int = 0) = guard { if (songs.isNotEmpty()) core.play(songs, start.toUInt()) }
    fun playShuffled(songs: List<Song>) = play(songs.shuffled())
    fun enqueue(songs: List<Song>) = guard { core.enqueue(songs) }
    fun enqueueNext(songs: List<Song>) = guard { core.enqueueNext(songs) }
    fun setQueue(songs: List<Song>, start: Int, position: Double) = guard { core.setQueue(songs, start.toUInt(), position) }
    fun command(c: PlayerCommand) = guard { core.command(c) }
    fun togglePause() = command(if (isPlaying) PlayerCommand.Pause else PlayerCommand.Play)
    fun next() = command(PlayerCommand.Next)
    fun previous() = command(PlayerCommand.Previous)
    fun seek(seconds: Double) {
        _position.value = seconds
        positionAt = System.nanoTime()
        command(PlayerCommand.Seek(seconds))
    }

    fun saveLocalQueue() {
        if (_queue.value.isEmpty()) return
        runCatching { core.saveLocalQueue() }
    }

    /** Put back the queue saved on this device, paused. */
    fun restoreLocalQueue(): Boolean {
        val q = core.localQueue() ?: return false
        setQueue(q.songs, q.index.toInt(), q.position)
        return true
    }
}

/** A message for the user. */
fun FfiException.reason(): String = when (this) {
    is FfiException.Auth -> reason
    is FfiException.Server -> reason
    is FfiException.Network -> reason
    is FfiException.Protocol -> reason
    is FfiException.NotSupported -> reason
    is FfiException.Other -> reason
    is FfiException.NotConnected -> "not connected"
}
