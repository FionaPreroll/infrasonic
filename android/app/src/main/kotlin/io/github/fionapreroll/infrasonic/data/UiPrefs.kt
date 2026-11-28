// SPDX-License-Identifier: GPL-3.0-only
package io.github.fionapreroll.infrasonic.data

import android.content.Context
import androidx.core.content.edit
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

enum class ThemeMode { System, Light, Dark, Black }

/** App settings that the Rust core does not keep (theme, network dependent quality, ...). */
class UiPrefs(context: Context) {
    private val sp = context.getSharedPreferences("ui", Context.MODE_PRIVATE)

    private fun <T> pref(key: String, read: () -> T, write: (T) -> Unit): Pref<T> = Pref(read(), key, write)

    inner class Pref<T>(initial: T, val key: String, private val write: (T) -> Unit) {
        private val state = MutableStateFlow(initial)
        val flow: StateFlow<T> = state.asStateFlow()
        var value: T
            get() = state.value
            set(v) {
                write(v)
                state.value = v
            }
    }

    private fun intPref(key: String, default: Int) =
        pref(key, { sp.getInt(key, default) }) { v -> sp.edit { putInt(key, v) } }

    private fun boolPref(key: String, default: Boolean) =
        pref(key, { sp.getBoolean(key, default) }) { v -> sp.edit { putBoolean(key, v) } }

    private fun stringPref(key: String, default: String) =
        pref(key, { sp.getString(key, default) ?: default }) { v -> sp.edit { putString(key, v) } }

    val theme = pref(
        "theme",
        { runCatching { ThemeMode.valueOf(sp.getString("theme", null) ?: "") }.getOrDefault(ThemeMode.System) },
    ) { v -> sp.edit { putString("theme", v.name) } }
    val dynamicColor = boolPref("dynamic_color", true)

    /** Fade between pages; off switches pages without animation. */
    val pageTransitions = boolPref("page_transitions", true)

    /** Kbit/s, 0 = original file. */
    val wifiBitRate = intPref("wifi_bit_rate", 0)
    val mobileBitRate = intPref("mobile_bit_rate", 192)

    /** Kbit/s for downloads; -1 = same as streaming over Wi-Fi. */
    val pinBitRate = intPref("pin_bit_rate", -1)

    /** Transcoding format, "" lets the server choose. */
    val format = stringPref("format", "opus")
    val downloadOnWifiOnly = boolPref("download_wifi_only", true)

    /** Use only downloaded songs, even with a connection. */
    val offlineMode = boolPref("offline_mode", false)

    private fun floatPref(key: String, default: Float) =
        pref(key, { sp.getFloat(key, default) }) { v -> sp.edit { putFloat(key, v) } }

    /** Playback speed for music and for spoken word (podcasts, audio books). */
    val speedMusic = floatPref("speed_music", 1f)
    val speedSpoken = floatPref("speed_spoken", 1f)

    /** Download starred songs on Wi-Fi; [autoPinned] are the songs downloaded that way. */
    val autoFavorites = boolPref("auto_favorites", false)
    val autoPinned = pref(
        "auto_pinned",
        { sp.getStringSet("auto_pinned", emptySet())?.toSet().orEmpty() },
    ) { v -> sp.edit { putStringSet("auto_pinned", v) } }

    /** Limit of the cover cache on disk in MiB. */
    val coverCacheMb = intPref("cover_cache_mb", 250)

    /** GitHub build: look for a newer build at every start; asked once at the first start. */
    val checkUpdates = boolPref("check_updates", false)
    val askedUpdates = boolPref("asked_updates", false)

    /** Version of the downloaded preset collection, 0 = only the starters shipped in the app. */
    val visualizerBundle = intPref("visualizer_bundle", 0)

    /** Look for a newer preset collection at every start (off until the user turns it on). */
    val visualizerUpdates = boolPref("visualizer_updates", false)

    /** Seconds a preset stays on screen, 0 = until swiped. */
    val visualizerSeconds = intPref("visualizer_seconds", 30)
}
