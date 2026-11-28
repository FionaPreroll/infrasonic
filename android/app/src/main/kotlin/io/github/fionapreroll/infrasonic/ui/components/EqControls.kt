// SPDX-License-Identifier: GPL-3.0-only
package io.github.fionapreroll.infrasonic.ui.components

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.awaitHorizontalTouchSlopOrCancellation
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.gestures.drag
import androidx.compose.foundation.gestures.horizontalDrag
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.PathEffect
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.clipRect
import androidx.compose.ui.graphics.drawscope.rotate
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.ProgressBarRangeInfo
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.disabled
import androidx.compose.ui.semantics.progressBarRangeInfo
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.setProgress
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.drawText
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import io.github.fionapreroll.infrasonic.playback.EQ_BANDS
import io.github.fionapreroll.infrasonic.playback.EQ_FREQS
import io.github.fionapreroll.infrasonic.playback.EQ_MAX_DB
import io.github.fionapreroll.infrasonic.playback.EqResponse
import java.util.Locale
import kotlin.math.PI
import kotlin.math.abs
import kotlin.math.cos
import kotlin.math.log2
import kotlin.math.pow
import kotlin.math.roundToInt
import kotlin.math.sin

/*
 * The controls of the equalizer screen. All of them leave vertical swipes to the page, so it
 * scrolls wherever it is touched: the graph takes a touch only on a band's point, the sliders
 * and knobs only once the finger moves sideways.
 */

val EQ_BAND_LABELS = listOf("31", "62", "125", "250", "500", "1k", "2k", "4k", "8k", "16k")

/** A level as text: "+4.5 dB", "0 dB", "−3 dB" (to half a dB). */
fun dbText(v: Float): String {
    val r = (v * 2).roundToInt() / 2f
    val s = if (abs(r - r.roundToInt()) < 0.01f) abs(r.roundToInt()).toString() else String.format(Locale.getDefault(), "%.1f", abs(r))
    return when {
        r > 0 -> "+$s dB"
        r < 0 -> "−$s dB"
        else -> "0 dB"
    }
}

private fun halfDb(v: Float) = ((v * 2).roundToInt() / 2f).coerceIn(-EQ_MAX_DB, EQ_MAX_DB)

private const val DISABLED = 0.38f

/** Colors shared by the controls, dimmed while the equalizer is off. */
private class EqColors(val accent: Color, val onAccent: Color, val track: Color, val grid: Color, val text: Color, val container: Color)

@Composable
private fun eqColors(enabled: Boolean): EqColors {
    val c = MaterialTheme.colorScheme
    val a = if (enabled) 1f else DISABLED
    return EqColors(
        accent = if (enabled) c.primary else c.onSurface.copy(alpha = DISABLED),
        onAccent = c.onPrimary,
        track = c.surfaceContainerHighest,
        grid = c.outlineVariant.copy(alpha = 0.7f * a),
        text = c.onSurfaceVariant.copy(alpha = a),
        container = c.surfaceContainer,
    )
}

/** Bounds of the graph's plot inside the canvas. */
private class Plot(val left: Float, val top: Float, val right: Float, val bottom: Float) {
    val width get() = right - left
    fun x(band: Int) = left + (band + 0.5f) / EQ_BANDS * width
    fun xOfFreq(f: Double) = left + ((log2(f / EQ_FREQS[0]) + 0.5) / EQ_BANDS).toFloat() * width
    fun freqOfX(x: Float) = EQ_FREQS[0] * 2.0.pow(((x - left) / width) * EQ_BANDS - 0.5)
    fun y(db: Float) = top + (1f - (db + EQ_MAX_DB) / (2 * EQ_MAX_DB)) * (bottom - top)
    fun db(y: Float) = (1f - (y - top) / (bottom - top)) * 2 * EQ_MAX_DB - EQ_MAX_DB
}

/**
 * The frequency response of [gains] with a point per band. Dragging a point up or down sets its
 * band; touches anywhere else go to the page. [active] is the band being set (also by its
 * slider), [highlight] the bands a tone control moves right now.
 */
@Composable
fun EqGraph(
    gains: List<Float>,
    enabled: Boolean,
    active: Int?,
    highlight: IntRange?,
    onActive: (Int?) -> Unit,
    onGain: (Int, Float) -> Unit,
    description: String,
    modifier: Modifier = Modifier,
) {
    val colors = eqColors(enabled)
    val measurer = rememberTextMeasurer()
    val labelStyle = MaterialTheme.typography.labelSmall
    val response = remember(gains) { EqResponse(gains) }
    val current by rememberUpdatedState(gains)
    val setGain by rememberUpdatedState(onGain)
    val setActive by rememberUpdatedState(onActive)
    var plot by remember { mutableStateOf<Plot?>(null) }

    Canvas(
        modifier
            .fillMaxWidth()
            .height(216.dp)
            .testTag("eq-graph")
            .semantics { contentDescription = description }
            .pointerInput(enabled) {
                if (!enabled) return@pointerInput
                val reach = 26.dp.toPx()
                awaitEachGesture {
                    val down = awaitFirstDown(requireUnconsumed = false)
                    val p = plot ?: return@awaitEachGesture
                    val band = (((down.position.x - p.left) / p.width) * EQ_BANDS).toInt()
                    if (band !in 0 until EQ_BANDS) return@awaitEachGesture
                    val handleY = p.y(current[band])
                    if (abs(down.position.y - handleY) > reach || abs(down.position.x - p.x(band)) > reach * 1.4f) {
                        return@awaitEachGesture
                    }
                    // A point was touched: this gesture is the graph's, not the page's.
                    down.consume()
                    val grab = down.position.y - handleY
                    setActive(band)
                    try {
                        drag(down.id) { change ->
                            change.consume()
                            setGain(band, halfDb(p.db(change.position.y - grab)))
                        }
                    } finally {
                        setActive(null)
                    }
                }
            },
    ) {
        val gutter = 30.dp.toPx()
        val axis = 22.dp.toPx()
        val pad = 14.dp.toPx()
        val p = Plot(gutter, pad, size.width - 6.dp.toPx(), size.height - axis - pad)
        plot = p
        val corner = CornerRadius(16.dp.toPx())

        drawRoundRect(colors.container, topLeft = Offset(p.left - 6.dp.toPx(), 0f), size = Size(p.width + 12.dp.toPx(), size.height - axis), cornerRadius = corner)

        // Columns of the bands being set.
        val colW = p.width / EQ_BANDS
        val hl = highlight ?: active?.let { it..it }
        if (hl != null) {
            drawRoundRect(
                colors.accent.copy(alpha = 0.10f),
                topLeft = Offset(p.left + hl.first * colW + 2.dp.toPx(), 4.dp.toPx()),
                size = Size((hl.last - hl.first + 1) * colW - 4.dp.toPx(), size.height - axis - 8.dp.toPx()),
                cornerRadius = CornerRadius(12.dp.toPx()),
            )
        }

        // Grid: a line every 6 dB, a dotted line at every band.
        val dash = PathEffect.dashPathEffect(floatArrayOf(2.dp.toPx(), 4.dp.toPx()))
        for (i in 0 until EQ_BANDS) {
            drawLine(colors.grid.copy(alpha = colors.grid.alpha * 0.6f), Offset(p.x(i), p.top), Offset(p.x(i), p.bottom), 1.dp.toPx(), pathEffect = dash)
        }
        for (db in listOf(-12f, -6f, 0f, 6f, 12f)) {
            val y = p.y(db)
            drawLine(
                if (db == 0f) colors.grid.copy(alpha = (colors.grid.alpha * 1.6f).coerceAtMost(1f)) else colors.grid,
                Offset(p.left, y),
                Offset(p.right, y),
                if (db == 0f) 1.5.dp.toPx() else 1.dp.toPx(),
            )
            val t = measurer.measure(if (db > 0) "+${db.toInt()}" else if (db < 0) "−${(-db).toInt()}" else "0", labelStyle.copy(color = colors.text))
            drawText(t, topLeft = Offset(gutter - 10.dp.toPx() - t.size.width, y - t.size.height / 2f))
        }
        for (i in 0 until EQ_BANDS) {
            val t = measurer.measure(
                EQ_BAND_LABELS[i],
                labelStyle.copy(color = if (i == active) colors.accent else colors.text, textAlign = TextAlign.Center),
            )
            drawText(t, topLeft = Offset(p.x(i) - t.size.width / 2f, size.height - axis + (axis - t.size.height) / 2f))
        }

        // The curve up to 20 kHz (above, the filters fall towards the Nyquist frequency), with a
        // glow towards the 0 dB line.
        val steps = 160
        val end = p.xOfFreq(20_000.0).coerceAtMost(p.right)
        val curve = Path()
        for (s in 0..steps) {
            val x = p.left + (end - p.left) * s / steps
            val y = p.y(response.db(p.freqOfX(x)).toFloat().coerceIn(-EQ_MAX_DB - 2f, EQ_MAX_DB + 2f))
            if (s == 0) curve.moveTo(x, y) else curve.lineTo(x, y)
        }
        val zero = p.y(0f)
        val fill = Path().apply {
            addPath(curve)
            lineTo(end, zero)
            lineTo(p.left, zero)
            close()
        }
        clipRect(p.left, 0f, p.right, size.height - axis) {
            drawPath(
                fill,
                Brush.verticalGradient(
                    0f to colors.accent.copy(alpha = 0.34f),
                    0.5f to colors.accent.copy(alpha = 0.04f),
                    1f to colors.accent.copy(alpha = 0.34f),
                    startY = p.top,
                    endY = p.bottom,
                ),
            )
            drawPath(curve, colors.accent.copy(alpha = 0.16f), style = Stroke(9.dp.toPx(), cap = StrokeCap.Round, join = StrokeJoin.Round))
            drawPath(curve, colors.accent, style = Stroke(2.5.dp.toPx(), cap = StrokeCap.Round, join = StrokeJoin.Round))
        }

        // The points.
        for (i in 0 until EQ_BANDS) {
            val c = Offset(p.x(i), p.y(gains[i]))
            if (i == active) {
                drawCircle(colors.accent.copy(alpha = 0.18f), 20.dp.toPx(), c)
                drawCircle(colors.container, 11.dp.toPx(), c)
                drawCircle(colors.accent, 9.dp.toPx(), c)
            } else {
                drawCircle(colors.container, 8.dp.toPx(), c)
                drawCircle(colors.accent, 6.dp.toPx(), c)
            }
        }
        active?.let { i -> bubble(dbText(gains[i]), Offset(p.x(i), p.y(gains[i])), colors, measurer, labelStyle, p) }
    }
}

/** The level of the band being dragged, above its point (below it near the top). */
private fun DrawScope.bubble(
    text: String,
    at: Offset,
    colors: EqColors,
    measurer: androidx.compose.ui.text.TextMeasurer,
    style: TextStyle,
    p: Plot,
) {
    val t = measurer.measure(text, style.copy(color = colors.onAccent))
    val w = t.size.width + 16.dp.toPx()
    val h = t.size.height + 8.dp.toPx()
    val gap = 26.dp.toPx()
    val top = if (at.y - gap - h < 0f) at.y + gap else at.y - gap - h
    val left = (at.x - w / 2f).coerceIn(p.left - 4.dp.toPx(), size.width - w)
    drawRoundRect(colors.accent, Offset(left, top), Size(w, h), CornerRadius(h / 2f))
    drawText(t, topLeft = Offset(left + 8.dp.toPx(), top + 4.dp.toPx()))
}

/**
 * A slider from −12 to +12 dB, filled from the middle. It follows the finger only once it moves
 * sideways (vertical swipes scroll the page); a double tap sets 0 dB.
 */
@Composable
fun EqSlider(
    label: String,
    value: Float,
    enabled: Boolean,
    active: Boolean,
    onActive: (Boolean) -> Unit,
    onChange: (Float) -> Unit,
    modifier: Modifier = Modifier,
) {
    val colors = eqColors(enabled)
    val change by rememberUpdatedState(onChange)
    val setActive by rememberUpdatedState(onActive)
    val rowShape = RoundedCornerShape(12.dp)
    Row(
        modifier
            .fillMaxWidth()
            .background(if (active) colors.accent.copy(alpha = 0.10f) else Color.Transparent, rowShape)
            .padding(horizontal = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            label,
            Modifier.width(64.dp),
            style = MaterialTheme.typography.labelLarge,
            color = if (active) colors.accent else MaterialTheme.colorScheme.onSurface.copy(alpha = if (enabled) 1f else DISABLED),
            maxLines = 1,
        )
        val thumb = 9.dp
        Canvas(
            Modifier
                .weight(1f)
                .height(40.dp)
                .testTag("eq-slider-$label")
                .semantics {
                    contentDescription = label
                    stateDescription = dbText(value)
                    progressBarRangeInfo = ProgressBarRangeInfo(value, -EQ_MAX_DB..EQ_MAX_DB, steps = (EQ_MAX_DB * 4).toInt() - 1)
                    if (enabled) {
                        setProgress { v ->
                            change(halfDb(v))
                            true
                        }
                    } else {
                        disabled()
                    }
                }
                .pointerInput(enabled) {
                    if (!enabled) return@pointerInput
                    detectTapGestures(onDoubleTap = { change(0f) })
                }
                .pointerInput(enabled) {
                    if (!enabled) return@pointerInput
                    val inset = thumb.toPx()
                    fun valueAt(x: Float) = halfDb(((x - inset) / (size.width - 2 * inset)) * 2 * EQ_MAX_DB - EQ_MAX_DB)
                    awaitEachGesture {
                        val down = awaitFirstDown(requireUnconsumed = false)
                        val start = awaitHorizontalTouchSlopOrCancellation(down.id) { c, _ -> c.consume() } ?: return@awaitEachGesture
                        setActive(true)
                        try {
                            change(valueAt(start.position.x))
                            horizontalDrag(start.id) { c ->
                                c.consume()
                                change(valueAt(c.position.x))
                            }
                        } finally {
                            setActive(false)
                        }
                    }
                },
        ) {
            val r = thumb.toPx()
            val y = size.height / 2f
            val l = r
            val w = size.width - 2 * r
            fun x(v: Float) = l + (v + EQ_MAX_DB) / (2 * EQ_MAX_DB) * w
            val h = 4.dp.toPx()
            drawLine(colors.track, Offset(l, y), Offset(l + w, y), h, cap = StrokeCap.Round)
            val mid = x(0f)
            drawLine(colors.grid, Offset(mid, y - 8.dp.toPx()), Offset(mid, y + 8.dp.toPx()), 1.5.dp.toPx(), cap = StrokeCap.Round)
            val vx = x(value)
            if (abs(vx - mid) > 0.5f) drawLine(colors.accent, Offset(mid, y), Offset(vx, y), 6.dp.toPx(), cap = StrokeCap.Round)
            if (active) drawCircle(colors.accent.copy(alpha = 0.18f), r * 2.1f, Offset(vx, y))
            drawCircle(colors.container, r + 2.dp.toPx(), Offset(vx, y))
            drawCircle(colors.accent, if (active) r * 1.1f else r, Offset(vx, y))
        }
        Text(
            dbText(value),
            Modifier.width(64.dp),
            style = MaterialTheme.typography.labelMedium,
            color = if (active) colors.accent else colors.text,
            textAlign = TextAlign.End,
            maxLines = 1,
        )
    }
}

/**
 * A rotary knob from −12 to +12 dB with 0 dB at the top. Turned by dragging sideways (or in a
 * circle once it turns; right and up is more), a double tap sets 0 dB.
 */
@Composable
fun EqKnob(
    label: String,
    value: Float,
    enabled: Boolean,
    onActive: (Boolean) -> Unit,
    onTurn: (Float) -> Unit,
    modifier: Modifier = Modifier,
    size: Dp = 88.dp,
) {
    val colors = eqColors(enabled)
    val scheme = MaterialTheme.colorScheme
    val dark = scheme.surface.luminance() < 0.5f
    val bodyLight = if (dark) lerp(scheme.surfaceContainerHighest, Color.White, 0.10f) else scheme.surfaceContainerLowest
    val bodyDark = if (dark) scheme.surfaceContainerLow else lerp(scheme.surfaceContainerHighest, Color.Black, 0.06f)
    val current by rememberUpdatedState(value)
    val turn by rememberUpdatedState(onTurn)
    val setActive by rememberUpdatedState(onActive)
    var active by remember { mutableStateOf(false) }
    var target by remember { mutableFloatStateOf(0f) }
    var sent by remember { mutableFloatStateOf(0f) }

    Column(modifier, horizontalAlignment = Alignment.CenterHorizontally) {
        Canvas(
            Modifier
                .size(size)
                .testTag("eq-knob-$label")
                .semantics {
                    contentDescription = label
                    stateDescription = dbText(value)
                    progressBarRangeInfo = ProgressBarRangeInfo(value, -EQ_MAX_DB..EQ_MAX_DB, steps = (EQ_MAX_DB * 4).toInt() - 1)
                    if (enabled) {
                        setProgress { v ->
                            turn(halfDb(v))
                            true
                        }
                    } else {
                        disabled()
                    }
                }
                .pointerInput(enabled) {
                    if (!enabled) return@pointerInput
                    detectTapGestures(onDoubleTap = { turn(0f) })
                }
                .pointerInput(enabled) {
                    if (!enabled) return@pointerInput
                    // The whole range in about two knob widths.
                    val perPx = 2 * EQ_MAX_DB / (size.toPx() * 2.2f)
                    awaitEachGesture {
                        val down = awaitFirstDown(requireUnconsumed = false)
                        val start = awaitHorizontalTouchSlopOrCancellation(down.id) { c, _ -> c.consume() } ?: return@awaitEachGesture
                        target = current
                        sent = halfDb(current)
                        active = true
                        setActive(true)
                        try {
                            drag(start.id) { c ->
                                val d = c.position - c.previousPosition
                                c.consume()
                                target = (target + (d.x - d.y) * perPx).coerceIn(-EQ_MAX_DB, EQ_MAX_DB)
                                val v = halfDb(target)
                                if (v != sent) {
                                    sent = v
                                    turn(v)
                                }
                            }
                        } finally {
                            active = false
                            setActive(false)
                        }
                    }
                },
        ) {
            val c = center
            val outer = this.size.minDimension / 2f
            val arcW = 5.dp.toPx()
            val arcR = outer - arcW / 2f - 1.dp.toPx()
            val arcTopLeft = Offset(c.x - arcR, c.y - arcR)
            val arcSize = Size(arcR * 2, arcR * 2)
            // Track: 270°, from bottom left to bottom right.
            drawArc(colors.track, 135f, 270f, false, arcTopLeft, arcSize, style = Stroke(arcW, cap = StrokeCap.Round))
            val sweep = value / EQ_MAX_DB * 135f
            if (abs(sweep) > 0.5f) {
                drawArc(colors.accent.copy(alpha = 0.18f), -90f, sweep, false, arcTopLeft, arcSize, style = Stroke(arcW * 2.4f, cap = StrokeCap.Round))
                drawArc(colors.accent, -90f, sweep, false, arcTopLeft, arcSize, style = Stroke(arcW, cap = StrokeCap.Round))
            }
            val body = arcR - arcW - 6.dp.toPx()
            // Shadow.
            for (k in 1..4) {
                drawCircle(Color.Black.copy(alpha = (if (dark) 0.10f else 0.05f)), body + k * 1.dp.toPx(), c.copy(y = c.y + k * 0.8f.dp.toPx()))
            }
            drawCircle(
                Brush.radialGradient(listOf(bodyLight, bodyDark), center = c + Offset(-body * 0.45f, -body * 0.55f), radius = body * 1.8f),
                body,
                c,
            )
            drawCircle(Color.White.copy(alpha = if (dark) 0.10f else 0.6f), body, c, style = Stroke(1.dp.toPx()))
            val angle = -90f + sweep
            rotate(angle + 90f, c) {
                // Grip ridges, turning with the knob.
                for (k in 0 until 32) {
                    val a = (k * 360f / 32f) * PI.toFloat() / 180f
                    val dir = Offset(sin(a), -cos(a))
                    drawLine(
                        Color.Black.copy(alpha = if (dark) 0.30f else 0.10f),
                        c + dir * (body - 4.dp.toPx()),
                        c + dir * (body - 1.dp.toPx()),
                        1.2.dp.toPx(),
                        cap = StrokeCap.Round,
                    )
                }
                // Cap and pointer.
                val cap = body * 0.62f
                drawCircle(
                    Brush.radialGradient(listOf(bodyDark, bodyLight), center = c + Offset(-cap * 0.4f, -cap * 0.5f), radius = cap * 2f),
                    cap,
                    c,
                )
                drawLine(colors.accent, Offset(c.x, c.y - body + 7.dp.toPx()), Offset(c.x, c.y - cap * 0.35f), 3.5.dp.toPx(), cap = StrokeCap.Round)
            }
        }
        Text(label, style = MaterialTheme.typography.labelLarge, color = scheme.onSurface.copy(alpha = if (enabled) 1f else DISABLED))
        Text(
            dbText(value),
            style = MaterialTheme.typography.labelMedium,
            color = if (active) colors.accent else colors.text,
        )
    }
}
