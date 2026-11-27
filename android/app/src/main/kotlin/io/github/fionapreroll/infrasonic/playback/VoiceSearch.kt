// SPDX-License-Identifier: GPL-3.0-only
package io.github.fionapreroll.infrasonic.playback

import io.github.fionapreroll.infrasonic.core.Song
import io.github.fionapreroll.infrasonic.data.Graph

/**
 * What to play for "play … on infrasonic" (Assistant, Android Auto). An empty query plays
 * random songs; otherwise matching songs, or else the best matching album or artist.
 */
suspend fun songsForQuery(query: String?): List<Song> {
    val core = Graph.core
    val q = query?.trim().orEmpty()
    if (q.isEmpty()) return runCatching { core.randomSongs(50u, null) }.getOrElse { core.pinned().shuffled() }
    val r = core.search(q, 50u, 0u)
    val artist = r.artists.firstOrNull { it.name.equals(q, ignoreCase = true) }
    val album = r.albums.firstOrNull { it.name.equals(q, ignoreCase = true) }
    return when {
        artist != null -> core.artistSongs(artist.id)
        album != null -> core.album(album.id).songs
        r.songs.isNotEmpty() -> r.songs
        r.albums.isNotEmpty() -> core.album(r.albums[0].id).songs
        r.artists.isNotEmpty() -> core.artistSongs(r.artists[0].id)
        else -> emptyList()
    }
}
