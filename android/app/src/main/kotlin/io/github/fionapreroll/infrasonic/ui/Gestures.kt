// SPDX-License-Identifier: GPL-3.0-only
package io.github.fionapreroll.infrasonic.ui

import kotlin.math.abs
import kotlin.math.roundToInt

/**
 * A horizontal swipe on the mini player: +1 for the next song (swiped to the left), -1 for
 * the previous one, 0 if it was too short and too slow. [dx] and [width] in pixels,
 * [velocity] in pixels per second.
 */
fun swipeStep(dx: Float, width: Float, velocity: Float): Int {
    val far = abs(dx) > width * 0.3f
    val fast = abs(velocity) > 1000f && abs(dx) > width * 0.08f
    if (!far && !fast) return 0
    return if (dx < 0) 1 else -1
}

/**
 * Where a queue row dragged from [from] by [offset] pixels lands, with rows [rowHeight]
 * pixels high in a queue of [count].
 */
fun dropTarget(from: Int, offset: Float, rowHeight: Float, count: Int): Int {
    if (count <= 0 || rowHeight <= 0f) return from
    return (from + (offset / rowHeight).roundToInt()).coerceIn(0, count - 1)
}

/** The line of synced lyrics that is sung at [ms], or -1 before the first. */
fun currentLyricsLine(starts: List<Long?>, ms: Long): Int {
    var current = -1
    for ((i, s) in starts.withIndex()) {
        if (s == null) continue
        if (s <= ms) current = i else break
    }
    return current
}

/** Launcher shortcuts (res/xml/shortcuts.xml). */
enum class Shortcut(val action: String) {
    ShuffleAll("io.github.fionapreroll.infrasonic.SHUFFLE_ALL"),
    Favorites("io.github.fionapreroll.infrasonic.PLAY_FAVORITES"),
    Resume("io.github.fionapreroll.infrasonic.RESUME"),
    Search("io.github.fionapreroll.infrasonic.SEARCH"),
    ;

    companion object {
        fun from(action: String?): Shortcut? = entries.firstOrNull { it.action == action }
    }
}
