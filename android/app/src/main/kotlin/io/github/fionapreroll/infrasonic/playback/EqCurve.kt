// SPDX-License-Identifier: GPL-3.0-only
package io.github.fionapreroll.infrasonic.playback

import kotlin.math.PI
import kotlin.math.abs
import kotlin.math.cos
import kotlin.math.hypot
import kotlin.math.log10
import kotlin.math.pow
import kotlin.math.sin

/** Center frequencies of the core's bands in Hz (BANDS in crates/engine/src/eq.rs). */
val EQ_FREQS = doubleArrayOf(31.0, 62.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0)

/**
 * The frequency response that the core's equalizer gives for band gains, for the curve on the
 * equalizer screen. The same math as crates/engine/src/eq.rs: one octave RBJ peaking filters,
 * with their gains solved so that every band center gets exactly its band's gain.
 */
class EqResponse(gains: List<Float>, private val rate: Double = 48_000.0) {
    private val filters: DoubleArray = filterGains(rate, DoubleArray(EQ_BANDS) { i -> gains.getOrElse(i) { 0f }.toDouble() })

    /** The gain at [freq] in dB. */
    fun db(freq: Double): Double {
        var sum = 0.0
        for (i in 0 until EQ_BANDS) sum += peakingDb(rate, EQ_FREQS[i], filters[i], freq)
        return sum
    }

    private companion object {
        const val Q = 1.4142135623730951

        /** Magnitude response of one peaking filter in dB. */
        fun peakingDb(rate: Double, center: Double, gainDb: Double, freq: Double): Double {
            if (gainDb == 0.0) return 0.0
            val a = 10.0.pow(gainDb / 40.0)
            val w0 = 2.0 * PI * center / rate
            val alpha = sin(w0) / (2.0 * Q)
            val cs = cos(w0)
            val a0 = 1.0 + alpha / a
            val b0 = (1.0 + alpha * a) / a0
            val b1 = -2.0 * cs / a0
            val b2 = (1.0 - alpha * a) / a0
            val a1 = b1
            val a2 = (1.0 - alpha / a) / a0
            val w = 2.0 * PI * freq / rate
            val c1 = cos(w)
            val s1 = sin(w)
            val c2 = cos(2.0 * w)
            val s2 = sin(2.0 * w)
            val num = hypot(b0 + b1 * c1 + b2 * c2, b1 * s1 + b2 * s2)
            val den = hypot(1.0 + a1 * c1 + a2 * c2, a1 * s1 + a2 * s2)
            return 20.0 * log10(num / den)
        }

        /** The filter gains that give [target] at the band centers (filter_gains in eq.rs). */
        fun filterGains(rate: Double, target: DoubleArray): DoubleArray {
            val probe = 12.0
            val n = EQ_BANDS
            val m = Array(n) { i -> DoubleArray(n) { j -> peakingDb(rate, EQ_FREQS[j], probe, EQ_FREQS[i]) / probe } }
            val g = DoubleArray(n)
            val miss = target.copyOf()
            repeat(3) {
                val step = solve(m, miss)
                for (i in 0 until n) g[i] += step[i]
                for (i in 0 until n) {
                    var total = 0.0
                    for (j in 0 until n) total += peakingDb(rate, EQ_FREQS[j], g[j], EQ_FREQS[i])
                    miss[i] = target[i] - total
                }
            }
            return g
        }

        /** Gaussian elimination with partial pivoting. */
        fun solve(m: Array<DoubleArray>, y: DoubleArray): DoubleArray {
            val n = y.size
            val a = Array(n) { r -> DoubleArray(n + 1) { c -> if (c < n) m[r][c] else y[r] } }
            for (c in 0 until n) {
                val p = (c until n).maxByOrNull { abs(a[it][c]) } ?: c
                val t = a[c]
                a[c] = a[p]
                a[p] = t
                if (abs(a[c][c]) < 1e-12) continue
                val pivot = a[c].copyOf()
                for (r in 0 until n) {
                    if (r == c) continue
                    val k = a[r][c] / pivot[c]
                    for (x in c..n) a[r][x] -= k * pivot[x]
                }
            }
            return DoubleArray(n) { i -> if (abs(a[i][i]) < 1e-12) 0.0 else a[i][n] / a[i][i] }
        }
    }
}

/**
 * The three tone controls (bass, mids, treble): each moves a group of bands, the bands in the
 * middle of its range the most. The groups do not overlap, so turning one control never moves
 * another. A control shows the level that its bands have together, so it also reads presets
 * and bands set by hand, and turning it keeps the shape of the bands it moves.
 */
enum class Tone(private val weights: FloatArray) {
    Bass(floatArrayOf(1f, 1f, 0.8f, 0.45f, 0f, 0f, 0f, 0f, 0f, 0f)),
    Mid(floatArrayOf(0f, 0f, 0f, 0f, 0.7f, 1f, 0.7f, 0f, 0f, 0f)),
    Treble(floatArrayOf(0f, 0f, 0f, 0f, 0f, 0f, 0f, 0.6f, 1f, 1f)),
    ;

    private val norm = weights.sumOf { (it * it).toDouble() }.toFloat()

    /** The bands this control moves. */
    val bands: IntRange = weights.indexOfFirst { it > 0f }..weights.indexOfLast { it > 0f }

    /** How much this control moves [band] (0 to 1). */
    fun weight(band: Int): Float = weights.getOrElse(band) { 0f }

    /** The level of this control for [gains], within ±[EQ_MAX_DB]. */
    fun level(gains: List<Float>): Float {
        var s = 0f
        for (i in weights.indices) s += weights[i] * gains.getOrElse(i) { 0f }
        return (s / norm).coerceIn(-EQ_MAX_DB, EQ_MAX_DB)
    }

    /** [start] with this control turned by [delta] dB. */
    fun turn(start: List<Float>, delta: Float): List<Float> =
        List(EQ_BANDS) { i -> (start.getOrElse(i) { 0f } + delta * weight(i)).coerceIn(-EQ_MAX_DB, EQ_MAX_DB) }
}
