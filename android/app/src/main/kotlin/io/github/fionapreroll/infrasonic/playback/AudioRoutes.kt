// SPDX-License-Identifier: GPL-3.0-only
package io.github.fionapreroll.infrasonic.playback

import android.content.Context
import android.media.AudioAttributes
import android.media.AudioDeviceCallback
import android.media.AudioDeviceInfo
import android.media.AudioManager
import android.os.Build
import android.os.Handler
import android.os.Looper

/** Watches which output media plays through and reports it to [onChange] on the main thread. */
class AudioRoutes(context: Context, private val onChange: (OutputDevice) -> Unit) {
    private val am = context.getSystemService(AudioManager::class.java)
    private val main = Handler(Looper.getMainLooper())
    private val media = AudioAttributes.Builder()
        .setUsage(AudioAttributes.USAGE_MEDIA)
        .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
        .build()

    private val callback = object : AudioDeviceCallback() {
        override fun onAudioDevicesAdded(added: Array<out AudioDeviceInfo>) = changed()
        override fun onAudioDevicesRemoved(removed: Array<out AudioDeviceInfo>) = changed()
    }

    // The routing can follow a device change a moment later; look again then.
    private val recheck = Runnable { onChange(current()) }

    private fun changed() {
        onChange(current())
        main.removeCallbacks(recheck)
        main.postDelayed(recheck, 750)
    }

    fun start() {
        am.registerAudioDeviceCallback(callback, main)
        onChange(current())
    }

    /** The output media plays through now. */
    fun current(): OutputDevice {
        val routed = if (Build.VERSION.SDK_INT >= 33) {
            runCatching { am.getAudioDevicesForAttributes(media) }.getOrNull()?.firstOrNull()
        } else {
            null
        }
        val info = routed ?: am.getDevices(AudioManager.GET_DEVICES_OUTPUTS).filter { it.isSink }
            .maxByOrNull { priority(it.type) }
        return info?.let { device(it.type, it.productName?.toString().orEmpty()) } ?: OutputDevice.SPEAKER
    }

    companion object {
        /** Which of several outputs Android plays media through, when it cannot tell (before 13). */
        fun priority(type: Int): Int = when (kind(type)) {
            OutputKind.Bluetooth -> 5
            OutputKind.Usb -> 4
            OutputKind.Wired -> 3
            OutputKind.Hdmi -> 2
            OutputKind.Speaker -> 1
            OutputKind.Other -> 0
        }

        fun kind(type: Int): OutputKind = when (type) {
            AudioDeviceInfo.TYPE_BUILTIN_SPEAKER, AudioDeviceInfo.TYPE_BUILTIN_SPEAKER_SAFE,
            AudioDeviceInfo.TYPE_BUILTIN_EARPIECE,
            -> OutputKind.Speaker
            AudioDeviceInfo.TYPE_WIRED_HEADPHONES, AudioDeviceInfo.TYPE_WIRED_HEADSET,
            AudioDeviceInfo.TYPE_LINE_ANALOG,
            -> OutputKind.Wired
            AudioDeviceInfo.TYPE_USB_HEADSET, AudioDeviceInfo.TYPE_USB_DEVICE, AudioDeviceInfo.TYPE_USB_ACCESSORY,
            AudioDeviceInfo.TYPE_DOCK,
            -> OutputKind.Usb
            AudioDeviceInfo.TYPE_BLUETOOTH_A2DP, AudioDeviceInfo.TYPE_BLUETOOTH_SCO,
            AudioDeviceInfo.TYPE_BLE_HEADSET, AudioDeviceInfo.TYPE_BLE_SPEAKER, AudioDeviceInfo.TYPE_BLE_BROADCAST,
            AudioDeviceInfo.TYPE_HEARING_AID,
            -> OutputKind.Bluetooth
            AudioDeviceInfo.TYPE_HDMI, AudioDeviceInfo.TYPE_HDMI_ARC, AudioDeviceInfo.TYPE_HDMI_EARC,
            -> OutputKind.Hdmi
            else -> OutputKind.Other
        }

        /**
         * Bluetooth and USB devices are told apart by name (each headset has its own sound);
         * the speaker, wired headphones and HDMI are one device each.
         */
        fun device(type: Int, name: String): OutputDevice {
            val kind = kind(type)
            val key = when (kind) {
                OutputKind.Speaker -> "speaker"
                OutputKind.Wired -> "wired"
                OutputKind.Hdmi -> "hdmi"
                OutputKind.Bluetooth -> "bt:$name"
                OutputKind.Usb -> "usb:$name"
                OutputKind.Other -> "other:$type"
            }
            // The speaker reports the phone's model as its name.
            return OutputDevice(key, kind, if (kind == OutputKind.Speaker) "" else name)
        }
    }
}
