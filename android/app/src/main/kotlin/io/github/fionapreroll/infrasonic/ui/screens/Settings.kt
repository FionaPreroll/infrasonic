// SPDX-License-Identifier: GPL-3.0-only
@file:OptIn(ExperimentalMaterial3Api::class)

package io.github.fionapreroll.infrasonic.ui.screens

import android.os.Build
import android.text.format.Formatter
import androidx.appcompat.app.AppCompatDelegate
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Slider
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
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
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.core.os.LocaleListCompat
import io.github.fionapreroll.infrasonic.BuildConfig
import io.github.fionapreroll.infrasonic.update.AppUpdates
import io.github.fionapreroll.infrasonic.update.UpdateState
import io.github.fionapreroll.infrasonic.R
import io.github.fionapreroll.infrasonic.core.PlaybackSettings
import io.github.fionapreroll.infrasonic.core.ReplayGainMode
import io.github.fionapreroll.infrasonic.core.coreVersion
import io.github.fionapreroll.infrasonic.data.Graph
import io.github.fionapreroll.infrasonic.data.ThemeMode
import io.github.fionapreroll.infrasonic.downloads.FavoriteDownloads
import io.github.fionapreroll.infrasonic.ui.LocalNav
import io.github.fionapreroll.infrasonic.ui.Routes
import io.github.fionapreroll.infrasonic.ui.Screen
import io.github.fionapreroll.infrasonic.ui.components.ConfirmDialog
import io.github.fionapreroll.infrasonic.ui.components.SectionTitle
import io.github.fionapreroll.infrasonic.visualizer.BundleState
import io.github.fionapreroll.infrasonic.visualizer.Presets
import kotlinx.coroutines.launch

private val bitRates = listOf(0, 320, 256, 192, 160, 128, 96, 64)
private val formats = listOf("opus", "mp3", "aac", "")
private val cacheSizes = listOf(512, 1024, 2048, 4096, 8192, 16384)
private val coverCacheSizes = listOf(50, 100, 250, 500, 1024)

@Composable
fun SettingsScreen() {
    val context = LocalContext.current
    val prefs = Graph.prefs
    var core by remember { mutableStateOf(Graph.core.settings()) }
    fun update(f: (PlaybackSettings) -> PlaybackSettings) {
        val next = f(Graph.core.settings())
        Graph.core.setSettings(next)
        core = next
    }
    val theme by prefs.theme.flow.collectAsState()
    val dynamic by prefs.dynamicColor.flow.collectAsState()
    val transitions by prefs.pageTransitions.flow.collectAsState()
    val wifi by prefs.wifiBitRate.flow.collectAsState()
    val mobile by prefs.mobileBitRate.flow.collectAsState()
    val pin by prefs.pinBitRate.flow.collectAsState()
    val format by prefs.format.flow.collectAsState()
    val wifiOnly by prefs.downloadOnWifiOnly.flow.collectAsState()
    val offline by prefs.offlineMode.flow.collectAsState()
    var clearing by remember { mutableStateOf<Clear?>(null) }
    val scope = rememberCoroutineScope()
    val nav = LocalNav.current
    var cacheTick by remember { mutableIntStateOf(0) }

    @Composable
    fun bitRateLabel(kbps: Int) = when {
        kbps < 0 -> stringResource(R.string.same_as_wifi)
        kbps == 0 -> stringResource(R.string.original_quality)
        else -> stringResource(R.string.kbps, kbps)
    }

    Screen(stringResource(R.string.settings)) {
        Column(Modifier.verticalScroll(rememberScrollState())) {
            SectionTitle(stringResource(R.string.appearance))
            Choice(
                stringResource(R.string.theme),
                ThemeMode.entries,
                theme,
                {
                    stringResource(
                        when (it) {
                            ThemeMode.System -> R.string.theme_system
                            ThemeMode.Light -> R.string.theme_light
                            ThemeMode.Dark -> R.string.theme_dark
                            ThemeMode.Black -> R.string.theme_black
                        },
                    )
                },
            ) { prefs.theme.value = it }
            if (Build.VERSION.SDK_INT >= 31) {
                Toggle(stringResource(R.string.dynamic_color), stringResource(R.string.dynamic_color_hint), dynamic) { prefs.dynamicColor.value = it }
            }
            Toggle(stringResource(R.string.page_transitions), stringResource(R.string.page_transitions_hint), transitions) { prefs.pageTransitions.value = it }
            val languages = listOf("", "en", "de")
            val current = AppCompatDelegate.getApplicationLocales().toLanguageTags().substringBefore('-')
            Choice(
                stringResource(R.string.language),
                languages,
                current.takeIf { it in languages } ?: "",
                {
                    when (it) {
                        "en" -> "English"
                        "de" -> "Deutsch"
                        else -> stringResource(R.string.language_system)
                    }
                },
            ) { AppCompatDelegate.setApplicationLocales(LocaleListCompat.forLanguageTags(it)) }

            HorizontalDivider()
            SectionTitle(stringResource(R.string.quality))
            Choice(stringResource(R.string.bit_rate_wifi), bitRates, wifi, { bitRateLabel(it) }) { prefs.wifiBitRate.value = it }
            Choice(stringResource(R.string.bit_rate_mobile), bitRates, mobile, { bitRateLabel(it) }) { prefs.mobileBitRate.value = it }
            Choice(stringResource(R.string.bit_rate_download), listOf(-1) + bitRates, pin, { bitRateLabel(it) }) { prefs.pinBitRate.value = it }
            Choice(
                stringResource(R.string.format),
                formats,
                format,
                { it.ifEmpty { null }?.uppercase() ?: stringResource(R.string.format_server) },
                stringResource(R.string.format_hint),
            ) { prefs.format.value = it }

            HorizontalDivider()
            SectionTitle(stringResource(R.string.playback))
            Choice(
                stringResource(R.string.replay_gain),
                ReplayGainMode.entries,
                core.replayGain,
                {
                    stringResource(
                        when (it) {
                            ReplayGainMode.AUTO -> R.string.replay_gain_auto
                            ReplayGainMode.OFF -> R.string.replay_gain_off
                            ReplayGainMode.TRACK -> R.string.replay_gain_track
                            ReplayGainMode.ALBUM -> R.string.replay_gain_album
                        },
                    )
                },
            ) { m -> update { it.copy(replayGain = m) } }
            Labeled(stringResource(R.string.replay_gain_preamp, "%+.1f".format(core.replayGainPreamp))) {
                var v by remember(core.replayGainPreamp) { mutableStateOf(core.replayGainPreamp) }
                Slider(v, { v = (it * 2).toInt() / 2f }, valueRange = -15f..15f, onValueChangeFinished = { update { s -> s.copy(replayGainPreamp = v) } })
            }
            Toggle(stringResource(R.string.prevent_clipping), null, core.preventClipping) { b -> update { it.copy(preventClipping = b) } }
            Toggle(stringResource(R.string.scrobble), stringResource(R.string.scrobble_hint), core.scrobble) { b -> update { it.copy(scrobble = b) } }
            Choice(
                stringResource(R.string.crossfade),
                listOf(0f, 2f, 4f, 6f, 8f, 10f, 12f),
                core.crossfadeSecs,
                { if (it == 0f) stringResource(R.string.crossfade_off) else stringResource(R.string.seconds, it.toInt()) },
                stringResource(R.string.crossfade_hint),
            ) { s -> update { it.copy(crossfadeSecs = s) } }
            Choice(
                stringResource(R.string.resume_long),
                listOf(0, 5, 10, 15, 30, 60),
                core.resumeAfterMinutes.toInt(),
                { if (it == 0) stringResource(R.string.off) else stringResource(R.string.resume_long_from, it) },
                stringResource(R.string.resume_long_hint),
            ) { m -> update { it.copy(resumeAfterMinutes = m.toUInt()) } }

            HorizontalDivider()
            SectionTitle(stringResource(R.string.offline))
            Toggle(stringResource(R.string.offline_mode), stringResource(R.string.offline_mode_hint), offline) {
                prefs.offlineMode.value = it
                Graph.scope.launch { Graph.servers.connectActive() }
            }
            Toggle(stringResource(R.string.download_wifi_only), null, wifiOnly) { prefs.downloadOnWifiOnly.value = it }
            val autoFavorites by prefs.autoFavorites.flow.collectAsState()
            Toggle(stringResource(R.string.auto_favorites), stringResource(R.string.auto_favorites_hint), autoFavorites) {
                prefs.autoFavorites.value = it
                FavoriteDownloads.schedule(context.applicationContext, now = true)
            }
            Choice(
                stringResource(R.string.preload),
                listOf(0, 1, 2, 3, 5),
                core.preloadSongs.toInt(),
                { if (it == 0) stringResource(R.string.off) else it.toString() },
                stringResource(R.string.preload_hint),
            ) { n -> update { it.copy(preloadSongs = n.toUInt()) } }

            HorizontalDivider()
            SectionTitle(stringResource(R.string.storage))
            val usage = remember(cacheTick) { Graph.core.cacheUsage() }
            fun size(bytes: Long) = Formatter.formatShortFileSize(context, bytes)
            Choice(
                stringResource(R.string.song_cache),
                cacheSizes,
                core.cacheMb.toInt(),
                { size(it * 1024L * 1024L) },
                stringResource(R.string.song_cache_hint, size(usage.cachedBytes.toLong())),
            ) { mb ->
                update { it.copy(cacheMb = mb.toULong()) }
                cacheTick++
            }
            val coverMb by prefs.coverCacheMb.flow.collectAsState()
            val coverBytes = remember(cacheTick, coverMb) { Graph.covers.diskBytes() }
            Choice(
                stringResource(R.string.cover_cache),
                coverCacheSizes,
                coverMb,
                { size(it * 1024L * 1024L) },
                stringResource(R.string.cover_cache_hint, size(coverBytes)),
            ) { mb ->
                prefs.coverCacheMb.value = mb
                scope.launch {
                    Graph.covers.evict()
                    cacheTick++
                }
            }
            ListItem(
                modifier = Modifier.clickable { nav.navigate(Routes.DOWNLOADS) },
                headlineContent = { Text(stringResource(R.string.downloads)) },
                supportingContent = { Text(stringResource(R.string.downloads_usage, size(usage.pinnedBytes.toLong()), usage.pinnedFiles.toInt())) },
            )
            val memory = remember(cacheTick) { Graph.covers.memoryBytes() to Graph.pages.size }
            ListItem(
                headlineContent = { Text(stringResource(R.string.memory_usage)) },
                supportingContent = { Text(stringResource(R.string.memory_usage_value, size(memory.first), memory.second)) },
            )
            ListItem(
                modifier = Modifier.clickable { clearing = Clear.Songs },
                headlineContent = { Text(stringResource(R.string.clear_song_cache)) },
            )
            ListItem(
                modifier = Modifier.clickable { clearing = Clear.Covers },
                headlineContent = { Text(stringResource(R.string.clear_covers)) },
            )

            if (AppUpdates.enabled) {
                val checkUpdates by prefs.checkUpdates.flow.collectAsState()
                val updating by AppUpdates.state.collectAsState()
                HorizontalDivider()
                SectionTitle(stringResource(R.string.updates))
                Toggle(stringResource(R.string.check_updates), stringResource(R.string.check_updates_hint), checkUpdates) {
                    prefs.checkUpdates.value = it
                }
                ListItem(
                    modifier = Modifier.clickable(enabled = updating == UpdateState.Idle) {
                        AppUpdates.check(context.applicationContext, manual = true)
                    },
                    headlineContent = { Text(stringResource(R.string.check_updates_now)) },
                    supportingContent = if (updating == UpdateState.Checking) {
                        { Text(stringResource(R.string.update_checking)) }
                    } else {
                        null
                    },
                )
            }

            HorizontalDivider()
            VisualizerSection()

            HorizontalDivider()
            SectionTitle(stringResource(R.string.about))
            ListItem(
                headlineContent = { Text("infrasonic ${BuildConfig.VERSION_NAME}") },
                supportingContent = { Text(stringResource(R.string.about_text, coreVersion())) },
            )
        }
    }
    when (clearing) {
        Clear.Songs -> ConfirmDialog(stringResource(R.string.clear_song_cache_confirm), { clearing = null }) {
            runCatching { Graph.core.clearCache() }
            cacheTick++
        }
        Clear.Covers -> ConfirmDialog(stringResource(R.string.clear_covers_confirm), { clearing = null }) {
            Graph.covers.clear()
            Graph.pages.clear()
            cacheTick++
        }
        null -> {}
    }
}

private enum class Clear { Songs, Covers }

/** Presets of the visualizer: the full collection on request, and whether to look for newer ones. */
@Composable
private fun VisualizerSection() {
    val context = LocalContext.current.applicationContext
    val prefs = Graph.prefs
    val installed by Presets.installed.collectAsState()
    val count by Presets.count.collectAsState()
    val state by Presets.state.collectAsState()
    val updates by prefs.visualizerUpdates.flow.collectAsState()
    val seconds by prefs.visualizerSeconds.flow.collectAsState()
    var removing by remember { mutableStateOf(false) }
    LaunchedEffect(Unit) { Presets.prepare(context) }
    SectionTitle(stringResource(R.string.visualizer))
    ListItem(
        headlineContent = { Text(stringResource(R.string.visualizer_presets)) },
        supportingContent = {
            Text(
                if (installed > 0) {
                    stringResource(R.string.visualizer_full, pluralStringResource(R.plurals.visualizer_presets_count, count, count))
                } else {
                    pluralStringResource(R.plurals.visualizer_starters, count, count)
                },
            )
        },
    )
    if (installed == 0) {
        ListItem(
            modifier = Modifier.clickable(enabled = state == BundleState.Idle || state == BundleState.Failed) {
                Presets.check(context, manual = true)
            },
            headlineContent = { Text(stringResource(R.string.visualizer_download)) },
            supportingContent = { Text(stringResource(R.string.visualizer_download_hint)) },
        )
    } else {
        ListItem(
            modifier = Modifier.clickable { removing = true },
            headlineContent = { Text(stringResource(R.string.visualizer_remove)) },
        )
    }
    Toggle(stringResource(R.string.visualizer_updates), stringResource(R.string.visualizer_updates_hint), updates) {
        prefs.visualizerUpdates.value = it
        if (it) Presets.check(context, manual = false)
    }
    Choice(
        stringResource(R.string.visualizer_change),
        listOf(15, 30, 60, 120, 0),
        seconds,
        { if (it == 0) stringResource(R.string.visualizer_by_hand) else stringResource(R.string.visualizer_every, it) },
    ) { prefs.visualizerSeconds.value = it }
    if (removing) {
        ConfirmDialog(stringResource(R.string.visualizer_remove), { removing = false }) { Presets.remove(context) }
    }
}

@Composable
private fun Toggle(title: String, hint: String?, value: Boolean, onChange: (Boolean) -> Unit) {
    ListItem(
        modifier = Modifier.clickable { onChange(!value) },
        headlineContent = { Text(title) },
        supportingContent = hint?.let { { Text(it) } },
        trailingContent = { Switch(value, onChange) },
    )
}

@Composable
private fun Labeled(title: String, content: @Composable () -> Unit) {
    Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp)) {
        Text(title, style = MaterialTheme.typography.bodyLarge)
        content()
    }
}

/** A setting with a few fixed values, picked in a dialog. */
@Composable
private fun <T> Choice(
    title: String,
    options: List<T>,
    value: T,
    label: @Composable (T) -> String,
    hint: String? = null,
    onChange: (T) -> Unit,
) {
    var open by remember { mutableStateOf(false) }
    ListItem(
        modifier = Modifier.clickable { open = true },
        headlineContent = { Text(title) },
        supportingContent = {
            Column {
                Text(label(value), color = MaterialTheme.colorScheme.primary)
                if (hint != null) Text(hint)
            }
        },
    )
    if (open) {
        AlertDialog(
            onDismissRequest = { open = false },
            title = { Text(title) },
            text = {
                Column(Modifier.verticalScroll(rememberScrollState())) {
                    options.forEach { o ->
                        Row(
                            Modifier.fillMaxWidth().selectable(o == value) {
                                open = false
                                onChange(o)
                            }.padding(vertical = 8.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            RadioButton(o == value, null)
                            Text(label(o), Modifier.padding(start = 12.dp))
                        }
                    }
                }
            },
            confirmButton = { TextButton(onClick = { open = false }) { Text(stringResource(R.string.cancel)) } },
        )
    }
}
