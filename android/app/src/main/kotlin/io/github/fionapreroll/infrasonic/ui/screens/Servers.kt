// SPDX-License-Identifier: GPL-3.0-only
@file:OptIn(ExperimentalMaterial3Api::class)

package io.github.fionapreroll.infrasonic.ui.screens

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Edit
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import androidx.core.net.toUri
import io.github.fionapreroll.infrasonic.R
import io.github.fionapreroll.infrasonic.core.ServerAuth
import io.github.fionapreroll.infrasonic.core.ServerProfile
import io.github.fionapreroll.infrasonic.core.ServerStatus
import io.github.fionapreroll.infrasonic.core.tokenAuth
import io.github.fionapreroll.infrasonic.data.Connection
import io.github.fionapreroll.infrasonic.data.Graph
import io.github.fionapreroll.infrasonic.ui.LocalNav
import io.github.fionapreroll.infrasonic.ui.Routes
import io.github.fionapreroll.infrasonic.ui.Screen
import io.github.fionapreroll.infrasonic.ui.components.ConfirmDialog
import io.github.fionapreroll.infrasonic.ui.components.Empty
import io.github.fionapreroll.infrasonic.ui.components.errorText
import io.github.fionapreroll.infrasonic.ui.navBarPadding
import kotlinx.coroutines.launch

@Composable
fun ServersScreen() {
    val nav = LocalNav.current
    val profiles by Graph.servers.profiles.collectAsState()
    val active by Graph.servers.activeId.collectAsState()
    val connection by Graph.servers.connection.collectAsState()
    var deleting by remember { mutableStateOf<ServerProfile?>(null) }
    Screen(stringResource(R.string.servers), actions = {
        IconButton(onClick = { nav.navigate(Routes.server(Routes.NEW_SERVER)) }) { Icon(Icons.Default.Add, stringResource(R.string.add_server)) }
    }) {
        if (profiles.isEmpty()) {
            Empty(stringResource(R.string.no_servers))
            return@Screen
        }
        LazyColumn {
            items(profiles, key = { it.id }) { p ->
                val isActive = p.id == active
                ListItem(
                    modifier = Modifier.clickable { Graph.scope.launch { Graph.servers.select(p.id) } },
                    leadingContent = { RadioButton(isActive, null) },
                    headlineContent = { Text(p.name) },
                    supportingContent = {
                        Column {
                            Text(p.url)
                            if (isActive) Text(connectionText(connection), color = MaterialTheme.colorScheme.primary)
                        }
                    },
                    trailingContent = {
                        Row {
                            IconButton(onClick = { nav.navigate(Routes.server(p.id)) }) { Icon(Icons.Default.Edit, stringResource(R.string.edit)) }
                            IconButton(onClick = { deleting = p }) { Icon(Icons.Default.Delete, stringResource(R.string.delete)) }
                        }
                    },
                )
            }
        }
    }
    deleting?.let { p ->
        ConfirmDialog(stringResource(R.string.delete_server, p.name), { deleting = null }) { Graph.servers.delete(p.id) }
    }
}

@Composable
private fun connectionText(c: Connection): String = when (c) {
    is Connection.Online -> stringResource(R.string.state_online, statusText(c.status))
    is Connection.Offline -> stringResource(R.string.offline_banner)
    is Connection.Connecting -> stringResource(R.string.connecting, c.profile.name)
    is Connection.Failed -> errorText(c.error)
    Connection.None -> ""
}

private fun statusText(s: ServerStatus) =
    listOfNotNull(s.serverType, s.serverVersion).joinToString(" ").ifEmpty { "Subsonic ${s.version}" }

private enum class AuthKind { Token, Plain, ApiKey }

@Composable
fun ServerEditScreen(id: String) {
    val nav = LocalNav.current
    val scope = rememberCoroutineScope()
    val existing = remember(id) { Graph.servers.profiles.value.find { it.id == id } }
    val first = remember { Graph.servers.profiles.value.isEmpty() }
    var name by rememberSaveable { mutableStateOf(existing?.name ?: "") }
    var url by rememberSaveable { mutableStateOf(existing?.url ?: "https://") }
    var kind by rememberSaveable {
        mutableStateOf(
            when (existing?.auth) {
                is ServerAuth.Plain -> AuthKind.Plain
                is ServerAuth.ApiKey -> AuthKind.ApiKey
                else -> AuthKind.Token
            },
        )
    }
    var user by rememberSaveable {
        mutableStateOf(
            when (val a = existing?.auth) {
                is ServerAuth.Token -> a.username
                is ServerAuth.Plain -> a.username
                else -> ""
            },
        )
    }
    // Saved passwords are not shown; an empty field keeps the saved login.
    var secret by rememberSaveable { mutableStateOf("") }
    var selfSigned by rememberSaveable { mutableStateOf(existing?.allowSelfSigned ?: false) }
    var testing by remember { mutableStateOf(false) }
    var result by remember { mutableStateOf<Result<ServerStatus>?>(null) }

    fun auth(): ServerAuth? {
        val keep = existing?.auth
        return when (kind) {
            AuthKind.Token -> when {
                secret.isNotEmpty() -> tokenAuth(user.trim(), secret)
                keep is ServerAuth.Token && keep.username == user.trim() -> keep
                else -> null
            }
            AuthKind.Plain -> when {
                secret.isNotEmpty() -> ServerAuth.Plain(user.trim(), secret)
                keep is ServerAuth.Plain && keep.username == user.trim() -> keep
                else -> null
            }
            AuthKind.ApiKey -> when {
                secret.isNotEmpty() -> ServerAuth.ApiKey(secret.trim())
                keep is ServerAuth.ApiKey -> keep
                else -> null
            }
        }
    }

    fun profile(): ServerProfile? {
        val a = auth() ?: return null
        val u = url.trim().trimEnd('/')
        if (!u.startsWith("http://") && !u.startsWith("https://") || u.length < 10) return null
        val n = name.trim().ifEmpty { u.toUri().host ?: u }
        return ServerProfile(existing?.id ?: Graph.servers.newId(), n, u, a, selfSigned)
    }

    val valid = profile() != null
    Screen(stringResource(if (existing == null) R.string.add_server else R.string.edit_server), back = !first) {
        Column(
            Modifier.verticalScroll(rememberScrollState()).padding(16.dp).imePadding().navBarPadding(),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            if (first) Text(stringResource(R.string.welcome), style = MaterialTheme.typography.bodyLarge)
            OutlinedTextField(
                url,
                { url = it; result = null },
                Modifier.fillMaxWidth(),
                label = { Text(stringResource(R.string.server_url)) },
                singleLine = true,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
            )
            OutlinedTextField(name, { name = it }, Modifier.fillMaxWidth(), label = { Text(stringResource(R.string.name_optional)) }, singleLine = true)
            SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
                AuthKind.entries.forEachIndexed { i, k ->
                    SegmentedButton(
                        selected = kind == k,
                        onClick = { kind = k; result = null },
                        shape = SegmentedButtonDefaults.itemShape(i, AuthKind.entries.size),
                    ) {
                        Text(
                            stringResource(
                                when (k) {
                                    AuthKind.Token -> R.string.auth_token
                                    AuthKind.Plain -> R.string.auth_plain
                                    AuthKind.ApiKey -> R.string.auth_api_key
                                },
                            ),
                        )
                    }
                }
            }
            Text(
                stringResource(
                    when (kind) {
                        AuthKind.Token -> R.string.auth_token_hint
                        AuthKind.Plain -> R.string.auth_plain_hint
                        AuthKind.ApiKey -> R.string.auth_api_key_hint
                    },
                ),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            if (kind != AuthKind.ApiKey) {
                OutlinedTextField(user, { user = it; result = null }, Modifier.fillMaxWidth(), label = { Text(stringResource(R.string.username)) }, singleLine = true)
            }
            OutlinedTextField(
                secret,
                { secret = it; result = null },
                Modifier.fillMaxWidth(),
                label = { Text(stringResource(if (kind == AuthKind.ApiKey) R.string.api_key else R.string.password)) },
                placeholder = existing?.let { { Text(stringResource(R.string.password_unchanged)) } },
                singleLine = true,
                visualTransformation = PasswordVisualTransformation(),
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
            )
            Row(verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text(stringResource(R.string.self_signed))
                    Text(stringResource(R.string.self_signed_hint), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                Switch(selfSigned, { selfSigned = it; result = null })
            }
            if (url.trim().startsWith("http://")) {
                Text(stringResource(R.string.plain_http_warning), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error)
            }
            result?.let { r ->
                Text(
                    r.fold({ stringResource(R.string.test_ok, statusText(it)) }, { errorText(it) }),
                    color = if (r.isSuccess) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.error,
                )
            }
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
                OutlinedButton(enabled = valid && !testing, onClick = {
                    val p = profile() ?: return@OutlinedButton
                    testing = true
                    scope.launch {
                        result = runCatching { Graph.core.testConnection(p) }
                        testing = false
                    }
                }) { Text(stringResource(R.string.test_connection)) }
                Button(enabled = valid && !testing, onClick = {
                    val p = profile() ?: return@Button
                    Graph.servers.save(p)
                    // Not the screen's scope: leaving the screen would cancel the connect.
                    Graph.scope.launch {
                        if (first || Graph.servers.activeId.value == p.id) Graph.servers.select(p.id)
                    }
                    if (first) {
                        nav.navigate(Routes.HOME) { popUpTo(0) }
                    } else {
                        nav.popBackStack()
                    }
                }) { Text(stringResource(R.string.save)) }
                if (testing) CircularProgressIndicator()
            }
        }
    }
}
