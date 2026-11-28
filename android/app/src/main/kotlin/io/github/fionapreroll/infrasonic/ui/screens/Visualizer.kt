// SPDX-License-Identifier: GPL-3.0-only
package io.github.fionapreroll.infrasonic.ui.screens

import android.app.Activity
import android.view.SurfaceView
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.SkipNext
import androidx.compose.material.icons.filled.SkipPrevious
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import io.github.fionapreroll.infrasonic.R
import io.github.fionapreroll.infrasonic.core.VisualizerSettings
import io.github.fionapreroll.infrasonic.data.Graph
import io.github.fionapreroll.infrasonic.ui.LocalNav
import io.github.fionapreroll.infrasonic.visualizer.BundleState
import io.github.fionapreroll.infrasonic.visualizer.Presets
import io.github.fionapreroll.infrasonic.visualizer.VisualizerSurface
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext
import kotlin.math.abs
import kotlin.math.max
import kotlin.math.roundToInt

/** Longest side of the picture in pixels; the display's scaler enlarges it for free. */
private const val MAX_SIDE = 1280

/** Seconds the controls stay after a touch. */
private const val CONTROLS_MS = 4000L

/** The visualizer, full screen: swipe for the next or previous preset, tap for the controls. */
@Composable
fun VisualizerScreen() {
    val nav = LocalNav.current
    val context = LocalContext.current
    val view = LocalView.current
    val seconds by Graph.prefs.visualizerSeconds.flow.collectAsState()
    var preset by remember { mutableStateOf<String?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var controls by remember { mutableStateOf(true) }
    var touched by remember { mutableIntStateOf(0) }

    LaunchedEffect(Unit) { Presets.prepare(context.applicationContext) }
    LaunchedEffect(seconds) {
        withContext(Dispatchers.IO) {
            Graph.core.visualizerSetSettings(VisualizerSettings(30u, seconds.toFloat(), 2.5f))
        }
    }
    LaunchedEffect(Unit) {
        while (true) {
            preset = Graph.core.visualizerPreset()
            error = Graph.core.visualizerError()
            delay(500)
        }
    }
    LaunchedEffect(touched) {
        controls = true
        delay(CONTROLS_MS)
        controls = false
    }
    // full screen, and the screen stays on while the visualizer shows
    DisposableEffect(Unit) {
        val window = (view.context as? Activity)?.window
        val insets = window?.let { WindowCompat.getInsetsController(it, view) }
        insets?.systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
        insets?.hide(WindowInsetsCompat.Type.systemBars())
        view.keepScreenOn = true
        onDispose {
            insets?.show(WindowInsetsCompat.Type.systemBars())
            view.keepScreenOn = false
        }
    }

    Box(Modifier.fillMaxSize().background(Color.Black)) {
        AndroidView(
            factory = { ctx ->
                SurfaceView(ctx).apply {
                    holder.addCallback(VisualizerSurface)
                    addOnLayoutChangeListener { v, l, t, r, b, _, _, _, _ ->
                        val (w, h) = r - l to b - t
                        val scale = MAX_SIDE.toFloat() / max(w, h).coerceAtLeast(1)
                        if (scale < 1f) {
                            (v as SurfaceView).holder.setFixedSize((w * scale).roundToInt(), (h * scale).roundToInt())
                        }
                    }
                }
            },
            modifier = Modifier.fillMaxSize(),
        )
        Box(
            Modifier.fillMaxSize()
                .pointerInput(Unit) { detectTapGestures { touched++ } }
                .pointerInput(Unit) {
                    var dx = 0f
                    detectHorizontalDragGestures(
                        onDragStart = { dx = 0f },
                        onDragEnd = {
                            if (abs(dx) > 80.dp.toPx()) {
                                if (dx < 0) Graph.core.visualizerNext() else Graph.core.visualizerPrevious()
                                touched++
                            }
                        },
                    ) { _, amount -> dx += amount }
                },
        )
        AnimatedVisibility(controls || error != null, enter = fadeIn(), exit = fadeOut()) {
            Column(Modifier.fillMaxSize().safeDrawingPadding().padding(8.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    IconButton(onClick = { nav.popBackStack() }) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, stringResource(R.string.back), tint = Color.White)
                    }
                    Text(
                        preset ?: stringResource(R.string.visualizer),
                        color = Color.White,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
                error?.let {
                    Text(stringResource(R.string.visualizer_unavailable, it), color = Color.White, modifier = Modifier.padding(16.dp))
                }
                Spacer(Modifier.weight(1f))
                Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                    IconButton(onClick = { Graph.core.visualizerPrevious(); touched++ }) {
                        Icon(Icons.Default.SkipPrevious, stringResource(R.string.previous), tint = Color.White)
                    }
                    Text(
                        stringResource(R.string.visualizer_hint),
                        color = Color.White.copy(alpha = 0.7f),
                        modifier = Modifier.weight(1f).padding(horizontal = 8.dp),
                    )
                    IconButton(onClick = { Graph.core.visualizerNext(); touched++ }) {
                        Icon(Icons.Default.SkipNext, stringResource(R.string.next), tint = Color.White)
                    }
                }
            }
        }
    }
}

/** The offer of a preset collection and its download (see [Presets]). */
@Composable
fun PresetBundleDialog() {
    val state by Presets.state.collectAsState()
    val context = LocalContext.current.applicationContext
    when (val s = state) {
        is BundleState.Available -> AlertDialog(
            onDismissRequest = Presets::dismiss,
            title = { Text(stringResource(R.string.visualizer_bundle_title)) },
            text = {
                val presets = pluralStringResource(R.plurals.visualizer_presets_count, s.bundle.presets, s.bundle.presets)
                Text(stringResource(R.string.visualizer_bundle_text, s.bundle.version, presets, s.bundle.megabytes()))
            },
            confirmButton = {
                TextButton({ Presets.download(context, s.bundle) }) { Text(stringResource(R.string.visualizer_download)) }
            },
            dismissButton = { TextButton(Presets::dismiss) { Text(stringResource(R.string.update_later)) } },
        )
        is BundleState.Downloading, BundleState.Installing -> AlertDialog(
            onDismissRequest = {},
            title = { Text(stringResource(R.string.visualizer_bundle_title)) },
            text = {
                Column {
                    if (s is BundleState.Downloading) {
                        LinearProgressIndicator(progress = { s.progress }, modifier = Modifier.fillMaxWidth())
                    } else {
                        Text(stringResource(R.string.visualizer_installing))
                        Spacer(Modifier.height(8.dp))
                        LinearProgressIndicator(modifier = Modifier.fillMaxWidth())
                    }
                }
            },
            confirmButton = {},
            dismissButton = {
                if (s is BundleState.Downloading) TextButton(Presets::dismiss) { Text(stringResource(R.string.cancel)) }
            },
        )
        BundleState.Failed -> AlertDialog(
            onDismissRequest = Presets::dismiss,
            title = { Text(stringResource(R.string.visualizer_bundle_title)) },
            text = { Text(stringResource(R.string.visualizer_bundle_failed)) },
            confirmButton = { TextButton(Presets::dismiss) { Text(stringResource(R.string.ok)) } },
        )
        else -> {}
    }
}
