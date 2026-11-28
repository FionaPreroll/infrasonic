// SPDX-License-Identifier: GPL-3.0-only
package io.github.fionapreroll.infrasonic.data

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

class CoversTest {
    @get:Rule
    val tmp = TemporaryFolder()

    @Test
    fun sampleSizeKeepsAtLeastTheAskedSize() {
        assertEquals(1, Covers.sampleSize(300, 300, 300))
        assertEquals(1, Covers.sampleSize(500, 500, 300))
        assertEquals(4, Covers.sampleSize(512, 512, 96))
        // A server that ignores the size and sends the original.
        assertEquals(16, Covers.sampleSize(3000, 2000, 96))
        assertEquals(1, Covers.sampleSize(50, 50, 96))
    }

    @Test
    fun trimDirDeletesTheOldestFirst() {
        val files = (0 until 10).map { i ->
            tmp.newFile("c$i").apply {
                writeBytes(ByteArray(100))
                setLastModified(1_000_000L + i * 1000)
            }
        }
        // Below the limit nothing happens.
        assertEquals(1000, Covers.trimDir(files, 1000))
        assertTrue(files.all { it.exists() })
        // Above it: down to 90 % of the limit, oldest first.
        assertEquals(400, Covers.trimDir(files, 500))
        assertTrue(files.take(6).none { it.exists() })
        assertTrue(files.drop(6).all { it.exists() })
        assertFalse(files[5].exists())
    }
}
