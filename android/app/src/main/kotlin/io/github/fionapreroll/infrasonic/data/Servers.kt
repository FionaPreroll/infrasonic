// SPDX-License-Identifier: GPL-3.0-only
package io.github.fionapreroll.infrasonic.data

import io.github.fionapreroll.infrasonic.core.FfiException
import io.github.fionapreroll.infrasonic.core.Infrasonic
import io.github.fionapreroll.infrasonic.core.ServerProfile
import io.github.fionapreroll.infrasonic.core.ServerStatus
import io.github.fionapreroll.infrasonic.playback.PlayerController
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withTimeoutOrNull
import java.util.UUID

sealed interface Connection {
    data object None : Connection
    data class Connecting(val profile: ServerProfile) : Connection
    data class Online(val profile: ServerProfile, val status: ServerStatus) : Connection

    /** Pinned songs only: the server is unreachable or offline mode is on. */
    data class Offline(val profile: ServerProfile, val reason: String?) : Connection

    /** Wrong login or not a Subsonic server. */
    data class Failed(val profile: ServerProfile, val error: FfiException) : Connection
}

val Connection.isUsable get() = this is Connection.Online || this is Connection.Offline

/** The saved servers and the connection to the one in use. */
class Servers(
    private val core: Infrasonic,
    private val prefs: UiPrefs,
    private val player: PlayerController,
) {
    private val _profiles = MutableStateFlow(core.profiles())
    val profiles: StateFlow<List<ServerProfile>> = _profiles.asStateFlow()

    private val _activeId = MutableStateFlow(core.activeProfileId() ?: _profiles.value.firstOrNull()?.id)
    val activeId: StateFlow<String?> = _activeId.asStateFlow()

    private val _connection = MutableStateFlow<Connection>(Connection.None)
    val connection: StateFlow<Connection> = _connection.asStateFlow()

    private val lock = Mutex()

    fun active(): ServerProfile? = _profiles.value.find { it.id == _activeId.value }

    fun newId(): String = UUID.randomUUID().toString()

    private fun persist() = core.saveProfiles(_profiles.value, _activeId.value)

    /** Add or replace a server (by id). */
    fun save(profile: ServerProfile) {
        val list = _profiles.value.toMutableList()
        val i = list.indexOfFirst { it.id == profile.id }
        if (i >= 0) list[i] = profile else list += profile
        _profiles.value = list
        if (_activeId.value == null) _activeId.value = profile.id
        persist()
    }

    fun delete(id: String) {
        _profiles.value = _profiles.value.filter { it.id != id }
        if (_activeId.value == id) {
            _activeId.value = _profiles.value.firstOrNull()?.id
            core.disconnect()
            _connection.value = Connection.None
        }
        persist()
    }

    /** Switch to another server and connect. */
    suspend fun select(id: String) {
        _activeId.value = id
        persist()
        connectActive()
    }

    /** Connect to the active server; falls back to offline mode when it cannot be reached. */
    suspend fun connectActive() = lock.withLock {
        val p = active() ?: run {
            _connection.value = Connection.None
            return@withLock
        }
        _connection.value = Connection.Connecting(p)
        val keepQueue = core.connectedProfile() == p.id && player.queue.value.isNotEmpty()
        _connection.value = if (prefs.offlineMode.value) {
            core.connectOffline(p)
            Connection.Offline(p, null)
        } else {
            try {
                Connection.Online(p, core.connect(p))
            } catch (e: FfiException.Network) {
                core.connectOffline(p)
                Connection.Offline(p, e.reason)
            } catch (e: FfiException) {
                Connection.Failed(p, e)
            }
        }
        if (_connection.value.isUsable && !keepQueue) player.restoreLocalQueue()
        player.refresh()
    }

    /** Wait until a session can be used (for background work started before the app). */
    suspend fun awaitUsable(timeoutMs: Long = 30_000): Boolean =
        withTimeoutOrNull(timeoutMs) { connection.first { it.isUsable || it is Connection.Failed } }?.isUsable == true
}
