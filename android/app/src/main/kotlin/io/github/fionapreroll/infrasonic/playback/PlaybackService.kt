// SPDX-License-Identifier: GPL-3.0-only
package io.github.fionapreroll.infrasonic.playback

import android.app.PendingIntent
import android.content.Intent
import androidx.annotation.OptIn
import androidx.media3.common.MediaItem
import androidx.media3.common.MediaMetadata
import androidx.media3.common.util.UnstableApi
import androidx.media3.session.LibraryResult
import androidx.media3.session.MediaLibraryService
import androidx.media3.session.MediaSession
import androidx.media3.session.SessionError
import com.google.common.collect.ImmutableList
import com.google.common.util.concurrent.Futures
import com.google.common.util.concurrent.ListenableFuture
import io.github.fionapreroll.infrasonic.R
import io.github.fionapreroll.infrasonic.core.AlbumListKind
import io.github.fionapreroll.infrasonic.core.Song
import io.github.fionapreroll.infrasonic.data.Connection
import io.github.fionapreroll.infrasonic.data.Graph
import io.github.fionapreroll.infrasonic.ui.MainActivity
import kotlinx.coroutines.guava.future
import kotlinx.coroutines.launch

/**
 * The media session: notification, lock screen, Bluetooth and headset buttons, Android Auto
 * (browse tree and search), and resuming playback after the app was closed.
 */
@OptIn(UnstableApi::class)
class PlaybackService : MediaLibraryService() {
    private var session: MediaLibrarySession? = null
    private lateinit var player: RustPlayer
    private lateinit var focus: AudioFocus
    private var lastSearch: List<Song> = emptyList()

    override fun onCreate() {
        super.onCreate()
        Graph.init(application)
        player = RustPlayer(Graph.player)
        focus = AudioFocus(this, Graph.player)
        val open = PendingIntent.getActivity(
            this, 0, Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        session = MediaLibrarySession.Builder(this, player, Callback())
            .setSessionActivity(open)
            .build()
    }

    override fun onGetSession(controllerInfo: MediaSession.ControllerInfo): MediaLibrarySession? = session

    override fun onTaskRemoved(rootIntent: Intent?) {
        if (!Graph.player.isPlaying) {
            Graph.player.saveLocalQueue()
            stopSelf()
        }
    }

    override fun onDestroy() {
        Graph.player.saveLocalQueue()
        focus.release()
        session?.run {
            release()
            player.release()
        }
        session = null
        super.onDestroy()
    }

    private fun nav(id: String, title: Int, type: Int = MediaMetadata.MEDIA_TYPE_FOLDER_MIXED) =
        browsable("nav:$id", getString(title), type = type)

    private fun songsIn(context: String, songs: List<Song>) = songs.map { it.toMediaItem(MediaIds.inContext(context, it.id)) }

    private suspend fun children(parent: String): List<MediaItem> {
        val core = Graph.core
        val offline = Graph.servers.connection.value is Connection.Offline
        return when {
            parent == MediaIds.ROOT && offline -> listOf(nav("pinned", R.string.downloads))
            parent == MediaIds.ROOT -> listOf(
                nav("recent", R.string.albums_recent, MediaMetadata.MEDIA_TYPE_FOLDER_ALBUMS),
                nav("newest", R.string.albums_newest, MediaMetadata.MEDIA_TYPE_FOLDER_ALBUMS),
                nav("random", R.string.albums_random, MediaMetadata.MEDIA_TYPE_FOLDER_ALBUMS),
                nav("starred", R.string.favorites),
                nav("playlists", R.string.playlists, MediaMetadata.MEDIA_TYPE_FOLDER_PLAYLISTS),
                nav("pinned", R.string.downloads),
            )
            parent == "nav:recent" -> core.albumList(AlbumListKind.Recent, 60u, 0u).map { it.toMediaItem() }
            parent == "nav:newest" -> core.albumList(AlbumListKind.Newest, 60u, 0u).map { it.toMediaItem() }
            parent == "nav:random" -> core.albumList(AlbumListKind.Random, 60u, 0u).map { it.toMediaItem() }
            parent == "nav:starred" -> core.starred().let { r -> r.albums.map { it.toMediaItem() } + songsIn(parent, r.songs) }
            parent == "nav:playlists" -> core.playlists().map { it.toMediaItem() }
            parent == "nav:pinned" -> songsIn(parent, core.pinned())
            else -> songsIn(parent, contextSongs(parent))
        }
    }

    /** The songs a context id stands for (album, playlist, favorites, downloads). */
    private suspend fun contextSongs(context: String): List<Song> {
        val core = Graph.core
        return when {
            context.startsWith("album:") -> core.album(context.removePrefix("album:")).songs
            context.startsWith("playlist:") -> core.playlist(context.removePrefix("playlist:")).songs
            context.startsWith("artist:") -> core.artistSongs(context.removePrefix("artist:"))
            context == "nav:starred" -> core.starred().songs
            context == "nav:pinned" -> core.pinned()
            else -> emptyList()
        }
    }

    /** Turn what a controller asks to play into songs; a song in a context plays the context. */
    private suspend fun expand(items: List<MediaItem>, startIndex: Int, startPositionMs: Long): MediaSession.MediaItemsWithStartPosition {
        // Voice search ("play … on infrasonic") comes as one item with only a query.
        items.singleOrNull()?.requestMetadata?.searchQuery?.let { q ->
            val songs = runCatching { songsForQuery(q) }.getOrDefault(emptyList())
            return MediaSession.MediaItemsWithStartPosition(songs.map { it.toMediaItem() }, 0, 0)
        }
        val single = items.singleOrNull()?.mediaId
        if (single != null) {
            val ctx = when {
                single.contains('|') -> single.substringBeforeLast('|')
                single.startsWith("album:") || single.startsWith("playlist:") || single.startsWith("artist:") -> single
                else -> null
            }
            if (ctx != null) {
                val songs = runCatching { contextSongs(ctx) }.getOrDefault(emptyList())
                val songId = songIdOf(single)
                val start = songs.indexOfFirst { it.id == songId }.coerceAtLeast(0)
                return MediaSession.MediaItemsWithStartPosition(songs.map { it.toMediaItem() }, start, 0)
            }
        }
        val resolved = items.mapNotNull { item ->
            val id = songIdOf(item.mediaId) ?: return@mapNotNull null
            (SongRegistry.get(id) ?: runCatching { Graph.core.song(id) }.getOrNull())?.toMediaItem()
        }
        return MediaSession.MediaItemsWithStartPosition(resolved, startIndex, startPositionMs)
    }

    private inner class Callback : MediaLibrarySession.Callback {
        override fun onGetLibraryRoot(
            session: MediaLibrarySession,
            browser: MediaSession.ControllerInfo,
            params: LibraryParams?,
        ): ListenableFuture<LibraryResult<MediaItem>> =
            Futures.immediateFuture(LibraryResult.ofItem(browsable(MediaIds.ROOT, getString(R.string.app_name)), params))

        override fun onGetChildren(
            session: MediaLibrarySession,
            browser: MediaSession.ControllerInfo,
            parentId: String,
            page: Int,
            pageSize: Int,
            params: LibraryParams?,
        ): ListenableFuture<LibraryResult<ImmutableList<MediaItem>>> = Graph.scope.future {
            Graph.servers.awaitUsable()
            try {
                val all = children(parentId)
                val from = (page * pageSize).coerceAtMost(all.size)
                val to = (from + pageSize).coerceAtMost(all.size)
                LibraryResult.ofItemList(ImmutableList.copyOf(all.subList(from, to)), params)
            } catch (e: Exception) {
                LibraryResult.ofError(SessionError.ERROR_IO)
            }
        }

        override fun onGetItem(
            session: MediaLibrarySession,
            browser: MediaSession.ControllerInfo,
            mediaId: String,
        ): ListenableFuture<LibraryResult<MediaItem>> = Graph.scope.future {
            val id = songIdOf(mediaId)
            val song = id?.let { SongRegistry.get(it) ?: runCatching { Graph.core.song(it) }.getOrNull() }
            if (song != null) LibraryResult.ofItem(song.toMediaItem(mediaId), null)
            else LibraryResult.ofError(SessionError.ERROR_BAD_VALUE)
        }

        override fun onSearch(
            session: MediaLibrarySession,
            browser: MediaSession.ControllerInfo,
            query: String,
            params: LibraryParams?,
        ): ListenableFuture<LibraryResult<Void>> {
            Graph.scope.launch {
                lastSearch = runCatching { Graph.core.search(query, 30u, 0u).songs }.getOrDefault(emptyList())
                session.notifySearchResultChanged(browser, query, lastSearch.size, params)
            }
            return Futures.immediateFuture(LibraryResult.ofVoid())
        }

        override fun onGetSearchResult(
            session: MediaLibrarySession,
            browser: MediaSession.ControllerInfo,
            query: String,
            page: Int,
            pageSize: Int,
            params: LibraryParams?,
        ): ListenableFuture<LibraryResult<ImmutableList<MediaItem>>> = Graph.scope.future {
            val songs = lastSearch.ifEmpty { runCatching { Graph.core.search(query, 30u, 0u).songs }.getOrDefault(emptyList()) }
            LibraryResult.ofItemList(ImmutableList.copyOf(songs.map { it.toMediaItem() }), params)
        }

        override fun onSetMediaItems(
            mediaSession: MediaSession,
            controller: MediaSession.ControllerInfo,
            mediaItems: MutableList<MediaItem>,
            startIndex: Int,
            startPositionMs: Long,
        ): ListenableFuture<MediaSession.MediaItemsWithStartPosition> = Graph.scope.future {
            Graph.servers.awaitUsable()
            expand(mediaItems, startIndex, startPositionMs)
        }

        override fun onAddMediaItems(
            mediaSession: MediaSession,
            controller: MediaSession.ControllerInfo,
            mediaItems: MutableList<MediaItem>,
        ): ListenableFuture<MutableList<MediaItem>> = Graph.scope.future {
            expand(mediaItems, 0, 0).mediaItems.toMutableList()
        }

        /** Bluetooth headsets and "resume" in the system media controls start from here. */
        override fun onPlaybackResumption(
            mediaSession: MediaSession,
            controller: MediaSession.ControllerInfo,
            isForPlayback: Boolean,
        ): ListenableFuture<MediaSession.MediaItemsWithStartPosition> = Graph.scope.future {
            Graph.servers.awaitUsable()
            val q = Graph.core.localQueue()
                ?: throw UnsupportedOperationException("nothing to resume")
            MediaSession.MediaItemsWithStartPosition(
                q.songs.map { it.toMediaItem() }, q.index.toInt(), (q.position * 1000).toLong(),
            )
        }
    }
}
