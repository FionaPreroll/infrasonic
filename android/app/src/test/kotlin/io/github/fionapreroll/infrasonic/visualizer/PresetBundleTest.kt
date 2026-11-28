// SPDX-License-Identifier: GPL-3.0-only
package io.github.fionapreroll.infrasonic.visualizer

import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import java.io.File
import java.io.IOException
import java.security.MessageDigest
import java.util.zip.ZipEntry
import java.util.zip.ZipOutputStream

class PresetBundleTest {
    @get:Rule
    val tmp = TemporaryFolder()

    private val releases = "https://github.com/FionaPreroll/lactea/releases"
    private val sha = "a".repeat(64)

    private fun index(url: String = "$releases/download/presets-v2/lactea-presets-2.zip", hash: String = sha, bytes: Long = 1000) =
        """{"schema": 1, "version": 2, "url": "$url", "sha256": "$hash", "bytes": $bytes,
            "expanded_bytes": 5000, "presets": 3, "textures": 1}"""

    @Test
    fun parsesAnIndexOfTheRelease() {
        val b = PresetBundle.parse(index(), releases)
        assertNotNull(b)
        assertEquals(2, b!!.version)
        assertEquals(3, b.presets)
    }

    @Test
    fun rejectsAnIndexThatPointsElsewhereOrIsMalformed() {
        assertNull(PresetBundle.parse(index(url = "https://example.org/lactea-presets-2.zip"), releases))
        assertNull(PresetBundle.parse(index(url = "$releases/download/presets-v3/lactea-presets-2.zip"), releases))
        assertNull(PresetBundle.parse(index(hash = "xyz"), releases))
        assertNull(PresetBundle.parse(index(bytes = PresetBundle.MAX_ARCHIVE + 1), releases))
        assertNull(PresetBundle.parse(index().replace("\"schema\": 1", "\"schema\": 2"), releases))
        assertNull(PresetBundle.parse("not json", releases))
        assertNull(PresetBundle.parse(" ".repeat(PresetBundle.MAX_INDEX + 1), releases))
    }

    private fun hash(b: ByteArray) = PresetBundle.hex(MessageDigest.getInstance("SHA-256").digest(b))

    /** A bundle archive with `files`, and a manifest that lists `listed` (default: the same files). */
    private fun archive(
        files: Map<String, ByteArray>,
        listed: Map<String, ByteArray> = files,
        version: Int = 2,
    ): File {
        val entry = { path: String, data: ByteArray ->
            JSONObject().put("path", path).put("sha256", hash(data)).put("bytes", data.size)
        }
        val presets = JSONArray()
        val textures = JSONArray()
        var notice: JSONObject? = null
        for ((p, d) in listed) {
            when {
                p.startsWith("presets/") -> presets.put(entry(p, d))
                p.startsWith("textures/") -> textures.put(entry(p, d))
                else -> notice = entry(p, d)
            }
        }
        val manifest = JSONObject().put("schema", 1).put("version", version)
            .put("presets", presets).put("textures", textures).put("notice", notice)
        val f = tmp.newFile()
        ZipOutputStream(f.outputStream()).use { z ->
            for ((p, d) in files + ("manifest.json" to manifest.toString().toByteArray())) {
                z.putNextEntry(ZipEntry(p))
                z.write(d)
                z.closeEntry()
            }
        }
        return f
    }

    private val good = mapOf(
        "presets/Fractal/a.milk" to "[preset00]\nzoom=1.01\n".toByteArray(),
        "presets/Waveform/b.milk" to "[preset00]\nzoom=0.99\n".toByteArray(),
        "textures/clouds.jpg" to byteArrayOf(1, 2, 3),
        "NOTICE.md" to "notice".toByteArray(),
    )
    private val bundle = PresetBundle(2, "unused", sha, 1000, 5000, 2)

    @Test
    fun installsAVerifiedBundle() {
        val dest = File(tmp.root, "bundle-2")
        PresetBundle.install(archive(good), bundle, dest)
        assertEquals("[preset00]\nzoom=1.01\n", File(dest, "presets/Fractal/a.milk").readText())
        assertTrue(File(dest, "textures/clouds.jpg").isFile)
    }

    private fun refused(file: File, b: PresetBundle = bundle) {
        val dest = File(tmp.root, "bundle-x")
        try {
            PresetBundle.install(file, b, dest)
            fail("installed a bundle that should be refused")
        } catch (e: IOException) {
            assertFalse("a refused bundle leaves nothing behind", dest.exists())
        }
    }

    @Test
    fun refusesAFileThatDiffersFromTheManifest() {
        refused(archive(good + ("presets/Fractal/a.milk" to "[preset00]\nzoom=9\n".toByteArray()), listed = good))
    }

    @Test
    fun refusesFilesTheManifestDoesNotList() {
        refused(archive(good + ("presets/extra.milk" to "x".toByteArray()), listed = good))
    }

    @Test
    fun refusesABundleWithFilesMissing() {
        refused(archive(good - "textures/clouds.jpg", listed = good))
    }

    @Test
    fun refusesPathsThatLeaveTheBundle() {
        for (p in listOf("../evil.milk", "presets/../../evil.milk", "/abs.milk", "presets\\a.milk", "other/a.milk")) {
            val files = good + (p to "x".toByteArray())
            refused(archive(files, listed = files))
        }
    }

    @Test
    fun refusesTheManifestOfAnotherVersionAndOversizedBundles() {
        refused(archive(good, version = 3))
        refused(archive(good), bundle.copy(expandedBytes = 10))
    }
}
