// SPDX-License-Identifier: GPL-3.0-only
@file:OptIn(ExperimentalMaterial3Api::class)

package io.github.fionapreroll.infrasonic.ui.screens

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.QueueMusic
import androidx.compose.material.icons.filled.BookmarkAdd
import androidx.compose.material.icons.filled.ClearAll
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Favorite
import androidx.compose.material.icons.filled.FavoriteBorder
import androidx.compose.material.icons.filled.GraphicEq
import androidx.compose.material.icons.filled.Lyrics
import androidx.compose.material.icons.filled.Pause
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.Repeat
import androidx.compose.material.icons.filled.RepeatOne
import androidx.compose.material.icons.filled.Save
import androidx.compose.material.icons.filled.Shuffle
import androidx.compose.material.icons.filled.SkipNext
import androidx.compose.material.icons.filled.SkipPrevious
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilledIconButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.IconButtonDefaults
import androidx.compose.material3.IconToggleButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Slider
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.github.fionapreroll.infrasonic.downloads.FavoriteDownloads
import io.github.fionapreroll.infrasonic.R
import io.github.fionapreroll.infrasonic.core.ItemKind
import io.github.fionapreroll.infrasonic.core.Lyrics
import io.github.fionapreroll.infrasonic.core.PlayState
import io.github.fionapreroll.infrasonic.core.PlayerCommand
import io.github.fionapreroll.infrasonic.core.Repeat
import io.github.fionapreroll.infrasonic.core.Song
import io.github.fionapreroll.infrasonic.data.Graph
import io.github.fionapreroll.infrasonic.ui.LocalNav
import io.github.fionapreroll.infrasonic.ui.Routes
import io.github.fionapreroll.infrasonic.ui.Screen
import io.github.fionapreroll.infrasonic.ui.components.Cover
import io.github.fionapreroll.infrasonic.ui.components.Empty
import io.github.fionapreroll.infrasonic.ui.components.TextInputDialog
import io.github.fionapreroll.infrasonic.ui.components.formatTime
import io.github.fionapreroll.infrasonic.ui.navBarPadding
import io.github.fionapreroll.infrasonic.ui.toast
import kotlinx.coroutines.launch
import androidx.compose.material.icons.filled.Bedtime
import androidx.compose.material.icons.filled.DragHandle
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.RadioButton
import androidx.compose.material3.TextButton
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.zIndex
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.platform.testTag
import androidx.compose.runtime.mutableFloatStateOf
import io.github.fionapreroll.infrasonic.playback.SPEEDS
import io.github.fionapreroll.infrasonic.playback.SleepMode
import io.github.fionapreroll.infrasonic.playback.speedLabel
import io.github.fionapreroll.infrasonic.ui.currentLyricsLine
import io.github.fionapreroll.infrasonic.ui.dropTarget
import kotlinx.coroutines.delay

private enum class Pane { Cover, Lyrics, Queue }

@Composable
fun NowPlayingScreen() {
    val st by Graph.player.state.collectAsState()
    var pane by rememberSaveable { mutableStateOf(Pane.Cover) }
    val item = st?.current
    val nav = LocalNav.current
    val eq by Graph.equalizer.state.collectAsState()
    Screen(stringResource(R.string.now_playing), actions = {
        IconButton(onClick = { nav.navigate(Routes.EQUALIZER) }) {
            Icon(
                Icons.Default.GraphicEq,
                stringResource(R.string.equalizer),
                tint = if (eq.current.enabled) MaterialTheme.colorScheme.primary else LocalContentColor.current,
            )
        }
        IconToggleButton(pane == Pane.Lyrics, { pane = if (it) Pane.Lyrics else Pane.Cover }) {
            Icon(Icons.Default.Lyrics, stringResource(R.string.lyrics))
        }
        IconToggleButton(pane == Pane.Queue, { pane = if (it) Pane.Queue else Pane.Cover }) {
            Icon(Icons.AutoMirrored.Filled.QueueMusic, stringResource(R.string.queue_short))
        }
    }) {
        Column(Modifier.fillMaxSize().navBarPadding()) {
            Box(Modifier.weight(1f).fillMaxWidth()) {
                when {
                    pane == Pane.Queue -> QueuePane()
                    item == null -> Empty(stringResource(R.string.nothing_playing))
                    pane == Pane.Lyrics -> LyricsPane(item.song)
                    else -> Box(Modifier.fillMaxSize().padding(24.dp), contentAlignment = Alignment.Center) {
                        Cover(item.song.coverArt, Modifier.widthIn(max = 480.dp).fillMaxWidth().aspectRatio(1f), size = 800, corner = 12.dp)
                    }
                }
            }
            if (item != null) Controls(item.song)
        }
    }
}

@Composable
private fun Controls(song: Song) {
    val nav = LocalNav.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val st by Graph.player.state.collectAsState()
    val position by Graph.player.position.collectAsState()
    var dragging by remember { mutableStateOf<Float?>(null) }
    var starred by remember(song.id) { mutableStateOf(song.starred) }
    val duration = st?.duration ?: song.duration?.toDouble() ?: 0.0
    Column(Modifier.fillMaxWidth().padding(horizontal = 24.dp, vertical = 8.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Text(
                    song.title,
                    style = MaterialTheme.typography.titleLarge,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.clickable(enabled = song.albumId != null) { song.albumId?.let { nav.navigate(Routes.album(it)) } },
                )
                Text(
                    song.artist,
                    style = MaterialTheme.typography.bodyLarge,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.clickable(enabled = song.artistId != null) { song.artistId?.let { nav.navigate(Routes.artist(it)) } },
                )
            }
            IconButton(onClick = {
                val pos = Graph.player.positionNow()
                scope.launch {
                    runCatching { Graph.core.createBookmark(song.id, (pos * 1000).toULong(), null).also { Graph.pages.stale() } }
                        .onSuccess { context.toast(R.string.bookmark_saved, formatTime(pos)) }
                        .onFailure { context.toast(it) }
                }
            }) { Icon(Icons.Default.BookmarkAdd, stringResource(R.string.add_bookmark)) }
            IconButton(onClick = {
                val new = !starred
                scope.launch {
                    runCatching { Graph.core.setStarred(ItemKind.SONG, song.id, new).also { Graph.pages.stale() } }
                        .onSuccess {
                            starred = new
                            FavoriteDownloads.syncSoon(context.applicationContext)
                        }
                        .onFailure { context.toast(it) }
                }
            }) {
                Icon(
                    if (starred) Icons.Default.Favorite else Icons.Default.FavoriteBorder,
                    stringResource(if (starred) R.string.unstar else R.string.star),
                    tint = if (starred) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
        Slider(
            value = dragging ?: position.toFloat().coerceIn(0f, duration.toFloat().coerceAtLeast(0f)),
            onValueChange = { dragging = it },
            onValueChangeFinished = {
                dragging?.let { Graph.player.seek(it.toDouble()) }
                dragging = null
            },
            valueRange = 0f..duration.toFloat().coerceAtLeast(1f),
        )
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            Text(formatTime(dragging?.toDouble() ?: position), style = MaterialTheme.typography.bodySmall)
            Box(Modifier.weight(1f))
            SpeedButton()
            SleepButton()
            Box(Modifier.weight(1f))
            Text(formatTime(duration), style = MaterialTheme.typography.bodySmall)
        }
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceEvenly, verticalAlignment = Alignment.CenterVertically) {
            IconButton(onClick = { Graph.player.command(PlayerCommand.Shuffle) }) {
                Icon(Icons.Default.Shuffle, stringResource(R.string.shuffle))
            }
            IconButton(onClick = { Graph.player.previous() }, Modifier.size(56.dp)) {
                Icon(Icons.Default.SkipPrevious, stringResource(R.string.previous), Modifier.size(36.dp))
            }
            val playing = st?.state == PlayState.PLAYING || st?.state == PlayState.BUFFERING
            FilledIconButton(onClick = { Graph.player.togglePause() }, Modifier.size(72.dp)) {
                Icon(
                    if (playing) Icons.Default.Pause else Icons.Default.PlayArrow,
                    stringResource(if (playing) R.string.pause else R.string.play),
                    Modifier.size(40.dp),
                )
            }
            IconButton(onClick = { Graph.player.next() }, Modifier.size(56.dp)) {
                Icon(Icons.Default.SkipNext, stringResource(R.string.next), Modifier.size(36.dp))
            }
            val repeat = st?.repeat ?: Repeat.OFF
            IconButton(
                onClick = {
                    val next = when (repeat) {
                        Repeat.OFF -> Repeat.ALL
                        Repeat.ALL -> Repeat.ONE
                        Repeat.ONE -> Repeat.OFF
                    }
                    Graph.player.command(PlayerCommand.SetRepeat(next))
                },
                colors = if (repeat != Repeat.OFF) IconButtonDefaults.iconButtonColors(contentColor = MaterialTheme.colorScheme.primary) else IconButtonDefaults.iconButtonColors(),
            ) {
                Icon(
                    if (repeat == Repeat.ONE) Icons.Default.RepeatOne else Icons.Default.Repeat,
                    stringResource(
                        when (repeat) {
                            Repeat.OFF -> R.string.repeat_off
                            Repeat.ALL -> R.string.repeat_all
                            Repeat.ONE -> R.string.repeat_one
                        },
                    ),
                )
            }
        }
    }
}

@Composable
private fun LyricsPane(song: Song) {
    // The position between the player's updates, so that lines change on time.
    val ms by produceState(0L, song.id) {
        while (true) {
            value = (Graph.player.positionNow() * 1000).toLong()
            delay(100)
        }
    }
    val lyrics by produceState<Result<Lyrics?>?>(null, song.id) { value = runCatching { Graph.core.lyrics(song) } }
    val l = lyrics?.getOrNull()
    when {
        lyrics == null -> Box(Modifier.fillMaxSize())
        l == null || l.lines.isEmpty() -> Empty(stringResource(R.string.no_lyrics))
        else -> {
            val list = rememberLazyListState()
            val starts = remember(l) { l.lines.map { it.startMs } }
            val current = if (l.synced) currentLyricsLine(starts, ms) else -1
            // The sung line stays a third down the screen.
            LaunchedEffect(current) {
                if (current >= 0) list.animateScrollToItem(current, -list.layoutInfo.viewportSize.height / 3)
            }
            LazyColumn(Modifier.fillMaxSize(), state = list, contentPadding = androidx.compose.foundation.layout.PaddingValues(24.dp)) {
                itemsIndexed(l.lines) { i, line ->
                    val active = i == current
                    Text(
                        line.text.ifBlank { "♪" },
                        Modifier.fillMaxWidth().padding(vertical = 6.dp).clickable(enabled = line.startMs != null) {
                            line.startMs?.let { Graph.player.seek(it / 1000.0) }
                        },
                        style = MaterialTheme.typography.titleMedium,
                        fontWeight = if (active) FontWeight.Bold else FontWeight.Normal,
                        color = when {
                            active || !l.synced -> MaterialTheme.colorScheme.onSurface
                            else -> MaterialTheme.colorScheme.onSurfaceVariant
                        },
                        textAlign = TextAlign.Center,
                    )
                }
            }
        }
    }
}

@Composable
private fun QueuePane() {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val queue by Graph.player.queue.collectAsState()
    val st by Graph.player.state.collectAsState()
    var saving by remember { mutableStateOf(false) }
    val list = rememberLazyListState()
    LaunchedEffect(Unit) { st?.index?.toInt()?.let { if (it > 1) list.scrollToItem(it - 1) } }
    Column(Modifier.fillMaxSize()) {
        Row(Modifier.fillMaxWidth().padding(horizontal = 8.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(
                stringResource(R.string.queue_count, queue.size),
                Modifier.weight(1f).padding(start = 8.dp),
                style = MaterialTheme.typography.titleSmall,
            )
            IconButton(enabled = queue.isNotEmpty(), onClick = { saving = true }) { Icon(Icons.Default.Save, stringResource(R.string.save_as_playlist)) }
            IconButton(enabled = queue.isNotEmpty(), onClick = { Graph.player.command(PlayerCommand.Clear) }) {
                Icon(Icons.Default.ClearAll, stringResource(R.string.clear_queue))
            }
        }
        if (queue.isEmpty()) {
            Empty(stringResource(R.string.queue_empty))
            return@Column
        }
        var dragged by remember { mutableStateOf<Int?>(null) }
        var dragOffset by remember { mutableFloatStateOf(0f) }
        var rowHeight by remember { mutableFloatStateOf(1f) }
        val target = dragged?.let { dropTarget(it, dragOffset, rowHeight, queue.size) }
        LazyColumn(Modifier.fillMaxSize(), state = list) {
            itemsIndexed(queue, key = { _, q -> q.uid.toLong() }) { i, q ->
                val current = st?.index?.toInt() == i
                val d = dragged
                // The dragged row follows the finger; the rows it passes make room.
                val shift = when {
                    d == null || target == null -> 0f
                    i == d -> dragOffset
                    d < target && i in d + 1..target -> -rowHeight
                    target < d && i in target until d -> rowHeight
                    else -> 0f
                }
                val moveUp = stringResource(R.string.move_up)
                val moveDown = stringResource(R.string.move_down)
                ListItem(
                    modifier = Modifier
                        .onSizeChanged { rowHeight = it.height.toFloat() }
                        .zIndex(if (i == d) 1f else 0f)
                        .graphicsLayer {
                            translationY = shift
                            shadowElevation = if (i == d) 8f else 0f
                        }
                        .semantics {
                            customActions = listOfNotNull(
                                CustomAccessibilityAction(moveUp) {
                                    Graph.player.command(PlayerCommand.Move(i.toUInt(), (i - 1).toUInt()))
                                    true
                                }.takeIf { i > 0 },
                                CustomAccessibilityAction(moveDown) {
                                    Graph.player.command(PlayerCommand.Move(i.toUInt(), (i + 1).toUInt()))
                                    true
                                }.takeIf { i < queue.size - 1 },
                            )
                        }
                        .clickable { Graph.player.command(PlayerCommand.PlayIndex(i.toUInt())) },
                    colors = if (current) ListItemDefaults.colors(containerColor = MaterialTheme.colorScheme.secondaryContainer) else ListItemDefaults.colors(),
                    leadingContent = { Cover(q.song.coverArt, Modifier.size(40.dp), size = 96, corner = 4.dp) },
                    headlineContent = {
                        Text(q.song.title, maxLines = 1, overflow = TextOverflow.Ellipsis, fontWeight = if (current) FontWeight.Bold else null)
                    },
                    supportingContent = { Text(q.song.artist, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                    trailingContent = {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            IconButton(onClick = { Graph.player.command(PlayerCommand.Remove(i.toUInt())) }) {
                                Icon(Icons.Default.Close, stringResource(R.string.remove))
                            }
                            Icon(
                                Icons.Default.DragHandle,
                                stringResource(R.string.drag_to_move),
                                Modifier.padding(8.dp).testTag("drag-$i").pointerInput(q.uid) {
                                    detectDragGestures(
                                        onDragStart = {
                                            dragged = queue.indexOfFirst { it.uid == q.uid }
                                            dragOffset = 0f
                                        },
                                        onDragEnd = {
                                            val from = dragged
                                            if (from != null) {
                                                val to = dropTarget(from, dragOffset, rowHeight, queue.size)
                                                if (to != from) Graph.player.command(PlayerCommand.Move(from.toUInt(), to.toUInt()))
                                            }
                                            dragged = null
                                            dragOffset = 0f
                                        },
                                        onDragCancel = {
                                            dragged = null
                                            dragOffset = 0f
                                        },
                                    ) { change, amount ->
                                        change.consume()
                                        dragOffset += amount.y
                                    }
                                },
                            )
                        }
                    },
                )
            }
        }
    }
    if (saving) {
        TextInputDialog(stringResource(R.string.save_as_playlist), label = stringResource(R.string.name), onDismiss = { saving = false }) { name ->
            saving = false
            scope.launch {
                runCatching { Graph.core.saveQueueAsPlaylist(name, null).also { Graph.pages.stale() } }
                    .onSuccess { context.toast(R.string.playlist_saved, name) }
                    .onFailure { context.toast(it) }
            }
        }
    }
}

@Composable
private fun SpeedButton() {
    val speed by Graph.speed.speed.collectAsState()
    val spoken by Graph.speed.spoken.collectAsState()
    var open by remember { mutableStateOf(false) }
    TextButton(onClick = { open = true }) {
        Text(
            speedLabel(speed),
            color = if (speed != 1f) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
    if (open) {
        AlertDialog(
            onDismissRequest = { open = false },
            title = { Text(stringResource(if (spoken) R.string.speed_spoken else R.string.speed_music)) },
            text = {
                Column {
                    SPEEDS.forEach { s ->
                        Row(
                            Modifier.fillMaxWidth().clickable {
                                Graph.speed.set(s)
                                open = false
                            }.padding(vertical = 4.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            RadioButton(selected = s == speed, onClick = null)
                            Text(speedLabel(s), Modifier.padding(start = 12.dp))
                        }
                    }
                }
            },
            confirmButton = { TextButton(onClick = { open = false }) { Text(stringResource(R.string.cancel)) } },
        )
    }
}

private val sleepMinutes = listOf(5, 10, 15, 30, 45, 60, 90)

@Composable
private fun SleepButton() {
    val mode by Graph.sleep.mode.collectAsState()
    var open by remember { mutableStateOf(false) }
    val left by produceState<Long?>(null, mode) {
        while (true) {
            value = Graph.sleep.remaining()
            delay(1000)
        }
    }
    val active = mode != SleepMode.Off
    TextButton(onClick = { open = true }) {
        Icon(
            Icons.Default.Bedtime,
            stringResource(R.string.sleep_timer),
            Modifier.size(18.dp),
            tint = if (active) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant,
        )
        when {
            mode == SleepMode.EndOfSong -> Text(stringResource(R.string.sleep_end_of_song_short), Modifier.padding(start = 4.dp))
            left != null -> Text(formatTime(left!! / 1000.0), Modifier.padding(start = 4.dp))
        }
    }
    if (open) {
        AlertDialog(
            onDismissRequest = { open = false },
            title = { Text(stringResource(R.string.sleep_timer)) },
            text = {
                Column {
                    @Composable
                    fun option(label: String, onClick: () -> Unit) = Text(
                        label,
                        Modifier.fillMaxWidth().clickable {
                            onClick()
                            open = false
                        }.padding(vertical = 10.dp),
                    )
                    sleepMinutes.forEach { m -> option(stringResource(R.string.sleep_minutes, m)) { Graph.sleep.start(m) } }
                    option(stringResource(R.string.sleep_end_of_song)) { Graph.sleep.endOfSong() }
                    if (active) option(stringResource(R.string.sleep_off)) { Graph.sleep.cancel() }
                }
            },
            confirmButton = { TextButton(onClick = { open = false }) { Text(stringResource(R.string.cancel)) } },
        )
    }
}
