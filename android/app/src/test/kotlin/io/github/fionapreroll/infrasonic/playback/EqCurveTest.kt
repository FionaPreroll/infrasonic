// SPDX-License-Identifier: GPL-3.0-only
package io.github.fionapreroll.infrasonic.playback

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import kotlin.math.abs

class EqCurveTest {
    private fun preset(id: String) = BUILT_IN_PRESETS.first { it.id == id }.levels.gains

    @Test
    fun flatIsFlat() {
        val r = EqResponse(List(EQ_BANDS) { 0f })
        for (f in listOf(20.0, 100.0, 1000.0, 15000.0)) assertEquals(0.0, r.db(f), 1e-9)
    }

    @Test
    fun theCurveGoesThroughEveryBand() {
        for (gains in listOf(preset("bass"), preset("loudness"), preset("electronic"), List(EQ_BANDS) { if (it % 2 == 0) 12f else -12f })) {
            val r = EqResponse(gains)
            for (i in 0 until EQ_BANDS) assertEquals("band $i of $gains", gains[i].toDouble(), r.db(EQ_FREQS[i]), 0.25)
        }
    }

    @Test
    fun oneBandIsAPeakAroundItsCenter() {
        val r = EqResponse(List(EQ_BANDS) { if (it == 5) 6f else 0f })
        assertEquals(6.0, r.db(1000.0), 0.05)
        assertTrue(r.db(1400.0) in 0.5..5.9)
        assertTrue(abs(r.db(100.0)) < 0.5)
    }

    @Test
    fun toneControlsReadAndTurnTheirBands() {
        val flat = List(EQ_BANDS) { 0f }
        for (t in Tone.entries) {
            assertEquals(0f, t.level(flat))
            val g = t.turn(flat, 4f)
            assertEquals(4f, t.level(g), 1e-4f)
            // The other controls stay where they are.
            for (o in Tone.entries) if (o != t) assertEquals(0f, o.level(g), 1e-6f)
            // Bands outside the control do not move.
            for (i in 0 until EQ_BANDS) if (t.weight(i) == 0f) assertEquals(0f, g[i])
        }
        assertTrue("the bass preset reads as bass", Tone.Bass.level(preset("bass")) > 4f)
        assertEquals(0f, Tone.Treble.level(preset("bass")))
    }

    @Test
    fun turningKeepsTheShapeAndStaysInRange() {
        val start = preset("rock")
        val up = Tone.Mid.turn(start, 2f)
        assertEquals(start[5] + 2f, up[5], 1e-5f)
        assertEquals(start[4] + 1.4f, up[4], 1e-5f)
        val max = Tone.Bass.turn(start, 40f)
        assertTrue(max.all { it in -EQ_MAX_DB..EQ_MAX_DB })
        assertEquals(EQ_MAX_DB, Tone.Bass.level(max))
    }
}
