// SPDX-License-Identifier: GPL-3.0-only
package io.github.fionapreroll.infrasonic.ui

import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.swipeUp
import androidx.compose.ui.unit.dp
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.fionapreroll.infrasonic.ui.components.FastScroll
import io.github.fionapreroll.infrasonic.ui.components.initial
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** The fast scroller shows up once the list scrolls; dragging its thumb jumps through the list. */
@RunWith(AndroidJUnit4::class)
class FastScrollTest {
    @get:Rule
    val compose = createComposeRule()

    private val names = ('A'..'Z').flatMap { c -> List(20) { "$c artist $it" } }
    private lateinit var list: LazyListState

    @Before
    fun setUp() {
        compose.setContent {
            list = rememberLazyListState()
            MaterialTheme {
                FastScroll(list, label = { initial(names.getOrNull(it)) }) {
                    LazyColumn(state = list) {
                        items(names) { Text(it, Modifier.padding(16.dp)) }
                    }
                }
            }
        }
    }

    @Test
    fun draggingTheThumbJumps() {
        compose.onNodeWithTag("fast-scroll-thumb").assertDoesNotExist()
        // The thumb hides again 1.5 s after the list stops; the clock must not skip that.
        compose.mainClock.autoAdvance = false
        compose.onNodeWithText("A artist 0").performTouchInput { swipeUp() }
        compose.mainClock.advanceTimeBy(500)
        val before = list.firstVisibleItemIndex
        val thumb = compose.onNodeWithTag("fast-scroll-thumb")
        thumb.performTouchInput {
            down(center)
            moveBy(Offset(0f, 300f))
        }
        compose.mainClock.advanceTimeBy(200)
        // The bubble names the letter of the first row while the thumb is held.
        compose.onNodeWithText(initial(names[list.firstVisibleItemIndex])!!).assertExists()
        thumb.performTouchInput {
            moveBy(Offset(0f, 100_000f))
            up()
        }
        compose.mainClock.advanceTimeBy(500)
        assertTrue("moved from $before to ${list.firstVisibleItemIndex}", list.firstVisibleItemIndex > before + 100)
        assertTrue(!list.canScrollForward)
        compose.mainClock.advanceTimeBy(3000)
        compose.onNodeWithTag("fast-scroll-thumb").assertDoesNotExist()
    }
}
