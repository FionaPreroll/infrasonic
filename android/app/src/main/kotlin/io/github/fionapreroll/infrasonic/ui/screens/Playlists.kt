// SPDX-License-Identifier: GPL-3.0-only
@file:OptIn(ExperimentalMaterial3Api::class)

package io.github.fionapreroll.infrasonic.ui.screens

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Download
import androidx.compose.material.icons.filled.Edit
import androidx.compose.material.icons.filled.Share
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import io.github.fionapreroll.infrasonic.ui.ScrollToTopOnReselect
import io.github.fionapreroll.infrasonic.R
import io.github.fionapreroll.infrasonic.core.SearchResult
import io.github.fionapreroll.infrasonic.data.Graph
import io.github.fionapreroll.infrasonic.downloads.Downloads
import io.github.fionapreroll.infrasonic.ui.LocalNav
import io.github.fionapreroll.infrasonic.ui.Routes
import io.github.fionapreroll.infrasonic.ui.Screen
import io.github.fionapreroll.infrasonic.ui.components.ConfirmDialog
import io.github.fionapreroll.infrasonic.ui.components.Cover
import io.github.fionapreroll.infrasonic.ui.components.Empty
import io.github.fionapreroll.infrasonic.ui.components.FastScroll
import io.github.fionapreroll.infrasonic.ui.components.ErrorBox
import io.github.fionapreroll.infrasonic.ui.components.Loader
import io.github.fionapreroll.infrasonic.ui.components.SectionTitle
import io.github.fionapreroll.infrasonic.ui.components.SongRow
import io.github.fionapreroll.infrasonic.ui.components.TextInputDialog
import io.github.fionapreroll.infrasonic.ui.components.errorText
import io.github.fionapreroll.infrasonic.ui.components.share
import io.github.fionapreroll.infrasonic.ui.toast
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import androidx.compose.foundation.lazy.rememberLazyListState

@Composable
fun PlaylistsScreen() {
    val nav = LocalNav.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var creating by remember { mutableStateOf(false) }
    var tick by remember { mutableIntStateOf(0) }
    Screen(stringResource(R.string.playlists), back = false, actions = {
        IconButton(onClick = { creating = true }) { Icon(Icons.Default.Add, stringResource(R.string.new_playlist)) }
    }) {
        val list = rememberLazyListState()
        ScrollToTopOnReselect(Routes.PLAYLISTS) { list.animateScrollToItem(0) }
        Loader(tick, cache = "playlists", load = { Graph.core.playlists() }) { lists, _ ->
            if (lists.isEmpty()) Empty(stringResource(R.string.no_playlists))
            LazyColumn(state = list) {
                items(lists, key = { it.id }) { p ->
                    ListItem(
                        modifier = Modifier.clickable { nav.navigate(Routes.playlist(p.id)) },
                        leadingContent = { Cover(p.coverArt, Modifier.size(48.dp), size = 96, corner = 4.dp) },
                        headlineContent = { Text(p.name) },
                        supportingContent = {
                            val n = p.songCount?.toInt() ?: 0
                            Text(listOfNotNull(pluralStringResource(R.plurals.songs_count, n, n), p.owner).joinToString(" · "))
                        },
                    )
                }
            }
        }
    }
    if (creating) {
        TextInputDialog(stringResource(R.string.new_playlist), label = stringResource(R.string.name), onDismiss = { creating = false }) { name ->
            creating = false
            scope.launch {
                runCatching { Graph.core.createPlaylist(name, emptyList()) }.onFailure { context.toast(it) }
                tick++
            }
        }
    }
}

@Composable
fun PlaylistScreen(id: String) {
    val nav = LocalNav.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var tick by remember { mutableIntStateOf(0) }
    var renaming by remember { mutableStateOf(false) }
    var deleting by remember { mutableStateOf(false) }
    Loader(id, tick, cache = "playlist/$id", load = { Graph.core.playlist(id) }) { p, _ ->
        Screen(p.name, actions = {
            IconButton(onClick = { Downloads.enqueue(context, listOf("playlist:${p.id}")) }) { Icon(Icons.Default.Download, stringResource(R.string.download)) }
            IconButton(onClick = { share(context, scope, listOf(p.id), p.name) }) { Icon(Icons.Default.Share, stringResource(R.string.share)) }
            IconButton(onClick = { renaming = true }) { Icon(Icons.Default.Edit, stringResource(R.string.rename)) }
            IconButton(onClick = { deleting = true }) { Icon(Icons.Default.Delete, stringResource(R.string.delete)) }
        }) {
            val list = rememberLazyListState()
            FastScroll(list) {
            LazyColumn(state = list) {
                item {
                    p.comment?.takeIf { it.isNotBlank() }?.let { Text(it, Modifier.padding(16.dp), style = MaterialTheme.typography.bodyMedium) }
                    PlayButtons(p.songs)
                }
                itemsIndexed(p.songs, key = { i, s -> "$i-${s.id}" }) { i, s ->
                    SongRow(s, onClick = { Graph.player.play(p.songs, i) }, extraMenu = { close ->
                        DropdownMenuItem(text = { Text(stringResource(R.string.remove_from_playlist)) }, onClick = {
                            close()
                            scope.launch {
                                runCatching { Graph.core.removeFromPlaylist(p.id, listOf(i.toUInt())) }.onFailure { context.toast(it) }
                                tick++
                            }
                        })
                    })
                }
            }
            }
        }
        if (renaming) {
            TextInputDialog(stringResource(R.string.rename), p.name, stringResource(R.string.name), { renaming = false }) { name ->
                renaming = false
                scope.launch {
                    runCatching { Graph.core.renamePlaylist(p.id, name, p.comment) }.onFailure { context.toast(it) }
                    tick++
                }
            }
        }
        if (deleting) {
            ConfirmDialog(stringResource(R.string.delete_playlist, p.name), { deleting = false }) {
                scope.launch {
                    runCatching { Graph.core.deletePlaylist(p.id) }.onSuccess { nav.popBackStack() }.onFailure { context.toast(it) }
                }
            }
        }
    }
}

@Composable
fun SearchScreen() {
    var query by rememberSaveable { mutableStateOf("") }
    var result by remember { mutableStateOf<SearchResult?>(null) }
    var error by remember { mutableStateOf<Throwable?>(null) }
    val nav = LocalNav.current
    LaunchedEffect(query) {
        if (query.isBlank()) {
            result = null
            return@LaunchedEffect
        }
        delay(350)
        runCatching { Graph.core.search(query.trim(), 30u, 0u) }
            .onSuccess { result = it; error = null }
            .onFailure { error = it }
    }
    val focus = remember { FocusRequester() }
    val keyboard = LocalSoftwareKeyboardController.current
    val list = rememberLazyListState()
    ScrollToTopOnReselect(Routes.SEARCH) {
        focus.requestFocus()
        keyboard?.show()
        list.animateScrollToItem(0)
    }
    Screen(stringResource(R.string.search), back = false) {
        OutlinedTextField(
            query,
            { query = it },
            Modifier.fillMaxWidth().padding(horizontal = 16.dp).focusRequester(focus),
            placeholder = { Text(stringResource(R.string.search_hint)) },
            singleLine = true,
        )
        error?.let { ErrorBox(errorText(it)) }
        val r = result ?: return@Screen
        if (r.artists.isEmpty() && r.albums.isEmpty() && r.songs.isEmpty()) {
            Empty(stringResource(R.string.no_results))
            return@Screen
        }
        LazyColumn(state = list) {
            if (r.artists.isNotEmpty()) item { SectionTitle(stringResource(R.string.artists)) }
            items(r.artists, key = { "a" + it.id }) { a ->
                ListItem(
                    modifier = Modifier.clickable { nav.navigate(Routes.artist(a.id)) },
                    leadingContent = { Cover(a.coverArt, Modifier.size(48.dp), size = 96, corner = 24.dp) },
                    headlineContent = { Text(a.name) },
                )
            }
            if (r.albums.isNotEmpty()) item { SectionTitle(stringResource(R.string.albums)) }
            items(r.albums, key = { "b" + it.id }) { a ->
                ListItem(
                    modifier = Modifier.clickable { nav.navigate(Routes.album(a.id)) },
                    leadingContent = { Cover(a.coverArt, Modifier.size(48.dp), size = 96, corner = 4.dp) },
                    headlineContent = { Text(a.name) },
                    supportingContent = a.artist?.let { { Text(it) } },
                )
            }
            if (r.songs.isNotEmpty()) item { SectionTitle(stringResource(R.string.songs)) }
            itemsIndexed(r.songs, key = { _, s -> "s" + s.id }) { i, s -> SongRow(s, onClick = { Graph.player.play(r.songs, i) }) }
            item { HorizontalDivider(Modifier.padding(vertical = 8.dp)) }
        }
    }
}
