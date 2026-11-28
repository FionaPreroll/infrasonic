// SPDX-License-Identifier: GPL-3.0-only
package io.github.fionapreroll.infrasonic.ui.components

import androidx.compose.animation.core.animateDpAsState
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectVerticalDragGestures
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.grid.LazyGridState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.testTag
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.filterNotNull
import kotlin.math.roundToInt

/**
 * Where a lazy list stands, in items: [first] is the index of the first visible item plus the
 * part of it that is scrolled away, [visible] the number of items on screen.
 */
data class ScrollPosition(val first: Float, val visible: Int, val total: Int, val atEnd: Boolean)

/** Where the thumb sits on its track, from 0 (top) to 1 (end of the list). */
fun thumbFraction(p: ScrollPosition): Float {
    if (p.atEnd) return 1f
    val range = p.total - p.visible
    if (range <= 0) return 0f
    return (p.first / range).coerceIn(0f, 1f)
}

/** The item to jump to when the thumb is dragged to [fraction]. */
fun jumpIndex(fraction: Float, total: Int, visible: Int): Int {
    if (total <= 0) return 0
    if (fraction >= 0.999f) return total - 1
    val range = (total - visible).coerceAtLeast(0)
    return (fraction.coerceIn(0f, 1f) * range).roundToInt().coerceIn(0, total - 1)
}

/** Lists of more than two screens get a scroller; for shorter ones a flick is enough. */
fun wantsScroller(total: Int, visible: Int) = visible > 0 && total > visible * 2

/** A list and its fast scroller: what the thumb needs to read and move it. */
private interface Scrollable {
    val position: ScrollPosition
    val scrolling: Boolean
    suspend fun jumpTo(index: Int)
}

private fun LazyListState.scrollable() = object : Scrollable {
    override val position: ScrollPosition
        get() {
            val info = layoutInfo
            val items = info.visibleItemsInfo
            val firstSize = items.firstOrNull()?.size?.coerceAtLeast(1) ?: 1
            return ScrollPosition(
                first = firstVisibleItemIndex + firstVisibleItemScrollOffset.toFloat() / firstSize,
                visible = items.size,
                total = info.totalItemsCount,
                atEnd = !canScrollForward && canScrollBackward,
            )
        }
    override val scrolling get() = isScrollInProgress
    override suspend fun jumpTo(index: Int) = scrollToItem(index)
}

private fun LazyGridState.scrollable() = object : Scrollable {
    override val position: ScrollPosition
        get() {
            val info = layoutInfo
            val items = info.visibleItemsInfo
            val head = items.firstOrNull()
            // The scrolled-away part of the first row counts for all the items in it.
            val perRow = head?.let { h -> items.count { it.row == h.row } } ?: 1
            val height = head?.size?.height?.coerceAtLeast(1) ?: 1
            return ScrollPosition(
                first = firstVisibleItemIndex + firstVisibleItemScrollOffset.toFloat() / height * perRow,
                visible = items.size,
                total = info.totalItemsCount,
                atEnd = !canScrollForward && canScrollBackward,
            )
        }
    override val scrolling get() = isScrollInProgress
    override suspend fun jumpTo(index: Int) = scrollToItem(index)
}

/**
 * A list with a fast scroller on the right edge: the thumb shows up while the list scrolls and can
 * be dragged to anywhere in it. [label] names the item at the top (an initial letter, say); the
 * name is shown beside the thumb while it is dragged.
 */
@Composable
fun FastScroll(state: LazyListState, modifier: Modifier = Modifier, label: ((Int) -> String?)? = null, content: @Composable () -> Unit) {
    val s = remember(state) { state.scrollable() }
    FastScrollBox(s, modifier, label, content)
}

@Composable
fun FastScroll(state: LazyGridState, modifier: Modifier = Modifier, label: ((Int) -> String?)? = null, content: @Composable () -> Unit) {
    val s = remember(state) { state.scrollable() }
    FastScrollBox(s, modifier, label, content)
}

/** The first letter of a name for the scroller bubble; "#" for names that start with a digit or sign. */
fun initial(name: String?): String? {
    val c = name?.trimStart()?.firstOrNull() ?: return null
    return if (c.isLetter()) c.uppercase() else "#"
}

private val TRACK_WIDTH = 24.dp
private val THUMB_HEIGHT = 56.dp

@Composable
private fun FastScrollBox(list: Scrollable, modifier: Modifier, label: ((Int) -> String?)?, content: @Composable () -> Unit) {
    val long by remember(list) { derivedStateOf { list.position.let { wantsScroller(it.total, it.visible) } } }
    var dragging by remember { mutableStateOf(false) }
    // While dragging the thumb follows the finger, not the list, so that it does not jitter.
    var dragFraction by remember { mutableFloatStateOf(0f) }
    // Where the list should go. Letting go keeps it, so that the last jump is not cut short.
    var target by remember { mutableStateOf<Float?>(null) }
    var shown by remember { mutableStateOf(false) }
    val scrolling by remember(list) { derivedStateOf { list.scrolling } }
    LaunchedEffect(scrolling, dragging, long) {
        if (!long) {
            shown = false
        } else if (scrolling || dragging) {
            shown = true
        } else {
            delay(1500)
            shown = false
        }
    }
    LaunchedEffect(list) {
        snapshotFlow { target }.filterNotNull().collectLatest { f ->
            val p = list.position
            list.jumpTo(jumpIndex(f, p.total, p.visible))
        }
    }
    val alpha by animateFloatAsState(if (shown) 1f else 0f, label = "scroller")
    val thumbWidth by animateDpAsState(if (dragging) 8.dp else 4.dp, label = "thumb")
    val labelOf by rememberUpdatedState(label)
    val haptics = LocalHapticFeedback.current

    BoxWithConstraints(modifier.fillMaxSize()) {
        content()
        if (alpha == 0f) return@BoxWithConstraints
        val density = LocalDensity.current
        val track = with(density) { (maxHeight - THUMB_HEIGHT).toPx() }.coerceAtLeast(1f)
        // Read in the layout phase: scrolling moves the thumb without recomposing the list.
        fun top() = ((if (dragging) dragFraction else thumbFraction(list.position)) * track).roundToInt()
        Box(
            Modifier
                .align(Alignment.TopEnd)
                .offset { IntOffset(0, top()) }
                .size(TRACK_WIDTH, THUMB_HEIGHT)
                .alpha(alpha)
                // TalkBack scrolls the list itself; the thumb would only be in the way.
                .clearAndSetSemantics { testTag = "fast-scroll-thumb" }
                .pointerInput(list, track) {
                    var lastLabel: String? = null
                    detectVerticalDragGestures(
                        onDragStart = {
                            dragFraction = thumbFraction(list.position)
                            target = null
                            dragging = true
                        },
                        onDragEnd = { dragging = false },
                        onDragCancel = { dragging = false },
                    ) { change, dy ->
                        change.consume()
                        dragFraction = (dragFraction + dy / track).coerceIn(0f, 1f)
                        target = dragFraction
                        val l = labelOf?.invoke(jumpIndex(dragFraction, list.position.total, list.position.visible))
                        if (l != null && l != lastLabel) {
                            if (lastLabel != null) haptics.performHapticFeedback(HapticFeedbackType.TextHandleMove)
                            lastLabel = l
                        }
                    }
                },
            contentAlignment = Alignment.CenterEnd,
        ) {
            Box(
                Modifier
                    .padding(end = 4.dp)
                    .width(thumbWidth)
                    .fillMaxHeight()
                    .background(
                        if (dragging) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.6f),
                        RoundedCornerShape(4.dp),
                    ),
            )
        }
        val text = if (dragging) labelOf?.invoke(firstIndex(list)) else null
        if (text != null) {
            val bubble = 64.dp
            Box(
                Modifier
                    .align(Alignment.TopEnd)
                    .offset {
                        val y = top() + (THUMB_HEIGHT - bubble).toPx() / 2
                        IntOffset(-(TRACK_WIDTH + 8.dp).roundToPx(), y.roundToInt().coerceAtLeast(0))
                    }
                    .heightIn(min = bubble)
                    .widthIn(min = bubble)
                    .background(
                        MaterialTheme.colorScheme.primaryContainer,
                        RoundedCornerShape(topStart = 32.dp, topEnd = 32.dp, bottomStart = 32.dp, bottomEnd = 4.dp),
                    ),
                contentAlignment = Alignment.Center,
            ) {
                Text(
                    text,
                    // A letter is big; a section name ("Albums") gets room beside it.
                    Modifier.padding(horizontal = if (text.length > 2) 16.dp else 0.dp),
                    style = if (text.length > 2) MaterialTheme.typography.titleMedium else MaterialTheme.typography.headlineMedium,
                    color = MaterialTheme.colorScheme.onPrimaryContainer,
                    textAlign = TextAlign.Center,
                    maxLines = 1,
                )
            }
        }
    }
}

private fun firstIndex(list: Scrollable) = list.position.first.toInt()
