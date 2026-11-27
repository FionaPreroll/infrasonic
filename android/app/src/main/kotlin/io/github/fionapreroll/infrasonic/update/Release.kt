// SPDX-License-Identifier: GPL-3.0-only
package io.github.fionapreroll.infrasonic.update

import org.json.JSONObject

/**
 * The newest GitHub build as the release android-latest describes it in version.json, which CI
 * writes next to the APK (.github/workflows/ci.yml):
 * `{"build": 1000036, "version": "0.1.0-36-abc1234", "size": 41234567, "sha256": "…"}`.
 */
data class Release(val build: Long, val version: String, val size: Long, val sha256: String) {
    fun newerThan(installed: Long) = build > installed

    /** Its size in megabytes, with one decimal. */
    fun megabytes(): String = String.format(java.util.Locale.ROOT, "%.1f", size / (1024.0 * 1024.0))

    companion object {
        /** The largest version.json read and the largest APK taken, in bytes. */
        const val MAX_JSON = 4 * 1024
        const val MAX_APK = 200L * 1024 * 1024
        private val SHA256 = Regex("[0-9a-f]{64}")
        private val VERSION = Regex("[0-9A-Za-z.+-]{1,64}")

        /** The release in a version.json, or null if it is none. */
        fun parse(json: String): Release? {
            if (json.length > MAX_JSON) return null
            return runCatching {
                val o = JSONObject(json)
                Release(o.getLong("build"), o.getString("version"), o.getLong("size"), o.getString("sha256"))
            }.getOrNull()?.takeIf {
                it.build > 0 && it.size in 1..MAX_APK && SHA256.matches(it.sha256) && VERSION.matches(it.version)
            }
        }
    }
}
