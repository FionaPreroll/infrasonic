// SPDX-License-Identifier: GPL-3.0-only
package io.github.fionapreroll.infrasonic.data

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import io.github.fionapreroll.infrasonic.core.Infrasonic
import io.github.fionapreroll.infrasonic.core.Quality
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.launch

/**
 * Picks the streaming quality for the current network (Wi-Fi or mobile data) and tells the
 * core; also reconnects when the network comes back while offline.
 */
class NetworkQuality(
    context: Context,
    private val core: Infrasonic,
    private val prefs: UiPrefs,
    private val servers: Servers,
) {
    private val cm = context.getSystemService(ConnectivityManager::class.java)
    private val _metered = MutableStateFlow(cm.isActiveNetworkMetered)
    val metered: StateFlow<Boolean> = _metered.asStateFlow()

    init {
        cm.registerDefaultNetworkCallback(object : ConnectivityManager.NetworkCallback() {
            override fun onCapabilitiesChanged(network: Network, caps: NetworkCapabilities) {
                _metered.value = !caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_METERED)
            }

            override fun onAvailable(network: Network) {
                val c = servers.connection.value
                if (c is Connection.Offline && !prefs.offlineMode.value) {
                    Graph.scope.launch { servers.connectActive() }
                }
            }
        })
        Graph.scope.launch {
            combine(
                _metered,
                prefs.wifiBitRate.flow,
                prefs.mobileBitRate.flow,
                prefs.pinBitRate.flow,
                prefs.format.flow,
            ) { metered, wifi, mobile, pin, format -> Triple(if (metered) mobile else wifi, wifi.takeIf { pin < 0 } ?: pin, format) }
                .collect { (stream, pin, format) -> apply(stream, pin, format) }
        }
    }

    private fun apply(stream: Int, pin: Int, format: String) {
        fun q(kbps: Int) = Quality(kbps.coerceAtLeast(0).toUInt(), format.ifEmpty { null }.takeIf { kbps > 0 })
        val s = core.settings()
        val next = s.copy(quality = q(stream), pinQuality = q(pin))
        if (next != s) core.setSettings(next)
    }
}
