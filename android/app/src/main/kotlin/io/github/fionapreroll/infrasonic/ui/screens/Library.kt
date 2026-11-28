// SPDX-License-Identifier: GPL-3.0-only
@file:OptIn(ExperimentalMaterial3Api::class)

package io.github.fionapreroll.infrasonic.ui.screens

import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.GridItemSpan
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.lazy.grid.rememberLazyGridState
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.PlaylistAdd
import androidx.compose.material.icons.filled.Download
import androidx.compose.material.icons.filled.Favorite
import androidx.compose.material.icons.filled.FavoriteBorder
import androidx.compose.material.icons.filled.Folder
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.Radio
import androidx.compose.material.icons.filled.Share
import androidx.compose.material.icons.filled.Shuffle
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.github.fionapreroll.infrasonic.R
import io.github.fionapreroll.infrasonic.core.Album
import io.github.fionapreroll.infrasonic.core.AlbumListKind
import io.github.fionapreroll.infrasonic.core.ItemKind
import io.github.fionapreroll.infrasonic.core.Song
import io.github.fionapreroll.infrasonic.data.Connection
import io.github.fionapreroll.infrasonic.data.Graph
import io.github.fionapreroll.infrasonic.downloads.Downloads
import io.github.fionapreroll.infrasonic.ui.LocalNav
import io.github.fionapreroll.infrasonic.ui.Routes
import io.github.fionapreroll.infrasonic.ui.Screen
import io.github.fionapreroll.infrasonic.ui.components.Cover
import io.github.fionapreroll.infrasonic.ui.components.Empty
import io.github.fionapreroll.infrasonic.ui.components.FastScroll
import io.github.fionapreroll.infrasonic.ui.components.initial
import io.github.fionapreroll.infrasonic.ui.components.ErrorBox
import io.github.fionapreroll.infrasonic.ui.components.Loader
import io.github.fionapreroll.infrasonic.ui.components.rememberPage
import io.github.fionapreroll.infrasonic.ui.components.SectionTitle
import io.github.fionapreroll.infrasonic.ui.components.SongRow
import io.github.fionapreroll.infrasonic.ui.components.Tile
import io.github.fionapreroll.infrasonic.ui.components.errorText
import io.github.fionapreroll.infrasonic.ui.components.formatTime
import io.github.fionapreroll.infrasonic.ui.components.share
import io.github.fionapreroll.infrasonic.ui.toast
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.launch
import androidx.compose.foundation.lazy.rememberLazyListState
import io.github.fionapreroll.infrasonic.ui.ScrollToTopOnReselect
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.Box
import androidx.compose.material3.TextButton

private enum class ListTab(val label: Int, val kind: AlbumListKind) {
    /** Rows of recently and often played, new and random albums. */
    Overview(R.string.albums_overview, AlbumListKind.Recent),
    Newest(R.string.albums_newest, AlbumListKind.Newest),
    Recent(R.string.albums_recent, AlbumListKind.Recent),
    Frequent(R.string.albums_frequent, AlbumListKind.Frequent),
    Random(R.string.albums_random, AlbumListKind.Random),
    Starred(R.string.favorites, AlbumListKind.Starred),
    Highest(R.string.albums_highest, AlbumListKind.Highest),
    ByName(R.string.albums_by_name, AlbumListKind.ByName),
    ByArtist(R.string.albums_by_artist, AlbumListKind.ByArtist),
}

private const val PAGE = 60

/** Album lists with paging; offline the downloaded songs instead. */
@Composable
fun HomeScreen() {
    val connection by Graph.servers.connection.collectAsState()
    if (connection is Connection.Offline) {
        DownloadsScreen(back = false)
        return
    }
    var tab by rememberSaveable { mutableStateOf(ListTab.Overview) }
    val nav = LocalNav.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    Screen(stringResource(R.string.albums), back = false, actions = {
        IconButton(onClick = {
            scope.launch {
                runCatching { Graph.core.randomSongs(100u, null) }.onSuccess { Graph.player.play(it) }.onFailure { context.toast(it) }
            }
        }) { Icon(Icons.Default.Shuffle, stringResource(R.string.shuffle_all)) }
    }) {
        Row(Modifier.horizontalScroll(rememberScrollState()).padding(horizontal = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            ListTab.entries.forEach { t ->
                FilterChip(selected = t == tab, onClick = { tab = t }, label = { Text(stringResource(t.label)) })
            }
            FilterChip(selected = false, onClick = { nav.navigate(Routes.GENRES) }, label = { Text(stringResource(R.string.genres)) })
        }
        if (tab == ListTab.Overview) Overview { tab = it } else AlbumGrid(tab.kind, tab = tab)
    }
}

/** The start page: a row of albums for each list, with a link to the whole list. */
@Composable
private fun Overview(open: (ListTab) -> Unit) {
    val nav = LocalNav.current
    val list = rememberLazyListState()
    ScrollToTopOnReselect(Routes.HOME) { list.animateScrollToItem(0) }
    val rows = listOf(ListTab.Recent, ListTab.Frequent, ListTab.Newest, ListTab.Random)
    LazyColumn(state = list) {
        items(rows, key = { it.name }) { t ->
            val (albums, _) = rememberPage(t, cache = "row/${t.name}") { Graph.core.albumList(t.kind, 20u, 0u) }
            val a = albums?.getOrDefault(emptyList())
            if (a != null && a.isEmpty()) return@items
            Row(Modifier.fillMaxWidth().padding(end = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                Box(Modifier.weight(1f)) { SectionTitle(stringResource(t.label)) }
                TextButton(onClick = { open(t) }) { Text(stringResource(R.string.show_all)) }
            }
            LazyRow(Modifier.height(200.dp), contentPadding = PaddingValues(horizontal = 8.dp)) {
                items(a.orEmpty(), key = { it.id }) { al ->
                    Box(Modifier.width(140.dp)) { Tile(al.name, al.artist, al.coverArt) { nav.navigate(Routes.album(al.id)) } }
                }
            }
        }
    }
}

/** What [AlbumGrid] keeps in [io.github.fionapreroll.infrasonic.data.PageCache]. */
private class GridPages(val albums: List<Album>, val end: Boolean)

@Composable
private fun AlbumGrid(kind: AlbumListKind, tab: Any) {
    val server by Graph.servers.activeId.collectAsState()
    val offline = Graph.servers.connection.collectAsState().value is Connection.Offline
    val key = tab to server
    // The pages loaded so far stay in memory, so that coming back shows them at once.
    val cacheKey = Graph.pages.key(server, offline, "grid/$tab")
    val cached = remember(key) { Graph.pages.get(cacheKey) }
    val albums = remember(key) { mutableStateListOf<Album>().apply { (cached?.value as? GridPages)?.let { addAll(it.albums) } } }
    var end by remember(key) { mutableStateOf((cached?.value as? GridPages)?.end ?: false) }
    var error by remember(key) { mutableStateOf<Throwable?>(null) }
    var loading by remember(key) { mutableStateOf(false) }
    val state = rememberLazyGridState()
    ScrollToTopOnReselect(Routes.HOME) { state.animateScrollToItem(0) }
    val nav = LocalNav.current
    suspend fun more() {
        if (loading || end) return
        loading = true
        // Right after the start or a server change the session is still connecting.
        Graph.servers.awaitUsable()
        runCatching { Graph.core.albumList(kind, PAGE.toUInt(), albums.size.toUInt()) }
            .onSuccess { page ->
                albums += page
                if (page.size < PAGE || kind == AlbumListKind.Random) end = true
                Graph.pages.put(cacheKey, GridPages(albums.toList(), end))
            }
            .onFailure { error = it }
        loading = false
    }
    LaunchedEffect(key) {
        if (cached?.fresh == true) return@LaunchedEffect
        // Stale: start again from the first page, keeping what is shown until it is there.
        if (cached != null) {
            loading = true
            Graph.servers.awaitUsable()
            runCatching { Graph.core.albumList(kind, PAGE.toUInt(), 0u) }.onSuccess { page ->
                albums.clear()
                albums += page
                end = page.size < PAGE || kind == AlbumListKind.Random
                Graph.pages.put(cacheKey, GridPages(albums.toList(), end))
            }
            loading = false
        } else {
            more()
        }
    }
    LaunchedEffect(key, state) {
        snapshotFlow { state.layoutInfo.visibleItemsInfo.lastOrNull()?.index ?: 0 }
            .distinctUntilChanged()
            .collect { last -> if (last >= albums.size - 12) more() }
    }
    error?.let {
        ErrorBox(errorText(it)) { error = null }
        return
    }
    if (albums.isEmpty() && end) {
        Empty(stringResource(R.string.no_albums))
        return
    }
    // Sorted lists name the letter they are at; the others have nothing to name.
    val label: ((Int) -> String?)? = when (kind) {
        AlbumListKind.ByName -> { i -> initial(albums.getOrNull(i)?.name) }
        AlbumListKind.ByArtist -> { i -> initial(albums.getOrNull(i)?.artist) }
        else -> null
    }
    FastScroll(state, label = label) {
        LazyVerticalGrid(GridCells.Adaptive(150.dp), state = state, modifier = Modifier.padding(horizontal = 8.dp)) {
            items(albums, key = { it.id }) { a ->
                Tile(a.name, a.artist, a.coverArt) { nav.navigate(Routes.album(a.id)) }
            }
            if (loading) item(span = { GridItemSpan(maxLineSpan) }) { Text("…", Modifier.padding(16.dp)) }
        }
    }
}

@Composable
fun ArtistsScreen() {
    val nav = LocalNav.current
    Screen(stringResource(R.string.artists), back = false, actions = {
        IconButton(onClick = { nav.navigate(Routes.folder(null)) }) { Icon(Icons.Default.Folder, stringResource(R.string.folders)) }
    }) {
        val list = rememberLazyListState()
        ScrollToTopOnReselect(Routes.ARTISTS) { list.animateScrollToItem(0) }
        Loader(cache = "artists", load = { Graph.core.artists() }) { groups, _ ->
            // The index group of each row (header and artists), for the scroller bubble.
            val letters = remember(groups) { groups.flatMap { g -> List(g.artists.size + 1) { g.name } } }
            FastScroll(list, label = { letters.getOrNull(it) }) {
                LazyColumn(state = list) {
                    groups.forEach { g ->
                        item(key = "h" + g.name) { SectionTitle(g.name) }
                        items(g.artists, key = { it.id }) { a ->
                            ListItem(
                                modifier = Modifier.clickable { nav.navigate(Routes.artist(a.id)) },
                                leadingContent = { Cover(a.coverArt, Modifier.size(48.dp), size = 96, corner = 24.dp) },
                                headlineContent = { Text(a.name) },
                                supportingContent = a.albumCount?.let { n -> { Text(pluralAlbums(n.toInt())) } },
                            )
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun pluralAlbums(n: Int) = androidx.compose.ui.res.pluralStringResource(R.plurals.albums_count, n, n)

@Composable
fun ArtistScreen(id: String) {
    val nav = LocalNav.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    Loader(id, cache = "artist/$id", load = { Graph.core.artist(id) }) { artist, _ ->
        Screen(artist.name, actions = {
            IconButton(onClick = { Downloads.enqueue(context, listOf("artist:${artist.id}")) }) {
                Icon(Icons.Default.Download, stringResource(R.string.download))
            }
        }) {
            LazyVerticalGrid(GridCells.Adaptive(150.dp), modifier = Modifier.padding(horizontal = 8.dp)) {
                item(span = { GridItemSpan(maxLineSpan) }) {
                    Row(Modifier.padding(8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        FilledTonalButton(onClick = {
                            scope.launch { runCatching { Graph.core.artistSongs(id) }.onSuccess { Graph.player.play(it) }.onFailure { context.toast(it) } }
                        }) { Icon(Icons.Default.PlayArrow, null); Text(stringResource(R.string.play)) }
                        FilledTonalButton(onClick = {
                            scope.launch { runCatching { Graph.core.artistSongs(id) }.onSuccess { Graph.player.playShuffled(it) }.onFailure { context.toast(it) } }
                        }) { Icon(Icons.Default.Shuffle, null); Text(stringResource(R.string.shuffle)) }
                        FilledTonalButton(onClick = {
                            scope.launch { runCatching { Graph.core.artistRadio(id, 50u) }.onSuccess { Graph.player.play(it) }.onFailure { context.toast(it) } }
                        }) { Icon(Icons.Default.Radio, null); Text(stringResource(R.string.radio)) }
                    }
                }
                items(artist.albums, key = { it.id }) { a ->
                    Tile(a.name, a.year?.toString(), a.coverArt) { nav.navigate(Routes.album(a.id)) }
                }
                item(span = { GridItemSpan(maxLineSpan) }) { TopSongs(artist.name) }
                item(span = { GridItemSpan(maxLineSpan) }) { ArtistBio(id) }
            }
        }
    }
}

@Composable
private fun TopSongs(name: String) {
    var songs by remember(name) { mutableStateOf<List<Song>>(emptyList()) }
    LaunchedEffect(name) { songs = runCatching { Graph.core.topSongs(name, 10u) }.getOrDefault(emptyList()) }
    if (songs.isEmpty()) return
    Column {
        SectionTitle(stringResource(R.string.top_songs))
        songs.forEachIndexed { i, s -> SongRow(s, onClick = { Graph.player.play(songs, i) }) }
    }
}

@Composable
private fun ArtistBio(id: String) {
    var bio by remember(id) { mutableStateOf<String?>(null) }
    LaunchedEffect(id) { bio = runCatching { Graph.core.artistInfo(id).biography }.getOrNull() }
    val text = bio ?: return
    Column(Modifier.padding(8.dp)) {
        SectionTitle(stringResource(R.string.biography))
        Text(
            androidx.core.text.HtmlCompat.fromHtml(text, androidx.core.text.HtmlCompat.FROM_HTML_MODE_COMPACT).toString(),
            style = MaterialTheme.typography.bodyMedium,
            modifier = Modifier.padding(horizontal = 8.dp),
        )
    }
}

@Composable
fun AlbumScreen(id: String) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val nav = LocalNav.current
    Loader(id, cache = "album/$id", load = { Graph.core.album(id) }) { album, reload ->
        var starred by remember(album) { mutableStateOf(album.starred) }
        Screen(album.name, actions = {
            IconButton(onClick = {
                scope.launch {
                    runCatching { Graph.core.setStarred(ItemKind.ALBUM, album.id, !starred).also { Graph.pages.stale() } }
                        .onSuccess { starred = !starred }.onFailure { context.toast(it) }
                }
            }) { Icon(if (starred) Icons.Default.Favorite else Icons.Default.FavoriteBorder, stringResource(R.string.star)) }
            IconButton(onClick = { Downloads.enqueue(context, listOf("album:${album.id}")) }) {
                Icon(Icons.Default.Download, stringResource(R.string.download))
            }
            IconButton(onClick = { share(context, scope, listOf(album.id), album.name) }) {
                Icon(Icons.Default.Share, stringResource(R.string.share))
            }
        }) {
            val multiDisc = album.songs.mapNotNull { it.disc }.distinct().size > 1
            LazyColumn {
                item {
                    Row(Modifier.padding(16.dp), verticalAlignment = Alignment.CenterVertically) {
                        Cover(album.coverArt, Modifier.size(128.dp), size = 512)
                        Column(Modifier.padding(start = 16.dp)) {
                            Text(album.name, style = MaterialTheme.typography.titleLarge)
                            album.artist?.let { a ->
                                Text(
                                    a,
                                    color = MaterialTheme.colorScheme.primary,
                                    modifier = Modifier.clickable(enabled = album.artistId != null) { album.artistId?.let { nav.navigate(Routes.artist(it)) } },
                                )
                            }
                            Text(
                                listOfNotNull(album.year?.toString(), album.genre, formatTime(album.duration?.toDouble())).joinToString(" · "),
                                style = MaterialTheme.typography.bodySmall,
                            )
                        }
                    }
                    PlayButtons(album.songs)
                }
                itemsIndexed(album.songs, key = { _, s -> s.id }) { i, s ->
                    val n = listOfNotNull(s.disc?.takeIf { multiDisc }, s.track).joinToString(".")
                    SongRow(s, onClick = { Graph.player.play(album.songs, i) }, leading = n.ifEmpty { "${i + 1}" })
                }
            }
        }
    }
}

@Composable
fun PlayButtons(songs: List<Song>) {
    Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        FilledTonalButton(onClick = { Graph.player.play(songs) }) { Icon(Icons.Default.PlayArrow, null); Text(stringResource(R.string.play)) }
        FilledTonalButton(onClick = { Graph.player.playShuffled(songs) }) { Icon(Icons.Default.Shuffle, null); Text(stringResource(R.string.shuffle)) }
        FilledTonalButton(onClick = { Graph.player.enqueue(songs) }) {
            Icon(Icons.AutoMirrored.Filled.PlaylistAdd, null)
            Text(stringResource(R.string.queue_short), maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
    }
}

@Composable
fun GenresScreen() {
    val nav = LocalNav.current
    Screen(stringResource(R.string.genres)) {
        Loader(cache = "genres", load = { Graph.core.genres() }) { genres, _ ->
            if (genres.isEmpty()) Empty(stringResource(R.string.nothing_here))
            val list = rememberLazyListState()
            FastScroll(list, label = { initial(genres.getOrNull(it)?.name) }) {
                LazyColumn(state = list) {
                    items(genres, key = { it.name }) { g ->
                        ListItem(
                            modifier = Modifier.clickable { nav.navigate(Routes.genre(g.name)) },
                            headlineContent = { Text(g.name) },
                            supportingContent = g.albumCount?.let { n -> { Text(pluralAlbums(n.toInt())) } },
                        )
                    }
                }
            }
        }
    }
}

@Composable
fun GenreScreen(name: String) {
    val nav = LocalNav.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    Screen(name, actions = {
        IconButton(onClick = {
            scope.launch {
                runCatching { Graph.core.songsByGenre(name, 200u, 0u) }.onSuccess { Graph.player.playShuffled(it) }.onFailure { context.toast(it) }
            }
        }) { Icon(Icons.Default.Shuffle, stringResource(R.string.shuffle)) }
    }) {
        Loader(name, cache = "genre/$name", load = { Graph.core.albumList(AlbumListKind.ByGenre(name), 200u, 0u) }) { albums, _ ->
            val grid = rememberLazyGridState()
            FastScroll(grid) {
                LazyVerticalGrid(GridCells.Adaptive(150.dp), state = grid, modifier = Modifier.padding(horizontal = 8.dp)) {
                    items(albums, key = { it.id }) { a -> Tile(a.name, a.artist, a.coverArt) { nav.navigate(Routes.album(a.id)) } }
                }
            }
        }
    }
}

/** File view (`getIndexes` / `getMusicDirectory`). */
@Composable
fun FolderScreen(id: String?) {
    val nav = LocalNav.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    Loader(id, cache = "folder/$id", load = { Graph.core.folder(id) }) { folder, _ ->
        Screen(folder.name.ifEmpty { stringResource(R.string.folders) }, actions = {
            if (id != null) {
                IconButton(onClick = {
                    scope.launch { runCatching { Graph.core.folderSongs(id) }.onSuccess { Graph.player.play(it) }.onFailure { context.toast(it) } }
                }) { Icon(Icons.Default.PlayArrow, stringResource(R.string.play_all)) }
                IconButton(onClick = { Downloads.enqueue(context, listOf("folder:$id")) }) { Icon(Icons.Default.Download, stringResource(R.string.download)) }
            }
        }) {
            val list = rememberLazyListState()
            // Folders are sorted by name; songs come in their own order and get no letter.
            FastScroll(list, label = { initial(folder.folders.getOrNull(it)?.name) }) {
                LazyColumn(state = list) {
                    items(folder.folders, key = { "d" + it.id }) { f ->
                        ListItem(
                            modifier = Modifier.clickable { nav.navigate(Routes.folder(f.id)) },
                            leadingContent = { Cover(f.coverArt, Modifier.size(48.dp), size = 96, corner = 4.dp) },
                            headlineContent = { Text(f.name) },
                        )
                    }
                    itemsIndexed(folder.songs, key = { _, s -> "s" + s.id }) { i, s ->
                        SongRow(s, onClick = { Graph.player.play(folder.songs, i) })
                    }
                }
            }
        }
    }
}
