// SPDX-License-Identifier: GPL-3.0-only
package io.github.fionapreroll.infrasonic.playback

import android.os.Bundle
import androidx.media3.common.MediaItem
import androidx.media3.common.MediaMetadata
import io.github.fionapreroll.infrasonic.core.Album
import io.github.fionapreroll.infrasonic.core.Playlist
import io.github.fionapreroll.infrasonic.core.Song
import io.github.fionapreroll.infrasonic.data.Covers

/**
 * Media ids: `song:<id>` for a song, `<context>|<id>` for a song inside an album, playlist or
 * list (playing it plays the whole context), `album:<id>`, `playlist:<id>`, and `nav:<name>`
 * for the browse tree.
 */
object MediaIds {
    const val ROOT = "nav:root"
    fun song(id: String) = "song:$id"
    fun inContext(context: String, songId: String) = "$context|$songId"
    fun album(id: String) = "album:$id"
    fun playlist(id: String) = "playlist:$id"
    fun artist(id: String) = "artist:$id"
}

/** Songs seen recently, so that media items from controllers can be turned back into songs. */
object SongRegistry {
    private val map = object : LinkedHashMap<String, Song>(256, 0.75f, true) {
        override fun removeEldestEntry(eldest: MutableMap.MutableEntry<String, Song>?) = size > 5000
    }

    @Synchronized
    fun put(s: Song) {
        map[s.id] = s
    }

    @Synchronized
    fun get(id: String): Song? = map[id]
}

/** The song id of a media id, if it names a song. */
fun songIdOf(mediaId: String): String? = when {
    mediaId.startsWith("song:") -> mediaId.removePrefix("song:")
    mediaId.contains('|') -> mediaId.substringAfterLast('|')
    else -> null
}

fun Song.toMediaItem(mediaId: String = MediaIds.song(id)): MediaItem {
    SongRegistry.put(this)
    val meta = MediaMetadata.Builder()
        .setTitle(title)
        .setArtist(artist)
        .setAlbumTitle(album)
        .setAlbumArtist(artist)
        .setTrackNumber(track?.toInt())
        .setDiscNumber(disc?.toInt())
        .setReleaseYear(year)
        .setGenre(genre)
        .setDurationMs(duration?.toLong()?.times(1000))
        .setArtworkUri(Covers.uri(coverArt))
        .setIsBrowsable(false)
        .setIsPlayable(true)
        .setMediaType(if (mediaType == "podcast") MediaMetadata.MEDIA_TYPE_PODCAST_EPISODE else MediaMetadata.MEDIA_TYPE_MUSIC)
        .build()
    return MediaItem.Builder().setMediaId(mediaId).setMediaMetadata(meta).build()
}

fun browsable(
    mediaId: String,
    title: String,
    subtitle: String? = null,
    cover: String? = null,
    type: Int = MediaMetadata.MEDIA_TYPE_FOLDER_MIXED,
    playable: Boolean = false,
    extras: Bundle? = null,
): MediaItem = MediaItem.Builder()
    .setMediaId(mediaId)
    .setMediaMetadata(
        MediaMetadata.Builder()
            .setTitle(title)
            .setArtist(subtitle)
            .setArtworkUri(Covers.uri(cover))
            .setIsBrowsable(true)
            .setIsPlayable(playable)
            .setMediaType(type)
            .setExtras(extras)
            .build(),
    )
    .build()

fun Album.toMediaItem() = browsable(MediaIds.album(id), name, artist, coverArt, MediaMetadata.MEDIA_TYPE_ALBUM, playable = true)
fun Playlist.toMediaItem() = browsable(MediaIds.playlist(id), name, null, coverArt, MediaMetadata.MEDIA_TYPE_PLAYLIST, playable = true)
