// SPDX-License-Identifier: GPL-3.0-only
@file:OptIn(ExperimentalMaterial3Api::class)

package io.github.fionapreroll.infrasonic.ui.screens

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.text.format.Formatter
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Label
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Bookmark
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Dns
import androidx.compose.material.icons.filled.Download
import androidx.compose.material.icons.filled.DownloadDone
import androidx.compose.material.icons.filled.Folder
import androidx.compose.material.icons.filled.GraphicEq
import androidx.compose.material.icons.filled.Pause
import androidx.compose.material.icons.filled.Podcasts
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.filled.Share
import androidx.compose.material.icons.filled.Shuffle
import androidx.compose.material.icons.filled.SkipNext
import androidx.compose.material.icons.filled.SkipPrevious
import androidx.compose.material.icons.filled.Speaker
import androidx.compose.material.icons.filled.Sync
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Slider
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.core.text.HtmlCompat
import io.github.fionapreroll.infrasonic.R
import io.github.fionapreroll.infrasonic.core.JukeboxCommand
import io.github.fionapreroll.infrasonic.core.JukeboxState
import io.github.fionapreroll.infrasonic.core.PlayerCommand
import io.github.fionapreroll.infrasonic.core.PodcastEpisode
import io.github.fionapreroll.infrasonic.data.Connection
import io.github.fionapreroll.infrasonic.data.Graph
import io.github.fionapreroll.infrasonic.downloads.Downloads
import io.github.fionapreroll.infrasonic.ui.LocalNav
import io.github.fionapreroll.infrasonic.ui.Routes
import io.github.fionapreroll.infrasonic.ui.Screen
import io.github.fionapreroll.infrasonic.ui.components.ConfirmDialog
import io.github.fionapreroll.infrasonic.ui.components.Cover
import io.github.fionapreroll.infrasonic.ui.components.Empty
import io.github.fionapreroll.infrasonic.ui.components.ErrorBox
import io.github.fionapreroll.infrasonic.ui.components.Loader
import io.github.fionapreroll.infrasonic.ui.components.SongRow
import io.github.fionapreroll.infrasonic.ui.components.TextInputDialog
import io.github.fionapreroll.infrasonic.ui.components.errorText
import io.github.fionapreroll.infrasonic.ui.components.formatTime
import io.github.fionapreroll.infrasonic.ui.toast
import kotlinx.coroutines.launch

@Composable
fun MoreScreen() {
    val nav = LocalNav.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val connection by Graph.servers.connection.collectAsState()
    val online = connection is Connection.Online
    @Composable
    fun entry(icon: ImageVector, label: Int, enabled: Boolean = true, onClick: () -> Unit) {
        ListItem(
            modifier = Modifier.clickable(enabled = enabled, onClick = onClick),
            leadingContent = { Icon(icon, null) },
            headlineContent = {
                Text(
                    stringResource(label),
                    color = if (enabled) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurfaceVariant,
                )
            },
        )
    }
    Screen(stringResource(R.string.more), back = false) {
        LazyColumn {
            item {
                entry(Icons.Default.DownloadDone, R.string.downloads) { nav.navigate(Routes.DOWNLOADS) }
                entry(Icons.AutoMirrored.Filled.Label, R.string.genres, online) { nav.navigate(Routes.GENRES) }
                entry(Icons.Default.Folder, R.string.folders, online) { nav.navigate(Routes.folder(null)) }
                entry(Icons.Default.Podcasts, R.string.podcasts, online) { nav.navigate(Routes.PODCASTS) }
                entry(Icons.Default.Bookmark, R.string.bookmarks, online) { nav.navigate(Routes.BOOKMARKS) }
                entry(Icons.Default.Share, R.string.shares, online) { nav.navigate(Routes.SHARES) }
                entry(Icons.Default.Speaker, R.string.jukebox, online) { nav.navigate(Routes.JUKEBOX) }
                entry(Icons.Default.Sync, R.string.start_scan, online) {
                    scope.launch {
                        runCatching { Graph.core.startScan() }
                            .onSuccess { context.toast(R.string.scan_started) }
                            .onFailure { context.toast(it) }
                    }
                }
                HorizontalDivider()
                entry(Icons.Default.GraphicEq, R.string.equalizer) { nav.navigate(Routes.EQUALIZER) }
                entry(Icons.Default.Dns, R.string.servers) { nav.navigate(Routes.SERVERS) }
                entry(Icons.Default.Settings, R.string.settings) { nav.navigate(Routes.SETTINGS) }
            }
        }
    }
}

// --- podcasts --------------------------------------------------------------------------

@Composable
fun PodcastsScreen() {
    val nav = LocalNav.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var tick by remember { mutableIntStateOf(0) }
    var adding by remember { mutableStateOf(false) }
    Screen(stringResource(R.string.podcasts), actions = {
        IconButton(onClick = {
            scope.launch {
                runCatching { Graph.core.refreshPodcasts() }.onFailure { context.toast(it) }
                tick++
            }
        }) { Icon(Icons.Default.Refresh, stringResource(R.string.refresh)) }
        IconButton(onClick = { adding = true }) { Icon(Icons.Default.Add, stringResource(R.string.add_podcast)) }
    }) {
        Loader(tick, load = { Graph.core.podcasts(false) to runCatching { Graph.core.newestEpisodes(10u) }.getOrDefault(emptyList()) }) { (channels, newest), _ ->
            if (channels.isEmpty()) {
                Empty(stringResource(R.string.no_podcasts))
                return@Loader
            }
            LazyColumn {
                items(channels, key = { it.id }) { c ->
                    ListItem(
                        modifier = Modifier.clickable { nav.navigate(Routes.podcast(c.id)) },
                        leadingContent = { Cover(c.coverArt, Modifier.size(48.dp), size = 96, corner = 4.dp) },
                        headlineContent = { Text(c.title.ifEmpty { c.url }, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                        supportingContent = c.error?.let { { Text(it, color = MaterialTheme.colorScheme.error, maxLines = 2) } },
                    )
                }
                if (newest.isNotEmpty()) {
                    item { io.github.fionapreroll.infrasonic.ui.components.SectionTitle(stringResource(R.string.newest_episodes)) }
                    items(newest, key = { "n" + it.id }) { e -> EpisodeRow(e) { tick++ } }
                }
            }
        }
    }
    if (adding) {
        TextInputDialog(stringResource(R.string.add_podcast), label = stringResource(R.string.feed_url), onDismiss = { adding = false }) { url ->
            adding = false
            scope.launch {
                runCatching { Graph.core.addPodcast(url) }.onFailure { context.toast(it) }
                tick++
            }
        }
    }
}

@Composable
fun PodcastScreen(id: String) {
    val nav = LocalNav.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var tick by remember { mutableIntStateOf(0) }
    var deleting by remember { mutableStateOf(false) }
    Loader(id, tick, load = { Graph.core.podcast(id) }) { c, _ ->
        Screen(c.title, actions = {
            IconButton(onClick = { deleting = true }) { Icon(Icons.Default.Delete, stringResource(R.string.delete)) }
        }) {
            LazyColumn {
                c.description?.takeIf { it.isNotBlank() }?.let { d ->
                    item {
                        Text(
                            HtmlCompat.fromHtml(d, HtmlCompat.FROM_HTML_MODE_COMPACT).toString().trim(),
                            Modifier.padding(16.dp),
                            style = MaterialTheme.typography.bodyMedium,
                            maxLines = 6,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                }
                items(c.episodes, key = { it.id }) { e -> EpisodeRow(e) { tick++ } }
            }
        }
        if (deleting) {
            ConfirmDialog(stringResource(R.string.delete_podcast, c.title), { deleting = false }) {
                scope.launch {
                    runCatching { Graph.core.deletePodcast(c.id) }.onSuccess { nav.popBackStack() }.onFailure { context.toast(it) }
                }
            }
        }
    }
}

@Composable
private fun EpisodeRow(e: PodcastEpisode, changed: () -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val song = e.song
    val ready = e.status == "completed" && song != null
    ListItem(
        modifier = Modifier.clickable(enabled = ready) { song?.let { Graph.player.play(listOf(it)) } },
        headlineContent = { Text(e.title, maxLines = 2, overflow = TextOverflow.Ellipsis) },
        supportingContent = {
            Text(
                listOfNotNull(
                    e.publishDate?.take(10),
                    e.duration?.let { formatTime(it.toDouble()) },
                    if (ready) null else stringResource(R.string.episode_status, e.status),
                ).joinToString(" · "),
            )
        },
        trailingContent = {
            when {
                ready -> IconButton(onClick = { Graph.player.play(listOf(song!!)) }) { Icon(Icons.Default.PlayArrow, stringResource(R.string.play)) }
                e.status == "new" || e.status == "skipped" || e.status == "error" -> IconButton(onClick = {
                    scope.launch {
                        runCatching { Graph.core.downloadEpisode(e.id) }
                            .onSuccess { context.toast(R.string.episode_downloading) }
                            .onFailure { context.toast(it) }
                        changed()
                    }
                }) { Icon(Icons.Default.Download, stringResource(R.string.download_on_server)) }
                else -> {}
            }
        },
    )
}

// --- bookmarks and shares --------------------------------------------------------------

@Composable
fun BookmarksScreen() {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var tick by remember { mutableIntStateOf(0) }
    Screen(stringResource(R.string.bookmarks)) {
        Loader(tick, load = { Graph.core.bookmarks() }) { list, _ ->
            if (list.isEmpty()) {
                Empty(stringResource(R.string.no_bookmarks))
                return@Loader
            }
            LazyColumn {
                items(list, key = { it.song.id }) { b ->
                    val pos = b.positionMs.toDouble() / 1000
                    ListItem(
                        modifier = Modifier.clickable {
                            Graph.player.setQueue(listOf(b.song), 0, pos)
                            Graph.player.command(PlayerCommand.Play)
                        },
                        leadingContent = { Cover(b.song.coverArt, Modifier.size(48.dp), size = 96, corner = 4.dp) },
                        headlineContent = { Text(b.song.title, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                        supportingContent = {
                            Text(listOfNotNull(formatTime(pos), b.song.artist, b.comment).joinToString(" · "), maxLines = 1, overflow = TextOverflow.Ellipsis)
                        },
                        trailingContent = {
                            IconButton(onClick = {
                                scope.launch {
                                    runCatching { Graph.core.deleteBookmark(b.song.id) }.onFailure { context.toast(it) }
                                    tick++
                                }
                            }) { Icon(Icons.Default.Close, stringResource(R.string.remove)) }
                        },
                    )
                }
            }
        }
    }
}

@Composable
fun SharesScreen() {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var tick by remember { mutableIntStateOf(0) }
    Screen(stringResource(R.string.shares)) {
        Loader(tick, load = { Graph.core.shares() }) { list, _ ->
            if (list.isEmpty()) {
                Empty(stringResource(R.string.no_shares))
                return@Loader
            }
            LazyColumn {
                items(list, key = { it.id }) { s ->
                    ListItem(
                        headlineContent = {
                            Text(s.description?.ifBlank { null } ?: s.songs.firstOrNull()?.title ?: s.url, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        },
                        supportingContent = {
                            Column {
                                Text(s.url, maxLines = 1, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.bodySmall)
                                Text(
                                    listOfNotNull(
                                        stringResource(R.string.visits, s.visitCount.toInt()),
                                        s.expires?.take(10)?.let { stringResource(R.string.expires, it) },
                                    ).joinToString(" · "),
                                    style = MaterialTheme.typography.bodySmall,
                                )
                            }
                        },
                        trailingContent = {
                            Row {
                                IconButton(onClick = { copy(context, s.url) }) { Icon(Icons.Default.ContentCopy, stringResource(R.string.copy_link)) }
                                IconButton(onClick = {
                                    scope.launch {
                                        runCatching { Graph.core.deleteShare(s.id) }.onFailure { context.toast(it) }
                                        tick++
                                    }
                                }) { Icon(Icons.Default.Close, stringResource(R.string.remove)) }
                            }
                        },
                    )
                }
            }
        }
    }
}

private fun copy(context: Context, text: String) {
    val cm = context.getSystemService(ClipboardManager::class.java)
    cm.setPrimaryClip(ClipData.newPlainText(text, text))
    // Android 13 and later show their own confirmation.
    if (android.os.Build.VERSION.SDK_INT < 33) context.toast(R.string.copied)
}

// --- downloads -------------------------------------------------------------------------

@Composable
fun DownloadsScreen(back: Boolean = true) {
    val context = LocalContext.current
    val pinnedTick by Graph.player.pinned.collectAsState()
    var tick by remember { mutableIntStateOf(0) }
    var clearing by remember { mutableStateOf(false) }
    val songs = remember(pinnedTick, tick) { runCatching { Graph.core.pinned() }.getOrDefault(emptyList()) }
    val usage = remember(pinnedTick, tick) { Graph.core.cacheUsage() }
    Screen(stringResource(R.string.downloads), back = back, actions = {
        IconButton(enabled = songs.isNotEmpty(), onClick = { Graph.player.playShuffled(songs) }) {
            Icon(Icons.Default.Shuffle, stringResource(R.string.shuffle))
        }
    }) {
        Text(
            stringResource(
                R.string.storage_usage,
                Formatter.formatShortFileSize(context, usage.pinnedBytes.toLong()),
                Formatter.formatShortFileSize(context, usage.cachedBytes.toLong()),
            ),
            Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        if (songs.isEmpty()) {
            Empty(stringResource(R.string.no_downloads))
            return@Screen
        }
        LazyColumn {
            item {
                Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    FilledTonalButton(onClick = { Graph.player.play(songs) }) {
                        Icon(Icons.Default.PlayArrow, null)
                        Text(stringResource(R.string.play_all), Modifier.padding(start = 4.dp))
                    }
                    FilledTonalButton(onClick = { clearing = true }) {
                        Icon(Icons.Default.Delete, null)
                        Text(stringResource(R.string.remove_all_downloads), Modifier.padding(start = 4.dp))
                    }
                }
            }
            itemsIndexed(songs, key = { _, s -> s.id }) { i, s -> SongRow(s, onClick = { Graph.player.play(songs, i) }) }
        }
    }
    if (clearing) {
        ConfirmDialog(stringResource(R.string.remove_all_downloads_confirm), { clearing = false }) {
            Downloads.cancelAll(context)
            runCatching { Graph.core.unpin(songs.map { it.id }) }
            Graph.player.notifyPinned()
            tick++
        }
    }
}

// --- jukebox ---------------------------------------------------------------------------

@Composable
fun JukeboxScreen() {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var state by remember { mutableStateOf<Result<JukeboxState>?>(null) }
    fun send(c: JukeboxCommand) {
        scope.launch {
            val r = runCatching { Graph.core.jukebox(c) }
            if (r.isFailure && state?.isSuccess == true) context.toast(r.exceptionOrNull()!!) else state = r
        }
    }
    LaunchedEffect(Unit) {
        while (true) {
            state = runCatching { Graph.core.jukebox(JukeboxCommand.Get) }
            if (state?.isFailure == true) break
            kotlinx.coroutines.delay(3000)
        }
    }
    Screen(stringResource(R.string.jukebox)) {
        Text(
            stringResource(R.string.jukebox_hint),
            Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        val r = state ?: return@Screen
        val s = r.getOrElse {
            ErrorBox(errorText(it))
            return@Screen
        }
        Row(Modifier.fillMaxWidth().padding(8.dp), horizontalArrangement = Arrangement.SpaceEvenly, verticalAlignment = Alignment.CenterVertically) {
            val cur = s.currentIndex?.toInt() ?: 0
            IconButton(enabled = cur > 0, onClick = { send(JukeboxCommand.Skip((cur - 1).toUInt(), 0u)) }) {
                Icon(Icons.Default.SkipPrevious, stringResource(R.string.previous))
            }
            IconButton(onClick = { send(if (s.playing) JukeboxCommand.Stop else JukeboxCommand.Start) }) {
                Icon(if (s.playing) Icons.Default.Pause else Icons.Default.PlayArrow, stringResource(if (s.playing) R.string.pause else R.string.play))
            }
            IconButton(enabled = cur + 1 < s.songs.size, onClick = { send(JukeboxCommand.Skip((cur + 1).toUInt(), 0u)) }) {
                Icon(Icons.Default.SkipNext, stringResource(R.string.next))
            }
            IconButton(onClick = { send(JukeboxCommand.Shuffle) }) { Icon(Icons.Default.Shuffle, stringResource(R.string.shuffle)) }
            IconButton(onClick = { send(JukeboxCommand.Clear) }) { Icon(Icons.Default.Delete, stringResource(R.string.clear_queue)) }
        }
        var gain by remember(s.gain) { mutableStateOf(s.gain) }
        Slider(gain, { gain = it }, Modifier.padding(horizontal = 24.dp), onValueChangeFinished = { send(JukeboxCommand.SetGain(gain)) })
        FilledTonalButton(
            onClick = {
                val ids = Graph.player.queue.value.map { it.song.id }
                if (ids.isNotEmpty()) send(JukeboxCommand.Set(ids))
            },
            modifier = Modifier.padding(horizontal = 16.dp),
        ) { Text(stringResource(R.string.jukebox_take_queue)) }
        if (s.songs.isEmpty()) {
            Empty(stringResource(R.string.queue_empty))
            return@Screen
        }
        LazyColumn {
            itemsIndexed(s.songs) { i, song ->
                val current = s.currentIndex?.toInt() == i
                ListItem(
                    modifier = Modifier.clickable { send(JukeboxCommand.Skip(i.toUInt(), 0u)) },
                    headlineContent = {
                        Text(song.title, maxLines = 1, overflow = TextOverflow.Ellipsis, color = if (current) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurface)
                    },
                    supportingContent = { Text(song.artist, maxLines = 1) },
                    trailingContent = {
                        IconButton(onClick = { send(JukeboxCommand.Remove(i.toUInt())) }) { Icon(Icons.Default.Close, stringResource(R.string.remove)) }
                    },
                )
            }
        }
    }
}
