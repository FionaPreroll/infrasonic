// SPDX-License-Identifier: GPL-3.0-only
package io.github.fionapreroll.infrasonic.playback

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.media.AudioAttributes
import android.media.AudioFocusRequest
import android.media.AudioManager
import androidx.core.content.ContextCompat
import io.github.fionapreroll.infrasonic.core.PlayerCommand

/**
 * Audio focus and "becoming noisy" for the Rust engine, which plays through AAudio and knows
 * nothing about either: pause for calls and other apps, resume after short interruptions,
 * pause when headphones are unplugged. Ducking is done by the system.
 */
class AudioFocus(private val context: Context, private val controller: PlayerController) {
    private val am = context.getSystemService(AudioManager::class.java)
    private var hasFocus = false
    private var resumeOnGain = false
    private var noisyRegistered = false
    private var abandonLater = false

    private val request = AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN)
        .setAudioAttributes(
            AudioAttributes.Builder()
                .setUsage(AudioAttributes.USAGE_MEDIA)
                .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
                .build(),
        )
        .setWillPauseWhenDucked(false)
        .setOnAudioFocusChangeListener { change ->
            when (change) {
                AudioManager.AUDIOFOCUS_GAIN -> {
                    hasFocus = true
                    if (resumeOnGain) controller.command(PlayerCommand.Play)
                    resumeOnGain = false
                }
                AudioManager.AUDIOFOCUS_LOSS_TRANSIENT -> {
                    resumeOnGain = controller.isPlaying
                    controller.command(PlayerCommand.Pause)
                }
                AudioManager.AUDIOFOCUS_LOSS -> {
                    hasFocus = false
                    resumeOnGain = false
                    controller.command(PlayerCommand.Pause)
                    abandonLater = true
                }
            }
        }
        .build()

    private val noisy = object : BroadcastReceiver() {
        override fun onReceive(c: Context, intent: Intent) {
            if (intent.action == AudioManager.ACTION_AUDIO_BECOMING_NOISY) controller.command(PlayerCommand.Pause)
        }
    }

    private val onChange: () -> Unit = { update() }

    init {
        controller.addListener(onChange)
    }

    private fun update() {
        if (abandonLater) {
            abandonLater = false
            am.abandonAudioFocusRequest(request)
        }
        if (controller.isPlaying) {
            if (!hasFocus) {
                val r = am.requestAudioFocus(request)
                hasFocus = r == AudioManager.AUDIOFOCUS_REQUEST_GRANTED
                if (r == AudioManager.AUDIOFOCUS_REQUEST_FAILED) controller.command(PlayerCommand.Pause)
            }
            if (!noisyRegistered) {
                ContextCompat.registerReceiver(
                    context, noisy, IntentFilter(AudioManager.ACTION_AUDIO_BECOMING_NOISY), ContextCompat.RECEIVER_NOT_EXPORTED,
                )
                noisyRegistered = true
            }
        } else {
            if (noisyRegistered) {
                context.unregisterReceiver(noisy)
                noisyRegistered = false
            }
            // Keep the focus during a transient loss, so that playback resumes afterwards.
            if (hasFocus && !resumeOnGain) {
                am.abandonAudioFocusRequest(request)
                hasFocus = false
            }
        }
    }

    fun release() {
        controller.removeListener(onChange)
        if (noisyRegistered) context.unregisterReceiver(noisy)
        noisyRegistered = false
        if (hasFocus) am.abandonAudioFocusRequest(request)
        hasFocus = false
    }
}
