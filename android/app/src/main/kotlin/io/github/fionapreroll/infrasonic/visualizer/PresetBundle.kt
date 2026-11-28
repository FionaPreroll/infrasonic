// SPDX-License-Identifier: GPL-3.0-only
package io.github.fionapreroll.infrasonic.visualizer

import org.json.JSONObject
import java.io.File
import java.io.IOException
import java.io.InputStream
import java.security.MessageDigest
import java.util.zip.ZipEntry
import java.util.zip.ZipFile

/**
 * A preset bundle as its index `lactea-presets.json` describes it (written by the Lactea fork's preset workflow next
 * to the archive): `{"schema": 1, "version": 3, "url": "…/lactea-presets-3.zip", "sha256": "…", "bytes": …,
 * "expanded_bytes": …, "presets": 9443, "textures": 43}`.
 */
data class PresetBundle(
    val version: Int,
    val url: String,
    val sha256: String,
    val bytes: Long,
    val expandedBytes: Long,
    val presets: Int,
) {
    /** Its download size in megabytes, with one decimal. */
    fun megabytes(): String = String.format(java.util.Locale.ROOT, "%.1f", bytes / (1024.0 * 1024.0))

    companion object {
        /** The largest index read, archive downloaded and archive unpacked, in bytes. */
        const val MAX_INDEX = 4 * 1024
        const val MAX_ARCHIVE = 128L * 1024 * 1024
        const val MAX_EXPANDED = 512L * 1024 * 1024
        const val MAX_FILES = 60_000
        private const val MAX_MANIFEST = 16 * 1024 * 1024
        private val SHA256 = Regex("[0-9a-f]{64}")

        /** The index of a bundle at `releases`, or null if `json` is none (or points anywhere else). */
        fun parse(json: String, releases: String): PresetBundle? {
            if (json.length > MAX_INDEX) return null
            return runCatching {
                val o = JSONObject(json)
                if (o.getInt("schema") != 1) return null
                PresetBundle(
                    o.getInt("version"),
                    o.getString("url"),
                    o.getString("sha256"),
                    o.getLong("bytes"),
                    o.getLong("expanded_bytes"),
                    o.getInt("presets"),
                )
            }.getOrNull()?.takeIf {
                it.version > 0 && SHA256.matches(it.sha256) && it.bytes in 1..MAX_ARCHIVE &&
                    it.expandedBytes in 1..MAX_EXPANDED && it.presets > 0 &&
                    it.url == "$releases/download/presets-v${it.version}/lactea-presets-${it.version}.zip"
            }
        }

        /**
         * Unpack `archive` (already checked against the index's size and SHA-256) into a new directory `dest` and check
         * every file against the bundle's own manifest: each one listed with its size and SHA-256, nothing else
         * present, no path leaving `dest`. On any failure `dest` is removed and an [IOException] says why.
         */
        fun install(archive: File, bundle: PresetBundle, dest: File) {
            if (dest.exists()) dest.deleteRecursively()
            if (!dest.mkdirs()) throw IOException("cannot create $dest")
            try {
                ZipFile(archive).use { zip -> unpack(zip, bundle, dest) }
            } catch (e: Exception) {
                dest.deleteRecursively()
                throw e as? IOException ?: IOException("invalid preset bundle", e)
            }
        }

        private fun unpack(zip: ZipFile, bundle: PresetBundle, dest: File) {
            val manifestEntry = zip.getEntry("manifest.json") ?: throw IOException("no manifest")
            if (manifestEntry.size !in 1..MAX_MANIFEST.toLong()) throw IOException("manifest too large")
            val manifest = JSONObject(zip.getInputStream(manifestEntry).use { readAtMost(it, MAX_MANIFEST) }.toString(Charsets.UTF_8))
            if (manifest.getInt("schema") != 1 || manifest.getInt("version") != bundle.version) {
                throw IOException("manifest of another bundle")
            }
            val expected = HashMap<String, Pair<Long, String>>()
            fun add(o: JSONObject) {
                val path = safePath(o.getString("path"))
                val sha = o.getString("sha256")
                if (!SHA256.matches(sha) || expected.put(path, o.getLong("bytes") to sha) != null) {
                    throw IOException("invalid manifest entry $path")
                }
            }
            for (key in listOf("presets", "textures")) {
                val list = manifest.getJSONArray(key)
                for (i in 0 until list.length()) add(list.getJSONObject(i))
            }
            add(manifest.getJSONObject("notice"))
            if (expected.size > MAX_FILES) throw IOException("too many files")
            var total = 0L
            val seen = HashSet<String>()
            val root = dest.canonicalFile
            for (entry in zip.entries()) {
                if (entry.isDirectory) continue
                val path = safePath(entry.name)
                if (path == "manifest.json") continue
                val (size, sha) = expected[path] ?: throw IOException("unexpected file $path")
                if (!seen.add(path)) throw IOException("duplicate file $path")
                total += size
                if (total > bundle.expandedBytes || total > MAX_EXPANDED) throw IOException("bundle larger than announced")
                val target = File(root, path)
                if (!target.canonicalPath.startsWith(root.path + File.separator)) throw IOException("unsafe path $path")
                target.parentFile?.mkdirs()
                copyChecked(zip, entry, target, size, sha)
            }
            if (seen.size != expected.size) throw IOException("files missing from the bundle")
        }

        /** Write `entry` to `target` if it has exactly `size` bytes with SHA-256 `sha`. */
        private fun copyChecked(zip: ZipFile, entry: ZipEntry, target: File, size: Long, sha: String) {
            val digest = MessageDigest.getInstance("SHA-256")
            var n = 0L
            zip.getInputStream(entry).use { input ->
                target.outputStream().use { out ->
                    val buf = ByteArray(64 * 1024)
                    while (true) {
                        val r = input.read(buf)
                        if (r < 0) break
                        n += r
                        if (n > size) throw IOException("${entry.name} is larger than listed")
                        digest.update(buf, 0, r)
                        out.write(buf, 0, r)
                    }
                }
            }
            if (n != size || hex(digest.digest()) != sha) throw IOException("${entry.name} does not match the manifest")
        }

        /** `path` if it is a plain relative path below a bundle's root (`presets/…`, `textures/…`, `NOTICE.md`). */
        internal fun safePath(path: String): String {
            val parts = path.split('/')
            val ok = path.isNotEmpty() && path.length <= 1024 && !path.startsWith("/") && '\\' !in path &&
                parts.none { it.isEmpty() || it == "." || it == ".." } && path.none { it < ' ' } &&
                (parts[0] in setOf("presets", "textures") && parts.size >= 2 || path in setOf("NOTICE.md", "manifest.json"))
            if (!ok) throw IOException("unsafe path $path")
            return path
        }

        internal fun hex(bytes: ByteArray) = bytes.joinToString("") { "%02x".format(it) }

        private fun readAtMost(input: InputStream, max: Int): ByteArray {
            val out = java.io.ByteArrayOutputStream()
            val buf = ByteArray(8192)
            while (true) {
                val n = input.read(buf)
                if (n < 0) break
                out.write(buf, 0, n)
                if (out.size() > max) throw IOException("too large")
            }
            return out.toByteArray()
        }
    }
}
