// SPDX-License-Identifier: GPL-3.0-only
package io.github.fionapreroll.infrasonic.update

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class ReleaseTest {
    private val sha = "ab".repeat(32)

    private fun json(build: Any = 1000036, version: Any = "\"0.1.0-36-abc1234\"", size: Any = 41234567, sha256: Any = "\"$sha\"") =
        """{"build": $build, "version": $version, "size": $size, "sha256": $sha256}"""

    @Test
    fun readsVersionJson() {
        val r = Release.parse(json())!!
        assertEquals(Release(1000036, "0.1.0-36-abc1234", 41234567, sha), r)
        assertTrue(r.newerThan(100))
        assertTrue(!r.newerThan(1000036))
        assertEquals("39.3", r.megabytes())
    }

    @Test
    fun refusesWhatIsNotARelease() {
        assertNull(Release.parse("not json"))
        assertNull(Release.parse("{}"))
        assertNull(Release.parse(json(build = 0)))
        assertNull(Release.parse(json(size = 0)))
        assertNull(Release.parse(json(size = Release.MAX_APK + 1)))
        assertNull(Release.parse(json(sha256 = "\"${"AB".repeat(32)}\"")))
        assertNull(Release.parse(json(sha256 = "\"abc\"")))
        assertNull(Release.parse(json(version = "\"<script>\"")))
        assertNull(Release.parse(json() + " ".repeat(Release.MAX_JSON)))
    }
}
