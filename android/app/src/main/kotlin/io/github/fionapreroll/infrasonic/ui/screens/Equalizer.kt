// SPDX-License-Identifier: GPL-3.0-only
package io.github.fionapreroll.infrasonic.ui.screens

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.BluetoothAudio
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Headphones
import androidx.compose.material.icons.filled.RestartAlt
import androidx.compose.material.icons.filled.Save
import androidx.compose.material.icons.filled.Speaker
import androidx.compose.material.icons.filled.Tv
import androidx.compose.material.icons.filled.Usb
import androidx.compose.material3.FilterChip
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.foundation.clickable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import io.github.fionapreroll.infrasonic.R
import io.github.fionapreroll.infrasonic.data.Graph
import io.github.fionapreroll.infrasonic.playback.EqPreset
import io.github.fionapreroll.infrasonic.playback.EqualizerManager
import io.github.fionapreroll.infrasonic.playback.FLAT
import io.github.fionapreroll.infrasonic.playback.OutputDevice
import io.github.fionapreroll.infrasonic.playback.OutputKind
import io.github.fionapreroll.infrasonic.playback.Tone
import io.github.fionapreroll.infrasonic.ui.Screen
import io.github.fionapreroll.infrasonic.ui.components.ConfirmDialog
import io.github.fionapreroll.infrasonic.ui.components.EQ_BAND_LABELS
import io.github.fionapreroll.infrasonic.ui.components.EqGraph
import io.github.fionapreroll.infrasonic.ui.components.EqKnob
import io.github.fionapreroll.infrasonic.ui.components.EqSlider
import io.github.fionapreroll.infrasonic.ui.components.TextInputDialog
import io.github.fionapreroll.infrasonic.ui.components.dbText

@Composable
fun presetName(p: EqPreset): String = if (!p.builtIn) {
    p.name
} else {
    stringResource(
        when (p.id) {
            FLAT -> R.string.eq_flat
            "bass" -> R.string.eq_bass
            "less_bass" -> R.string.eq_less_bass
            "treble" -> R.string.eq_treble
            "loudness" -> R.string.eq_loudness
            "vocal" -> R.string.eq_vocal
            "speech" -> R.string.eq_speech
            "rock" -> R.string.eq_rock
            "pop" -> R.string.eq_pop
            "jazz" -> R.string.eq_jazz
            "classical" -> R.string.eq_classical
            "electronic" -> R.string.eq_electronic
            "acoustic" -> R.string.eq_acoustic
            else -> R.string.eq_custom
        },
    )
}

@Composable
private fun deviceLabel(d: OutputDevice): String = when (d.kind) {
    OutputKind.Speaker -> stringResource(R.string.eq_device_speaker)
    OutputKind.Wired -> stringResource(R.string.eq_device_wired)
    OutputKind.Hdmi -> stringResource(R.string.eq_device_hdmi)
    else -> d.name.ifEmpty { stringResource(R.string.eq_device_other) }
}

private fun deviceIcon(d: OutputDevice): ImageVector = when (d.kind) {
    OutputKind.Speaker -> Icons.Default.Speaker
    OutputKind.Wired -> Icons.Default.Headphones
    OutputKind.Usb -> Icons.Default.Usb
    OutputKind.Bluetooth -> Icons.Default.BluetoothAudio
    OutputKind.Hdmi -> Icons.Default.Tv
    OutputKind.Other -> Icons.Default.Speaker
}


@Composable
private fun toneLabel(t: Tone): String = stringResource(
    when (t) {
        Tone.Bass -> R.string.eq_tone_bass
        Tone.Mid -> R.string.eq_tone_mid
        Tone.Treble -> R.string.eq_tone_treble
    },
)

/** A rounded panel of the equalizer screen, with an optional title. */
@Composable
private fun Panel(title: String?, content: @Composable ColumnScope.() -> Unit) {
    Surface(
        Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 6.dp),
        shape = RoundedCornerShape(24.dp),
        color = MaterialTheme.colorScheme.surfaceContainerLow,
    ) {
        Column(Modifier.padding(vertical = 12.dp)) {
            if (title != null) {
                Text(
                    title,
                    Modifier.padding(start = 20.dp, end = 20.dp, bottom = 8.dp),
                    style = MaterialTheme.typography.titleSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            content()
        }
    }
}

@Composable
fun EqualizerScreen(eq: EqualizerManager = Graph.equalizer) {
    val st by eq.state.collectAsState()
    val cur = st.current
    val gains = cur.levels.gains
    var saving by remember { mutableStateOf(false) }
    var deleting by remember { mutableStateOf<EqPreset?>(null) }
    // What is being set right now, so that the graph and the sliders show it together.
    var activeBand by remember { mutableStateOf<Int?>(null) }
    var activeTone by remember { mutableStateOf<Tone?>(null) }
    // Bands and level of the tone control being turned, when it started.
    var toneStart by remember { mutableStateOf<Pair<List<Float>, Float>?>(null) }
    val preset = st.preset
    val itemColors = ListItemDefaults.colors(containerColor = Color.Transparent)

    Screen(stringResource(R.string.equalizer), actions = {
        IconButton(onClick = { eq.selectPreset(FLAT) }) { Icon(Icons.Default.RestartAlt, stringResource(R.string.eq_reset)) }
        IconButton(onClick = { saving = true }) { Icon(Icons.Default.Save, stringResource(R.string.eq_save_preset)) }
        if (preset != null && !preset.builtIn) {
            IconButton(onClick = { deleting = preset }) { Icon(Icons.Default.Delete, stringResource(R.string.eq_delete_preset)) }
        }
    }) {
        Column(Modifier.verticalScroll(rememberScrollState()).padding(vertical = 6.dp)) {
            Panel(null) {
                ListItem(
                    modifier = Modifier.clickable { eq.setEnabled(!cur.enabled) },
                    colors = itemColors,
                    headlineContent = { Text(stringResource(R.string.equalizer)) },
                    supportingContent = {
                        Text(
                            if (st.perDevice) {
                                stringResource(R.string.eq_for_device, deviceLabel(st.device))
                            } else {
                                stringResource(R.string.eq_for_all)
                            },
                        )
                    },
                    leadingContent = { Icon(deviceIcon(st.device), null) },
                    trailingContent = { Switch(cur.enabled, { eq.setEnabled(it) }) },
                )
                HorizontalDivider(Modifier.padding(horizontal = 20.dp), color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.5f))
                ListItem(
                    modifier = Modifier.clickable { eq.setPerDevice(!st.perDevice) },
                    colors = itemColors,
                    headlineContent = { Text(stringResource(R.string.eq_per_device)) },
                    supportingContent = { Text(stringResource(R.string.eq_per_device_hint)) },
                    trailingContent = { Switch(st.perDevice, { eq.setPerDevice(it) }) },
                )
            }

            Panel(stringResource(R.string.eq_curve)) {
                EqGraph(
                    gains = gains,
                    enabled = cur.enabled,
                    active = activeBand,
                    highlight = activeTone?.bands,
                    onActive = { activeBand = it },
                    onGain = { band, v -> eq.setGain(band, v) },
                    description = stringResource(R.string.eq_curve_description),
                    modifier = Modifier.padding(horizontal = 12.dp),
                )
                // Presets in a row; the selected one (or "Custom") is scrolled into view.
                val chips = remember(st.presets, cur.preset) { (if (cur.preset == null) listOf(null) else emptyList()) + st.presets }
                val row = rememberLazyListState()
                LaunchedEffect(Unit) {
                    val i = chips.indexOfFirst { it?.id == cur.preset }
                    if (i > 0) row.scrollToItem(i, -48)
                }
                LazyRow(
                    Modifier.fillMaxWidth().padding(top = 8.dp),
                    state = row,
                    contentPadding = PaddingValues(horizontal = 16.dp),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    items(chips, key = { it?.id ?: "" }) { p ->
                        if (p == null) {
                            FilterChip(selected = cur.enabled, onClick = { saving = true }, label = { Text(stringResource(R.string.eq_custom)) })
                        } else {
                            FilterChip(
                                selected = cur.enabled && p.id == cur.preset,
                                onClick = { eq.selectPreset(p.id) },
                                label = { Text(presetName(p)) },
                            )
                        }
                    }
                }
            }

            Panel(stringResource(R.string.eq_tone)) {
                Row(Modifier.fillMaxWidth().padding(horizontal = 8.dp), horizontalArrangement = Arrangement.SpaceEvenly) {
                    for (t in Tone.entries) {
                        EqKnob(
                            label = toneLabel(t),
                            value = t.level(gains),
                            enabled = cur.enabled,
                            onActive = { on ->
                                activeTone = if (on) t else null
                                toneStart = if (on) gains to t.level(gains) else null
                            },
                            onTurn = { v ->
                                val (from, level) = toneStart ?: (gains to t.level(gains))
                                eq.setGains(t.turn(from, v - level))
                            },
                        )
                    }
                }
            }

            Panel(stringResource(R.string.eq_bands)) {
                EqSlider(
                    stringResource(R.string.eq_preamp),
                    cur.levels.preamp,
                    cur.enabled,
                    active = false,
                    onActive = {},
                    onChange = { eq.setPreamp(it) },
                    modifier = Modifier.padding(horizontal = 8.dp),
                )
                HorizontalDivider(Modifier.padding(horizontal = 20.dp, vertical = 4.dp), color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.5f))
                gains.forEachIndexed { i, g ->
                    EqSlider(
                        "${EQ_BAND_LABELS[i]} Hz",
                        g,
                        cur.enabled,
                        active = activeBand == i || (activeTone?.let { it.weight(i) > 0f } ?: false),
                        onActive = { on -> activeBand = if (on) i else null },
                        onChange = { eq.setGain(i, it) },
                        modifier = Modifier.padding(horizontal = 8.dp),
                    )
                }
                val headroom = cur.levels.headroom
                if (cur.enabled && headroom < cur.levels.preamp) {
                    Text(
                        stringResource(R.string.eq_headroom, dbText(headroom - cur.levels.preamp)),
                        Modifier.padding(start = 20.dp, end = 20.dp, top = 8.dp),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
            Spacer(Modifier.height(8.dp))
        }
    }

    if (saving) {
        TextInputDialog(
            stringResource(R.string.eq_save_preset),
            preset?.takeIf { !it.builtIn }?.name.orEmpty(),
            stringResource(R.string.name),
            onDismiss = { saving = false },
        ) {
            eq.savePreset(it)
            saving = false
        }
    }
    deleting?.let { p ->
        ConfirmDialog(stringResource(R.string.eq_delete_confirm, p.name), { deleting = null }) {
            eq.deletePreset(p.id)
            deleting = null
        }
    }
}
