// SPDX-License-Identifier: GPL-3.0-only
package io.github.fionapreroll.infrasonic.playback

import android.os.Looper
import androidx.annotation.OptIn
import androidx.media3.common.AudioAttributes
import androidx.media3.common.C
import androidx.media3.common.MediaItem
import androidx.media3.common.Player
import androidx.media3.common.SimpleBasePlayer
import androidx.media3.common.util.UnstableApi
import com.google.common.util.concurrent.Futures
import com.google.common.util.concurrent.ListenableFuture
import io.github.fionapreroll.infrasonic.core.PlayState
import io.github.fionapreroll.infrasonic.core.PlayerCommand
import io.github.fionapreroll.infrasonic.core.Repeat
import io.github.fionapreroll.infrasonic.core.Song
import io.github.fionapreroll.infrasonic.data.Graph
import kotlinx.coroutines.guava.future

/**
 * A Media3 player backed by the Rust engine, for the media session: notification, lock
 * screen, Bluetooth, Android Auto and other controllers. The queue and state live in Rust;
 * this class only describes them and forwards commands.
 */
@OptIn(UnstableApi::class)
class RustPlayer(private val controller: PlayerController) : SimpleBasePlayer(Looper.getMainLooper()) {
    private val onChange: () -> Unit = { invalidateState() }

    init {
        controller.addListener(onChange)
    }

    private val commands = Player.Commands.Builder().addAll(
        COMMAND_PLAY_PAUSE, COMMAND_PREPARE, COMMAND_STOP, COMMAND_SEEK_TO_DEFAULT_POSITION,
        COMMAND_SEEK_IN_CURRENT_MEDIA_ITEM, COMMAND_SEEK_TO_PREVIOUS_MEDIA_ITEM, COMMAND_SEEK_TO_PREVIOUS,
        COMMAND_SEEK_TO_NEXT_MEDIA_ITEM, COMMAND_SEEK_TO_NEXT, COMMAND_SEEK_TO_MEDIA_ITEM, COMMAND_SEEK_BACK,
        COMMAND_SEEK_FORWARD, COMMAND_SET_REPEAT_MODE, COMMAND_SET_SHUFFLE_MODE, COMMAND_GET_CURRENT_MEDIA_ITEM,
        COMMAND_GET_TIMELINE, COMMAND_GET_METADATA, COMMAND_SET_MEDIA_ITEM, COMMAND_CHANGE_MEDIA_ITEMS,
        COMMAND_GET_AUDIO_ATTRIBUTES, COMMAND_RELEASE,
    ).build()

    private val attributes = AudioAttributes.Builder()
        .setUsage(C.USAGE_MEDIA)
        .setContentType(C.AUDIO_CONTENT_TYPE_MUSIC)
        .build()

    override fun getState(): State {
        val st = controller.state.value
        val queue = controller.queue.value
        val playlist = queue.map { item ->
            MediaItemData.Builder(item.uid)
                .setMediaItem(item.song.toMediaItem())
                .setDurationUs(item.song.duration?.toLong()?.times(1_000_000) ?: C.TIME_UNSET)
                .build()
        }
        val playing = st?.state == PlayState.PLAYING || st?.state == PlayState.BUFFERING
        val playbackState = when {
            queue.isEmpty() || st == null -> STATE_IDLE
            st.state == PlayState.BUFFERING -> STATE_BUFFERING
            st.state == PlayState.STOPPED && st.current == null -> STATE_ENDED
            else -> STATE_READY
        }
        val posMs = (controller.positionNow() * 1000).toLong()
        val b = State.Builder()
            .setAvailableCommands(commands)
            .setPlayWhenReady(playing, PLAY_WHEN_READY_CHANGE_REASON_USER_REQUEST)
            .setPlaybackState(playbackState)
            .setAudioAttributes(attributes)
            .setRepeatMode(
                when (st?.repeat) {
                    Repeat.ALL -> REPEAT_MODE_ALL
                    Repeat.ONE -> REPEAT_MODE_ONE
                    else -> REPEAT_MODE_OFF
                },
            )
            .setSeekBackIncrementMs(10_000)
            .setSeekForwardIncrementMs(30_000)
            .setPlaylist(playlist)
            .setContentPositionMs(
                if (st?.state == PlayState.PLAYING) PositionSupplier.getExtrapolating(posMs, 1f)
                else PositionSupplier.getConstant(posMs),
            )
        if (playlist.isNotEmpty()) {
            b.setCurrentMediaItemIndex((st?.index?.toInt() ?: 0).coerceIn(0, playlist.size - 1))
        }
        return b.build()
    }

    private fun done(): ListenableFuture<*> = Futures.immediateVoidFuture()

    override fun handleSetPlayWhenReady(playWhenReady: Boolean): ListenableFuture<*> {
        val st = controller.state.value
        when {
            !playWhenReady -> controller.command(PlayerCommand.Pause)
            st?.state == PlayState.STOPPED && st.current == null && controller.queue.value.isNotEmpty() ->
                controller.command(PlayerCommand.PlayIndex(0u))
            else -> controller.command(PlayerCommand.Play)
        }
        return done()
    }

    override fun handlePrepare(): ListenableFuture<*> = done()

    override fun handleStop(): ListenableFuture<*> {
        controller.command(PlayerCommand.Pause)
        return done()
    }

    override fun handleSeek(mediaItemIndex: Int, positionMs: Long, seekCommand: Int): ListenableFuture<*> {
        val current = controller.state.value?.index?.toInt() ?: 0
        if (mediaItemIndex != C.INDEX_UNSET && mediaItemIndex != current) {
            controller.command(PlayerCommand.PlayIndex(mediaItemIndex.toUInt()))
        }
        if (positionMs != C.TIME_UNSET && (positionMs > 0 || mediaItemIndex == current)) {
            controller.seek(positionMs / 1000.0)
        }
        return done()
    }

    override fun handleSetRepeatMode(repeatMode: Int): ListenableFuture<*> {
        val r = when (repeatMode) {
            REPEAT_MODE_ALL -> Repeat.ALL
            REPEAT_MODE_ONE -> Repeat.ONE
            else -> Repeat.OFF
        }
        controller.command(PlayerCommand.SetRepeat(r))
        return done()
    }

    /** Shuffling reorders the queue once; there is no lasting shuffle mode. */
    override fun handleSetShuffleModeEnabled(shuffleModeEnabled: Boolean): ListenableFuture<*> {
        if (shuffleModeEnabled) controller.command(PlayerCommand.Shuffle)
        return done()
    }

    private suspend fun songs(items: List<MediaItem>): List<Song> = items.mapNotNull { item ->
        val id = songIdOf(item.mediaId) ?: return@mapNotNull null
        SongRegistry.get(id) ?: runCatching { Graph.core.song(id) }.getOrNull()
    }

    override fun handleSetMediaItems(mediaItems: MutableList<MediaItem>, startIndex: Int, startPositionMs: Long): ListenableFuture<*> =
        Graph.scope.future {
            val songs = songs(mediaItems)
            val start = if (startIndex == C.INDEX_UNSET) 0 else startIndex
            val pos = if (startPositionMs == C.TIME_UNSET) 0.0 else startPositionMs / 1000.0
            controller.setQueue(songs, start.coerceIn(0, (songs.size - 1).coerceAtLeast(0)), pos)
        }

    override fun handleAddMediaItems(index: Int, mediaItems: MutableList<MediaItem>): ListenableFuture<*> =
        Graph.scope.future {
            val songs = songs(mediaItems)
            val size = controller.queue.value.size
            val current = controller.state.value?.index?.toInt() ?: -1
            when {
                index >= size -> controller.enqueue(songs)
                index == current + 1 -> controller.enqueueNext(songs)
                else -> {
                    controller.enqueue(songs)
                    songs.indices.forEach { i ->
                        controller.command(PlayerCommand.Move((size + i).toUInt(), (index + i).toUInt()))
                    }
                }
            }
        }

    override fun handleRemoveMediaItems(fromIndex: Int, toIndex: Int): ListenableFuture<*> {
        for (i in toIndex - 1 downTo fromIndex) controller.command(PlayerCommand.Remove(i.toUInt()))
        return done()
    }

    override fun handleMoveMediaItems(fromIndex: Int, toIndex: Int, newIndex: Int): ListenableFuture<*> {
        val n = toIndex - fromIndex
        if (newIndex < fromIndex) {
            for (i in 0 until n) controller.command(PlayerCommand.Move((fromIndex + i).toUInt(), (newIndex + i).toUInt()))
        } else {
            for (i in 0 until n) controller.command(PlayerCommand.Move(fromIndex.toUInt(), (newIndex + n - 1).toUInt()))
        }
        return done()
    }

    override fun handleRelease(): ListenableFuture<*> {
        controller.removeListener(onChange)
        return done()
    }
}
