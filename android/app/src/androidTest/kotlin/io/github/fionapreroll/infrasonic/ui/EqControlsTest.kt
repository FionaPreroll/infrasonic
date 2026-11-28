// SPDX-License-Identifier: GPL-3.0-only
package io.github.fionapreroll.infrasonic.ui

import androidx.compose.foundation.ScrollState
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.test.TouchInjectionScope
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.unit.dp
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.fionapreroll.infrasonic.playback.EQ_BANDS
import io.github.fionapreroll.infrasonic.ui.components.EqGraph
import io.github.fionapreroll.infrasonic.ui.components.EqKnob
import io.github.fionapreroll.infrasonic.ui.components.EqSlider
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * The equalizer's controls on a scrolling page: vertical swipes over them scroll the page and
 * change nothing, only their own gestures set a level.
 */
@RunWith(AndroidJUnit4::class)
class EqControlsTest {
    @get:Rule
    val compose = createComposeRule()

    private val gains = mutableStateListOf(*Array(EQ_BANDS) { 0f })
    private var tone by mutableFloatStateOf(0f)
    private lateinit var scroll: ScrollState

    @Before
    fun setUp() {
        compose.setContent {
            scroll = rememberScrollState()
            MaterialTheme {
                Column(Modifier.fillMaxSize().verticalScroll(scroll)) {
                    EqGraph(gains.toList(), true, null, null, {}, { b, v -> gains[b] = v }, "graph")
                    EqKnob("Bass", tone, true, {}, { tone = it })
                    EqSlider("31 Hz", gains[0], true, false, {}, { gains[0] = it })
                    Spacer(Modifier.height(3000.dp))
                }
            }
        }
    }

    private fun TouchInjectionScope.drag(from: Offset, by: Offset) {
        down(from)
        repeat(10) { moveBy(by / 10f) }
        up()
    }

    private fun assertUnchanged() {
        compose.runOnIdle {
            assertEquals(List(EQ_BANDS) { 0f }, gains.toList())
            assertEquals(0f, tone)
        }
    }

    /** Where the graph draws band [band] at 0 dB (see EqGraph). */
    private fun TouchInjectionScope.point(band: Int): Offset {
        val left = 30.dp.toPx()
        val right = width - 6.dp.toPx()
        val top = 14.dp.toPx()
        val bottom = height - 36.dp.toPx()
        return Offset(left + (band + 0.5f) / EQ_BANDS * (right - left), (top + bottom) / 2f)
    }

    @Test
    fun swipingOverTheGraphScrolls() {
        compose.onNodeWithTag("eq-graph").performTouchInput {
            // Away from the points, which are on the 0 dB line.
            drag(Offset(point(3).x, 30.dp.toPx()), Offset(0f, -200.dp.toPx()))
        }
        compose.runOnIdle { assertTrue("the page scrolled", scroll.value > 0) }
        assertUnchanged()
    }

    @Test
    fun draggingAPointSetsItsBandAndDoesNotScroll() {
        compose.onNodeWithTag("eq-graph").performTouchInput {
            drag(point(4), Offset(0f, -40.dp.toPx()))
        }
        compose.runOnIdle {
            assertEquals(0, scroll.value)
            assertTrue("band 4 went up: ${gains[4]}", gains[4] > 3f)
            assertEquals(0f, gains[3])
        }
    }

    @Test
    fun swipingOverASliderScrolls() {
        compose.onNodeWithTag("eq-slider-31 Hz").performTouchInput { drag(center, Offset(0f, -200.dp.toPx())) }
        compose.runOnIdle { assertTrue("the page scrolled", scroll.value > 0) }
        assertUnchanged()
    }

    @Test
    fun swipingOverAKnobScrolls() {
        compose.onNodeWithTag("eq-knob-Bass").performTouchInput { drag(center, Offset(0f, -200.dp.toPx())) }
        compose.runOnIdle { assertTrue("the page scrolled", scroll.value > 0) }
        assertUnchanged()
    }

    @Test
    fun slidersAndKnobsMoveSideways() {
        compose.onNodeWithTag("eq-slider-31 Hz").performTouchInput { drag(center, Offset(width / 4f, 0f)) }
        compose.onNodeWithTag("eq-knob-Bass").performTouchInput { drag(center, Offset(60.dp.toPx(), 0f)) }
        compose.runOnIdle {
            assertEquals(0, scroll.value)
            assertTrue("slider: ${gains[0]}", gains[0] > 3f)
            assertTrue("knob: $tone", tone > 3f)
        }
    }
}
