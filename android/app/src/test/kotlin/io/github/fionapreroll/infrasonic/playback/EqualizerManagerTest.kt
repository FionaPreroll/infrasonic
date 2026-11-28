// SPDX-License-Identifier: GPL-3.0-only
package io.github.fionapreroll.infrasonic.playback

import android.media.AudioDeviceInfo
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class EqualizerManagerTest {
    private class MemStore(var json: String? = null) : EqStore {
        override fun load() = json
        override fun save(json: String) {
            this.json = json
        }
    }

    /** What the core was told, in order. */
    private val sent = mutableListOf<EqLevels?>()
    private val store = MemStore()

    private fun manager(s: EqStore = store) = EqualizerManager(s, { sent += it }).also { it.applyNow() }

    private val buds = OutputDevice("bt:Buds", OutputKind.Bluetooth, "Buds")
    private val wired = OutputDevice("wired", OutputKind.Wired, "USB-C")

    private fun preset(id: String) = BUILT_IN_PRESETS.first { it.id == id }.levels

    @Test
    fun startsOffAndTellsTheCoreOnce() {
        val m = manager()
        assertEquals(listOf<EqLevels?>(null), sent)
        assertEquals(false, m.state.value.current.enabled)
        m.onDevice(OutputDevice.SPEAKER)
        assertEquals("the same device again changes nothing", 1, sent.size)
    }

    @Test
    fun onOffAndSwitchingPresets() {
        val m = manager()
        m.selectPreset("bass")
        m.setEnabled(false)
        m.setEnabled(true)
        m.selectPreset("treble")
        m.selectPreset("treble")
        assertEquals(listOf(null, preset("bass"), null, preset("bass"), preset("treble")), sent)
        assertEquals("treble", m.state.value.current.preset)
    }

    @Test
    fun turningOnWithFlatBandsStillTurnsItOn() {
        val m = manager()
        m.setEnabled(true)
        assertEquals(listOf(null, preset(FLAT)), sent)
    }

    @Test
    fun eachOutputKeepsItsOwnSettings() {
        val m = manager()
        m.selectPreset("loudness") // speaker
        m.onDevice(buds)
        assertNull("a new device starts off", sent.last())
        m.selectPreset("bass")
        m.onDevice(wired)
        m.onDevice(OutputDevice.SPEAKER)
        m.onDevice(buds)
        assertEquals(
            listOf(null, preset("loudness"), null, preset("bass"), null, preset("loudness"), preset("bass")),
            sent,
        )
        assertEquals(buds, m.state.value.device)
        assertEquals("Buds", m.state.value.current.name)
    }

    @Test
    fun devicesWithTheSameSettingsSwitchSilently() {
        val m = manager()
        m.selectPreset("rock")
        m.onDevice(buds)
        m.selectPreset("rock")
        val before = sent.size
        m.onDevice(OutputDevice.SPEAKER)
        m.onDevice(buds)
        assertEquals("nothing to change for the core", before, sent.size)
    }

    @Test
    fun oneSettingForAllOutputs() {
        val m = manager()
        m.selectPreset("vocal")
        m.setPerDevice(false)
        assertEquals("the switch itself changes nothing", preset("vocal"), sent.last())
        val n = sent.size
        m.onDevice(buds)
        m.onDevice(wired)
        assertEquals(n, sent.size)
        m.selectPreset("jazz")
        m.onDevice(OutputDevice.SPEAKER)
        assertEquals(preset("jazz"), sent.last())
        m.setPerDevice(true)
        assertEquals("the speaker keeps what it had", preset("vocal"), sent.last())
    }

    @Test
    fun bandsAndPreamp() {
        val m = manager()
        m.selectPreset("bass")
        m.setGain(9, 4f)
        m.setPreamp(-3f)
        m.setGain(3, 99f)
        m.setGain(42, 1f)
        val cur = m.state.value.current
        assertNull("changed by hand", cur.preset)
        assertEquals(listOf(6f, 5f, 4f, 12f, 0f, 0f, 0f, 0f, 0f, 4f), cur.levels.gains)
        assertEquals(-3f, cur.levels.preamp)
        assertEquals(-15f, cur.levels.headroom)
        assertEquals(cur.levels, sent.last())
    }

    @Test
    fun savedPresetsAndSettingsSurviveARestart() {
        val m = manager()
        m.setEnabled(true)
        m.setGain(0, 3f)
        val id = m.savePreset("  Mine ")
        assertNotNull(id)
        assertEquals(id, m.state.value.current.preset)
        m.onDevice(buds)
        m.selectPreset(id!!)
        m.setGain(1, -2f)
        assertEquals("same name replaces", id, m.savePreset("mine"))
        assertNull(m.savePreset(" "))

        sent.clear()
        val again = manager(MemStore(store.json))
        val mine = again.state.value.presets.single { !it.builtIn }
        assertEquals("the last spelling wins", "mine", mine.name)
        assertEquals(listOf(3f, -2f, 0f, 0f, 0f, 0f, 0f, 0f, 0f, 0f), mine.levels.gains)
        assertEquals("the speaker follows the changed preset", mine.levels, sent.single())
        again.onDevice(buds)
        assertEquals(mine.levels, sent.last())
        assertEquals(id, again.state.value.current.preset)
    }

    @Test
    fun deletingAPresetKeepsWhatDevicesHear() {
        val m = manager()
        m.setGain(2, 5f)
        m.setEnabled(true)
        val id = m.savePreset("Warm")!!
        val n = sent.size
        m.deletePreset(id)
        m.deletePreset(FLAT)
        assertEquals(n, sent.size)
        assertNull(m.state.value.current.preset)
        assertEquals(5f, m.state.value.current.levels.gains[2])
        assertTrue(m.state.value.presets.all { it.builtIn })
        assertEquals(BUILT_IN_PRESETS.size, m.state.value.presets.size)
    }

    @Test
    fun brokenOrOldSettingsAreIgnored() {
        val m = manager(MemStore("{not json"))
        assertEquals(false, m.state.value.current.enabled)
        sent.clear()
        val short = """{"devices":{"speaker":{"enabled":true,"preset":null,"levels":{"preamp":"x","gains":[1,2,99]}}}}"""
        val o = manager(MemStore(short))
        assertEquals(listOf(1f, 2f, 12f, 0f, 0f, 0f, 0f, 0f, 0f, 0f), o.state.value.current.levels.gains)
        assertEquals(0f, o.state.value.current.levels.preamp)
        assertTrue(o.state.value.perDevice)
    }

    @Test
    fun outputsAreToldApart() {
        val a = AudioRoutes.device(AudioDeviceInfo.TYPE_BLUETOOTH_A2DP, "Buds")
        val b = AudioRoutes.device(AudioDeviceInfo.TYPE_BLUETOOTH_A2DP, "Speaker Box")
        assertEquals("bt:Buds", a.key)
        assertTrue(a.key != b.key)
        assertEquals(
            AudioRoutes.device(AudioDeviceInfo.TYPE_WIRED_HEADSET, "x").key,
            AudioRoutes.device(AudioDeviceInfo.TYPE_WIRED_HEADPHONES, "y").key,
        )
        val speaker = AudioRoutes.device(AudioDeviceInfo.TYPE_BUILTIN_SPEAKER, "Pixel 9")
        assertEquals(OutputDevice.SPEAKER, speaker)
        val types = listOf(
            AudioDeviceInfo.TYPE_BUILTIN_SPEAKER,
            AudioDeviceInfo.TYPE_WIRED_HEADPHONES,
            AudioDeviceInfo.TYPE_BLUETOOTH_A2DP,
        )
        assertEquals(AudioDeviceInfo.TYPE_BLUETOOTH_A2DP, types.maxBy { AudioRoutes.priority(it) })
        assertEquals(
            AudioDeviceInfo.TYPE_WIRED_HEADPHONES,
            types.take(2).maxBy { AudioRoutes.priority(it) },
        )
    }

    @Test
    fun settingAllBandsAtOnce() {
        val m = manager()
        m.selectPreset("bass")
        val gains = List(EQ_BANDS) { it - 4f + 20f * (if (it == 9) 1 else 0) }
        m.setGains(gains)
        val cur = m.state.value.current
        assertNull("bands set by hand are no preset", cur.preset)
        assertEquals(List(EQ_BANDS) { if (it == 9) EQ_MAX_DB else it - 4f }, cur.levels.gains)
        assertEquals(cur.levels, sent.last())
        m.setGains(listOf(1f))
        assertEquals("a wrong number of bands is ignored", cur, m.state.value.current)
    }
}
