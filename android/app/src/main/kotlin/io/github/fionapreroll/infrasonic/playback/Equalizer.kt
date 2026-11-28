// SPDX-License-Identifier: GPL-3.0-only
package io.github.fionapreroll.infrasonic.playback

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import org.json.JSONArray
import org.json.JSONObject

/** Bands of the core's equalizer (31 Hz to 16 kHz) and the range of each gain. */
const val EQ_BANDS = 10
const val EQ_MAX_DB = 12f

/** What the core gets: a preamp and one gain per band, in dB. */
data class EqLevels(val preamp: Float, val gains: List<Float>) {
    /** The level the core lowers everything by so that boosts cannot clip. */
    val headroom: Float get() = preamp - maxOf(0f, gains.maxOrNull() ?: 0f)
}

data class EqPreset(val id: String, val name: String, val levels: EqLevels, val builtIn: Boolean)

enum class OutputKind { Speaker, Wired, Usb, Bluetooth, Hdmi, Other }

/** An audio output; devices with the same key share their equalizer settings. */
data class OutputDevice(val key: String, val kind: OutputKind, val name: String) {
    companion object {
        val SPEAKER = OutputDevice("speaker", OutputKind.Speaker, "")
    }
}

/** The equalizer of one output device. `preset` is null once the bands were changed by hand. */
data class DeviceEq(
    val enabled: Boolean = false,
    val preset: String? = FLAT,
    val levels: EqLevels = EqLevels(0f, List(EQ_BANDS) { 0f }),
    /** Name of the device when it was last seen, for the list of devices. */
    val name: String = "",
)

data class EqState(
    val device: OutputDevice,
    /** Settings of [device], or of all devices when [perDevice] is off. */
    val current: DeviceEq,
    val presets: List<EqPreset>,
    val perDevice: Boolean,
) {
    val preset: EqPreset? get() = presets.firstOrNull { it.id == current.preset }
}

const val FLAT = "flat"

/** Built-in presets; the app shows their names from its string resources by id. */
val BUILT_IN_PRESETS: List<EqPreset> = listOf(
    FLAT to floatArrayOf(0f, 0f, 0f, 0f, 0f, 0f, 0f, 0f, 0f, 0f),
    "bass" to floatArrayOf(6f, 5f, 4f, 2f, 0f, 0f, 0f, 0f, 0f, 0f),
    "less_bass" to floatArrayOf(-6f, -5f, -3f, -1f, 0f, 0f, 0f, 0f, 0f, 0f),
    "treble" to floatArrayOf(0f, 0f, 0f, 0f, 0f, 0f, 1f, 3f, 5f, 6f),
    "loudness" to floatArrayOf(5f, 4f, 2f, 0f, -1f, -1f, 0f, 1f, 3f, 4f),
    "vocal" to floatArrayOf(-2f, -2f, -1f, 0f, 2f, 3f, 3f, 2f, 0f, -1f),
    "speech" to floatArrayOf(-6f, -4f, -2f, 0f, 2f, 3f, 3f, 2f, 0f, -2f),
    "rock" to floatArrayOf(4f, 3f, 2f, 0f, -1f, -1f, 0f, 2f, 3f, 4f),
    "pop" to floatArrayOf(-1f, 0f, 2f, 3f, 3f, 1f, 0f, -1f, -1f, -1f),
    "jazz" to floatArrayOf(3f, 2f, 1f, 2f, -1f, -1f, 0f, 1f, 2f, 3f),
    "classical" to floatArrayOf(0f, 0f, 0f, 0f, 0f, 0f, -2f, -3f, -3f, -5f),
    "electronic" to floatArrayOf(5f, 4f, 1f, 0f, -2f, 1f, 0f, 1f, 4f, 5f),
    "acoustic" to floatArrayOf(3f, 3f, 2f, 1f, 1f, 1f, 2f, 2f, 2f, 1f),
).map { (id, g) -> EqPreset(id, id, EqLevels(0f, g.toList()), builtIn = true) }

/** Where the equalizer settings are kept (shared preferences in the app, memory in tests). */
interface EqStore {
    fun load(): String?
    fun save(json: String)
}

/**
 * The equalizer settings: presets, and the settings of each output device. Every change is
 * saved and handed to [apply] (null = off), but only when what the core hears changes, so
 * that switching between devices with the same settings does nothing. The core fades
 * between settings, so changes can be made while playing.
 */
class EqualizerManager(
    private val store: EqStore,
    private val apply: (EqLevels?) -> Unit,
    device: OutputDevice = OutputDevice.SPEAKER,
) {
    private var userPresets: List<EqPreset>
    private val devices: MutableMap<String, DeviceEq>
    private var perDevice: Boolean
    private var device: OutputDevice = device
    private var applied: EqLevels? = null
    private var appliedOnce = false

    private val _state: MutableStateFlow<EqState>
    val state: StateFlow<EqState>

    init {
        val json = store.load()?.let { runCatching { JSONObject(it) }.getOrNull() }
        userPresets = json?.optJSONArray("presets")?.let(::readPresets).orEmpty()
        devices = json?.optJSONObject("devices")?.let(::readDevices) ?: mutableMapOf()
        perDevice = json?.optBoolean("perDevice", true) ?: true
        _state = MutableStateFlow(snapshot())
        state = _state.asStateFlow()
    }

    private fun key() = if (perDevice) device.key else SHARED

    private fun current() = devices[key()] ?: DeviceEq()

    private fun snapshot() = EqState(device, current(), BUILT_IN_PRESETS + userPresets, perDevice)

    /** Send the current settings to the core (again); call once at the start. */
    @Synchronized
    fun applyNow() {
        appliedOnce = false
        publish()
    }

    /** The output changed (headphones plugged in, Bluetooth connected, ...). */
    @Synchronized
    fun onDevice(device: OutputDevice) {
        if (device == this.device && appliedOnce) return
        this.device = device
        if (perDevice) devices[device.key]?.let { devices[device.key] = it.copy(name = device.name) }
        publish()
    }

    @Synchronized
    fun setEnabled(enabled: Boolean) = change { it.copy(enabled = enabled) }

    @Synchronized
    fun selectPreset(id: String) {
        val p = (BUILT_IN_PRESETS + userPresets).firstOrNull { it.id == id } ?: return
        change { it.copy(preset = p.id, levels = p.levels, enabled = true) }
    }

    @Synchronized
    fun setGain(band: Int, db: Float) {
        if (band !in 0 until EQ_BANDS) return
        change {
            val gains = it.levels.gains.toMutableList().apply { this[band] = clamp(db) }
            it.copy(preset = null, levels = it.levels.copy(gains = gains))
        }
    }

    /** Set all bands at once (the tone controls move several). */
    @Synchronized
    fun setGains(gains: List<Float>) {
        if (gains.size != EQ_BANDS) return
        change { it.copy(preset = null, levels = it.levels.copy(gains = gains.map(::clamp))) }
    }

    @Synchronized
    fun setPreamp(db: Float) = change { it.copy(preset = null, levels = it.levels.copy(preamp = clamp(db))) }

    /**
     * Save the current bands as a preset named [name], replacing a saved preset of the same
     * name (devices that use it get the new bands).
     * Returns its id, or null for an empty name.
     */
    @Synchronized
    fun savePreset(name: String): String? {
        val n = name.trim()
        if (n.isEmpty()) return null
        val old = userPresets.firstOrNull { it.name.equals(n, ignoreCase = true) }
        val id = old?.id ?: "user:${System.currentTimeMillis().toString(36)}:${userPresets.size}"
        val preset = EqPreset(id, n, current().levels, builtIn = false)
        userPresets = if (old != null) userPresets.map { if (it.id == id) preset else it } else userPresets + preset
        // Other devices with this preset get its new bands too.
        for ((k, v) in devices) if (v.preset == id) devices[k] = v.copy(levels = preset.levels)
        change { it.copy(preset = id) }
        return id
    }

    /** Delete a saved preset; devices that used it keep its bands. */
    @Synchronized
    fun deletePreset(id: String) {
        if (userPresets.none { it.id == id }) return
        userPresets = userPresets.filter { it.id != id }
        for ((k, v) in devices) if (v.preset == id) devices[k] = v.copy(preset = null)
        save()
        publish()
    }

    /** Keep settings per output device, or one for all. */
    @Synchronized
    fun setPerDevice(on: Boolean) {
        if (on == perDevice) return
        // Start from what is heard now, so that the switch itself changes nothing.
        val now = current()
        perDevice = on
        if (devices[key()] == null) devices[key()] = now.copy(name = if (on) device.name else "")
        save()
        publish()
    }

    private fun change(f: (DeviceEq) -> DeviceEq) {
        devices[key()] = f(current()).copy(name = if (perDevice) device.name else "")
        save()
        publish()
    }

    private fun publish() {
        val s = snapshot()
        _state.value = s
        val levels = s.current.levels.takeIf { s.current.enabled }
        if (!appliedOnce || levels != applied) {
            applied = levels
            appliedOnce = true
            apply(levels)
        }
    }

    private fun save() {
        val json = JSONObject()
            .put("perDevice", perDevice)
            .put("presets", JSONArray().apply { userPresets.forEach { put(writePreset(it)) } })
            .put(
                "devices",
                JSONObject().apply {
                    for ((k, d) in devices) {
                        put(
                            k,
                            JSONObject().put("enabled", d.enabled).put("preset", d.preset ?: JSONObject.NULL)
                                .put("name", d.name).put("levels", writeLevels(d.levels)),
                        )
                    }
                },
            )
        store.save(json.toString())
    }

    companion object {
        /** Key of the settings shared by all devices. */
        const val SHARED = "*"

        private fun clamp(db: Float) = if (db.isFinite()) db.coerceIn(-EQ_MAX_DB, EQ_MAX_DB) else 0f

        private fun writeLevels(l: EqLevels) = JSONObject().put("preamp", l.preamp.toDouble())
            .put("gains", JSONArray().apply { l.gains.forEach { put(it.toDouble()) } })

        private fun readLevels(o: JSONObject?): EqLevels {
            val g = o?.optJSONArray("gains")
            val gains = List(EQ_BANDS) { i -> clamp(g?.optDouble(i, 0.0)?.toFloat() ?: 0f) }
            return EqLevels(clamp(o?.optDouble("preamp", 0.0)?.toFloat() ?: 0f), gains)
        }

        private fun writePreset(p: EqPreset) = JSONObject().put("id", p.id).put("name", p.name).put("levels", writeLevels(p.levels))

        private fun readPresets(a: JSONArray): List<EqPreset> = (0 until a.length()).mapNotNull { i ->
            val o = a.optJSONObject(i) ?: return@mapNotNull null
            val id = o.optString("id").takeIf { it.isNotEmpty() } ?: return@mapNotNull null
            EqPreset(id, o.optString("name", id), readLevels(o.optJSONObject("levels")), builtIn = false)
        }

        private fun readDevices(o: JSONObject): MutableMap<String, DeviceEq> {
            val m = mutableMapOf<String, DeviceEq>()
            for (k in o.keys()) {
                val d = o.optJSONObject(k) ?: continue
                m[k] = DeviceEq(
                    enabled = d.optBoolean("enabled", false),
                    preset = if (d.isNull("preset")) null else d.optString("preset"),
                    levels = readLevels(d.optJSONObject("levels")),
                    name = d.optString("name", ""),
                )
            }
            return m
        }
    }
}
