// SPDX-License-Identifier: GPL-3.0-only
@file:OptIn(ExperimentalMaterial3Api::class)

package io.github.fionapreroll.infrasonic.ui.components

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Album
import androidx.compose.material.icons.filled.DownloadDone
import androidx.compose.material.icons.filled.Favorite
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import io.github.fionapreroll.infrasonic.R
import io.github.fionapreroll.infrasonic.core.FfiException
import io.github.fionapreroll.infrasonic.core.ItemKind
import io.github.fionapreroll.infrasonic.core.Playlist
import io.github.fionapreroll.infrasonic.core.Song
import io.github.fionapreroll.infrasonic.data.Graph
import io.github.fionapreroll.infrasonic.downloads.Downloads
import io.github.fionapreroll.infrasonic.playback.reason
import io.github.fionapreroll.infrasonic.ui.LocalNav
import io.github.fionapreroll.infrasonic.ui.Routes
import io.github.fionapreroll.infrasonic.ui.toast
import kotlinx.coroutines.launch

fun formatTime(seconds: Double?): String {
    if (seconds == null || seconds.isNaN() || seconds < 0) return "–:––"
    val s = seconds.toLong()
    return if (s >= 3600) "%d:%02d:%02d".format(s / 3600, s % 3600 / 60, s % 60) else "%d:%02d".format(s / 60, s % 60)
}

@Composable
fun errorText(e: Throwable): String = when (e) {
    is FfiException.NotConnected -> stringResource(R.string.not_connected)
    is FfiException.NotSupported -> stringResource(R.string.not_supported)
    is FfiException.Auth -> stringResource(R.string.error_auth)
    is FfiException.Network -> stringResource(R.string.error_network, e.reason)
    is FfiException -> e.reason()
    else -> e.message ?: e.toString()
}

/** Cover art from the server, with a placeholder. */
@Composable
fun Cover(id: String?, modifier: Modifier = Modifier, size: Int = 300, corner: Dp = 8.dp) {
    val bmp by produceState(Graph.covers.cached(id ?: "", size)?.asImageBitmap(), id, size) {
        if (id != null && value == null) value = Graph.covers.bitmap(id, size)?.asImageBitmap()
    }
    val shape = RoundedCornerShape(corner)
    val b = bmp
    if (b != null) {
        Image(b, null, modifier.clip(shape), contentScale = ContentScale.Crop)
    } else {
        Box(modifier.clip(shape).background(MaterialTheme.colorScheme.surfaceContainerHighest), contentAlignment = Alignment.Center) {
            Icon(Icons.Default.Album, null, tint = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}

/** Loads data with a spinner and an error message with "retry". */
@Composable
fun <T> Loader(vararg keys: Any?, load: suspend () -> T, content: @Composable (T, reload: () -> Unit) -> Unit) {
    var attempt by remember { mutableIntStateOf(0) }
    val server by Graph.servers.activeId.collectAsState()
    val result by produceState<Result<T>?>(null, *keys, attempt, server) {
        value = null
        // Right after the start or a server change the session is still connecting.
        Graph.servers.awaitUsable()
        value = runCatching { load() }
    }
    val r = result
    when {
        r == null -> Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { CircularProgressIndicator() }
        r.isFailure -> ErrorBox(errorText(r.exceptionOrNull()!!)) { attempt++ }
        else -> content(r.getOrThrow()) { attempt++ }
    }
}

@Composable
fun ErrorBox(message: String, retry: (() -> Unit)? = null) {
    Column(
        Modifier.fillMaxSize().padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        Text(message, style = MaterialTheme.typography.bodyLarge)
        if (retry != null) {
            Spacer(Modifier.size(16.dp))
            Button(onClick = retry) { Text(stringResource(R.string.retry)) }
        }
    }
}

@Composable
fun Empty(text: String) {
    Box(Modifier.fillMaxSize().padding(24.dp), contentAlignment = Alignment.Center) {
        Text(text, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

@Composable
fun SectionTitle(text: String) {
    Text(
        text,
        style = MaterialTheme.typography.titleMedium,
        modifier = Modifier.padding(start = 16.dp, end = 16.dp, top = 16.dp, bottom = 4.dp),
    )
}

/** A song in a list: tap plays the list from here, the menu has everything else. */
@Composable
fun SongRow(
    song: Song,
    onClick: () -> Unit,
    leading: String? = null,
    showCover: Boolean = leading == null,
    extraMenu: (@Composable (close: () -> Unit) -> Unit)? = null,
) {
    val pinnedTick by Graph.player.pinned.collectAsState()
    val pinned = remember(song.id, pinnedTick) { Graph.core.isPinned(song.id) }
    ListItem(
        modifier = Modifier.clickable(onClick = onClick),
        leadingContent = {
            if (showCover) {
                Cover(song.coverArt, Modifier.size(48.dp), size = 96, corner = 4.dp)
            } else {
                Text(leading ?: "", modifier = Modifier.width(28.dp), color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        },
        headlineContent = { Text(song.title, maxLines = 1, overflow = TextOverflow.Ellipsis) },
        supportingContent = {
            Row(verticalAlignment = Alignment.CenterVertically) {
                if (song.starred) Icon(Icons.Default.Favorite, null, Modifier.size(14.dp).padding(end = 2.dp), tint = MaterialTheme.colorScheme.primary)
                if (pinned) Icon(Icons.Default.DownloadDone, null, Modifier.size(14.dp).padding(end = 2.dp), tint = MaterialTheme.colorScheme.primary)
                Text(
                    listOfNotNull(song.artist.ifEmpty { null }, formatTime(song.duration?.toDouble())).joinToString(" · "),
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        },
        trailingContent = { SongMenu(song, pinned, extraMenu) },
    )
}

@Composable
fun SongMenu(song: Song, pinned: Boolean, extra: (@Composable (close: () -> Unit) -> Unit)? = null) {
    var open by remember { mutableStateOf(false) }
    var addTo by remember { mutableStateOf(false) }
    val nav = LocalNav.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val player = Graph.player
    IconButton(onClick = { open = true }) { Icon(Icons.Default.MoreVert, stringResource(R.string.more)) }
    DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
        val close = { open = false }
        DropdownMenuItem(text = { Text(stringResource(R.string.play_next)) }, onClick = { player.enqueueNext(listOf(song)); close() })
        DropdownMenuItem(text = { Text(stringResource(R.string.add_to_queue)) }, onClick = { player.enqueue(listOf(song)); close() })
        DropdownMenuItem(
            text = { Text(stringResource(R.string.song_radio)) },
            onClick = {
                close()
                scope.launch {
                    runCatching { Graph.core.songRadio(song.id, 50u) }
                        .onSuccess { player.play(listOf(song) + it.filter { s -> s.id != song.id }) }
                        .onFailure { context.toast(it) }
                }
            },
        )
        DropdownMenuItem(
            text = { Text(stringResource(if (song.starred) R.string.unstar else R.string.star)) },
            onClick = {
                close()
                scope.launch { runCatching { Graph.core.setStarred(ItemKind.SONG, song.id, !song.starred) }.onFailure { context.toast(it) } }
            },
        )
        if (pinned) {
            DropdownMenuItem(text = { Text(stringResource(R.string.remove_download)) }, onClick = {
                close()
                runCatching { Graph.core.unpin(listOf(song.id)) }
                Graph.player.notifyPinned()
            })
        } else {
            DropdownMenuItem(text = { Text(stringResource(R.string.download)) }, onClick = { close(); Downloads.songs(context, listOf(song)) })
        }
        DropdownMenuItem(text = { Text(stringResource(R.string.add_to_playlist)) }, onClick = { close(); addTo = true })
        song.albumId?.let { id ->
            DropdownMenuItem(text = { Text(stringResource(R.string.go_to_album)) }, onClick = { close(); nav.navigate(Routes.album(id)) })
        }
        song.artistId?.let { id ->
            DropdownMenuItem(text = { Text(stringResource(R.string.go_to_artist)) }, onClick = { close(); nav.navigate(Routes.artist(id)) })
        }
        DropdownMenuItem(text = { Text(stringResource(R.string.share)) }, onClick = { close(); share(context, scope, listOf(song.id), song.title) })
        extra?.invoke(close)
    }
    if (addTo) AddToPlaylistDialog(listOf(song.id)) { addTo = false }
}

/** Create a share link on the server and hand it to the system share sheet. */
fun share(context: android.content.Context, scope: kotlinx.coroutines.CoroutineScope, ids: List<String>, title: String) {
    scope.launch {
        runCatching { Graph.core.createShare(ids, title, null) }
            .onSuccess { s ->
                val send = android.content.Intent(android.content.Intent.ACTION_SEND)
                    .setType("text/plain")
                    .putExtra(android.content.Intent.EXTRA_TEXT, s.url)
                    .putExtra(android.content.Intent.EXTRA_SUBJECT, title)
                context.startActivity(android.content.Intent.createChooser(send, title))
            }
            .onFailure { context.toast(it) }
    }
}

@Composable
fun AddToPlaylistDialog(songIds: List<String>, onDone: () -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val lists by produceState<List<Playlist>?>(null) { value = runCatching { Graph.core.playlists() }.getOrDefault(emptyList()) }
    var newName by remember { mutableStateOf("") }
    AlertDialog(
        onDismissRequest = onDone,
        title = { Text(stringResource(R.string.add_to_playlist)) },
        text = {
            Column {
                lists?.forEach { p ->
                    Text(
                        p.name,
                        Modifier.fillMaxWidth().clickable {
                            scope.launch {
                                runCatching { Graph.core.addToPlaylist(p.id, songIds) }
                                    .onSuccess { context.toast(R.string.added_to, p.name) }
                                    .onFailure { context.toast(it) }
                                onDone()
                            }
                        }.padding(vertical = 12.dp),
                    )
                } ?: CircularProgressIndicator()
                OutlinedTextField(newName, { newName = it }, label = { Text(stringResource(R.string.new_playlist)) }, singleLine = true)
            }
        },
        confirmButton = {
            TextButton(enabled = newName.isNotBlank(), onClick = {
                scope.launch {
                    runCatching { Graph.core.createPlaylist(newName.trim(), songIds) }.onFailure { context.toast(it) }
                    onDone()
                }
            }) { Text(stringResource(R.string.create)) }
        },
        dismissButton = { TextButton(onClick = onDone) { Text(stringResource(R.string.cancel)) } },
    )
}

/** An album, artist or playlist tile for grids. */
@Composable
fun Tile(title: String, subtitle: String?, cover: String?, onClick: () -> Unit) {
    Column(Modifier.clickable(onClick = onClick).padding(8.dp)) {
        Cover(cover, Modifier.fillMaxWidth().aspectRatio(1f))
        Text(title, maxLines = 1, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.padding(top = 4.dp))
        if (subtitle != null) {
            Text(subtitle, maxLines = 1, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}

@Composable
fun TextInputDialog(title: String, initial: String = "", label: String, onDismiss: () -> Unit, onConfirm: (String) -> Unit) {
    var text by remember { mutableStateOf(initial) }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(title) },
        text = { OutlinedTextField(text, { text = it }, label = { Text(label) }, singleLine = true) },
        confirmButton = { TextButton(enabled = text.isNotBlank(), onClick = { onConfirm(text.trim()) }) { Text(stringResource(R.string.ok)) } },
        dismissButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.cancel)) } },
    )
}

@Composable
fun ConfirmDialog(text: String, onDismiss: () -> Unit, onConfirm: () -> Unit) {
    AlertDialog(
        onDismissRequest = onDismiss,
        text = { Text(text) },
        confirmButton = { TextButton(onClick = { onConfirm(); onDismiss() }) { Text(stringResource(R.string.ok)) } },
        dismissButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.cancel)) } },
    )
}

/** Run once when a value changes, skipping the first composition. */
@Composable
fun OnChange(key: Any?, block: suspend () -> Unit) {
    var first by remember { mutableStateOf(true) }
    LaunchedEffect(key) {
        if (first) first = false else block()
    }
}
