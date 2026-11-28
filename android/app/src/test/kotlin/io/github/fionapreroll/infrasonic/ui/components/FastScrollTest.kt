// SPDX-License-Identifier: GPL-3.0-only
package io.github.fionapreroll.infrasonic.ui.components

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class FastScrollTest {
    @Test
    fun theThumbFollowsTheList() {
        assertEquals(0f, thumbFraction(ScrollPosition(0f, 10, 110, atEnd = false)), 0f)
        assertEquals(0.5f, thumbFraction(ScrollPosition(50f, 10, 110, atEnd = false)), 1e-6f)
        assertEquals(1f, thumbFraction(ScrollPosition(97f, 13, 110, atEnd = true)), 0f)
        // A partly visible last row must not keep the thumb short of the end.
        assertEquals(1f, thumbFraction(ScrollPosition(120f, 10, 110, atEnd = false)), 0f)
    }

    @Test
    fun aListThatFitsHasNowhereToGo() {
        assertEquals(0f, thumbFraction(ScrollPosition(0f, 10, 8, atEnd = false)), 0f)
        assertEquals(0, jumpIndex(0.7f, 0, 0))
    }

    @Test
    fun draggingJumpsToTheMatchingItem() {
        assertEquals(0, jumpIndex(0f, 110, 10))
        assertEquals(50, jumpIndex(0.5f, 110, 10))
        assertEquals(109, jumpIndex(1f, 110, 10))
        assertEquals(0, jumpIndex(-0.2f, 110, 10))
        // Thumb and jump agree, so a drag that has not moved does not move the list.
        val p = ScrollPosition(37f, 10, 110, atEnd = false)
        assertEquals(37, jumpIndex(thumbFraction(p), p.total, p.visible))
    }

    @Test
    fun onlyLongListsGetAScroller() {
        assertFalse(wantsScroller(25, 10))
        assertTrue(wantsScroller(31, 10))
        assertFalse(wantsScroller(100, 0))
    }

    @Test
    fun initials() {
        assertEquals("A", initial("abba"))
        assertEquals("Ä", initial("  Ärzte"))
        assertEquals("#", initial("2Pac"))
        assertNull(initial(""))
        assertNull(initial(null))
    }
}
