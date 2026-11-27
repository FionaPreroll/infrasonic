// SPDX-License-Identifier: GPL-3.0-only
@file:OptIn(ExperimentalLayoutApi::class)

package io.github.fionapreroll.infrasonic.ui.screens

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
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
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Slider
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import io.github.fionapreroll.infrasonic.R
import io.github.fionapreroll.infrasonic.data.Graph
import io.github.fionapreroll.infrasonic.playback.EQ_MAX_DB
import io.github.fionapreroll.infrasonic.playback.EqPreset
import io.github.fionapreroll.infrasonic.playback.OutputDevice
import io.github.fionapreroll.infrasonic.playback.OutputKind
import io.github.fionapreroll.infrasonic.playback.FLAT
import io.github.fionapreroll.infrasonic.ui.Screen
import io.github.fionapreroll.infrasonic.ui.components.ConfirmDialog
import io.github.fionapreroll.infrasonic.ui.components.SectionTitle
import io.github.fionapreroll.infrasonic.ui.components.TextInputDialog
import java.util.Locale
import kotlin.math.abs
import kotlin.math.roundToInt

private val bandLabels = listOf("31", "62", "125", "250", "500", "1k", "2k", "4k", "8k", "16k")

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

private fun db(v: Float): String {
    val r = (v * 2).roundToInt() / 2f
    val s = if (abs(r - r.roundToInt()) < 0.01f) r.roundToInt().toString() else String.format(Locale.getDefault(), "%.1f", r)
    return if (r > 0) "+$s dB" else "$s dB"
}

@Composable
fun EqualizerScreen() {
    val eq = Graph.equalizer
    val st by eq.state.collectAsState()
    val cur = st.current
    var saving by remember { mutableStateOf(false) }
    var deleting by remember { mutableStateOf<EqPreset?>(null) }
    val preset = st.preset

    Screen(stringResource(R.string.equalizer), actions = {
        IconButton(onClick = { eq.selectPreset(FLAT) }) { Icon(Icons.Default.RestartAlt, stringResource(R.string.eq_reset)) }
        IconButton(onClick = { saving = true }) { Icon(Icons.Default.Save, stringResource(R.string.eq_save_preset)) }
        if (preset != null && !preset.builtIn) {
            IconButton(onClick = { deleting = preset }) { Icon(Icons.Default.Delete, stringResource(R.string.eq_delete_preset)) }
        }
    }) {
        Column(Modifier.verticalScroll(rememberScrollState())) {
            ListItem(
                modifier = Modifier.clickable { eq.setEnabled(!cur.enabled) },
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
            ListItem(
                modifier = Modifier.clickable { eq.setPerDevice(!st.perDevice) },
                headlineContent = { Text(stringResource(R.string.eq_per_device)) },
                supportingContent = { Text(stringResource(R.string.eq_per_device_hint)) },
                trailingContent = { Switch(st.perDevice, { eq.setPerDevice(it) }) },
            )
            HorizontalDivider()
            SectionTitle(stringResource(R.string.eq_presets))
            FlowRow(
                Modifier.fillMaxWidth().padding(horizontal = 16.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                for (p in st.presets) {
                    FilterChip(
                        selected = cur.enabled && p.id == cur.preset,
                        onClick = { eq.selectPreset(p.id) },
                        label = { Text(presetName(p)) },
                    )
                }
                if (cur.preset == null) {
                    FilterChip(selected = cur.enabled, onClick = { saving = true }, label = { Text(stringResource(R.string.eq_custom)) })
                }
            }
            HorizontalDivider(Modifier.padding(top = 8.dp))
            SectionTitle(stringResource(R.string.eq_bands))
            Band(stringResource(R.string.eq_preamp), cur.levels.preamp, cur.enabled) { eq.setPreamp(it) }
            cur.levels.gains.forEachIndexed { i, g ->
                Band("${bandLabels[i]} Hz", g, cur.enabled) { eq.setGain(i, it) }
            }
            val headroom = cur.levels.headroom
            if (cur.enabled && headroom < cur.levels.preamp) {
                Text(
                    stringResource(R.string.eq_headroom, db(headroom - cur.levels.preamp)),
                    Modifier.padding(16.dp),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
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

@Composable
private fun Band(label: String, value: Float, enabled: Boolean, onChange: (Float) -> Unit) {
    Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp), verticalAlignment = Alignment.CenterVertically) {
        Text(label, Modifier.width(72.dp), style = MaterialTheme.typography.bodyMedium)
        Slider(
            value = value,
            onValueChange = { onChange((it * 2).roundToInt() / 2f) },
            valueRange = -EQ_MAX_DB..EQ_MAX_DB,
            enabled = enabled,
            modifier = Modifier.weight(1f).semantics { contentDescription = label },
        )
        Text(db(value), Modifier.width(64.dp).padding(start = 8.dp), style = MaterialTheme.typography.bodySmall)
    }
}
