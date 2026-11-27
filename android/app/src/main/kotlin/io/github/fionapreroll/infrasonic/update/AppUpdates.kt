// SPDX-License-Identifier: GPL-3.0-only
package io.github.fionapreroll.infrasonic.update

import android.content.Context
import android.net.ConnectivityManager
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import io.github.fionapreroll.infrasonic.BuildConfig
import io.github.fionapreroll.infrasonic.R
import io.github.fionapreroll.infrasonic.data.Graph
import io.github.fionapreroll.infrasonic.ui.toast
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

sealed interface UpdateState {
    data object Idle : UpdateState
    data object Checking : UpdateState
    data class Available(val release: Release) : UpdateState
    data class Downloading(val release: Release, val progress: Float) : UpdateState
}

/** Looking for, offering and installing a newer GitHub build (see [Updates]). */
object AppUpdates {
    /** Debug builds have their own package, which a release cannot update. */
    val enabled = Updates.AVAILABLE && !BuildConfig.DEBUG

    private val _state = MutableStateFlow<UpdateState>(UpdateState.Idle)
    val state: StateFlow<UpdateState> = _state.asStateFlow()
    private var job: Job? = null

    /** At the app's start, if the user agreed. */
    fun onStart(context: Context) {
        if (enabled && Graph.prefs.checkUpdates.value) check(context, manual = false)
    }

    /** A check started by the user also says when there is nothing new or the release did not answer. */
    fun check(context: Context, manual: Boolean) {
        if (!enabled || _state.value != UpdateState.Idle) return
        _state.value = UpdateState.Checking
        job = Graph.scope.launch {
            val release = Updates.latest()
            _state.value = when {
                release == null -> UpdateState.Idle.also { if (manual) context.toast(R.string.update_failed) }
                release.newerThan(BuildConfig.VERSION_CODE.toLong()) -> UpdateState.Available(release)
                else -> UpdateState.Idle.also { if (manual) context.toast(R.string.update_none) }
            }
        }
    }

    fun download(context: Context, release: Release) {
        _state.value = UpdateState.Downloading(release, 0f)
        job = Graph.scope.launch {
            val apk = Updates.download(context, release) { p -> _state.value = UpdateState.Downloading(release, p) }
            _state.value = UpdateState.Idle
            if (apk == null) context.toast(R.string.update_failed) else Updates.install(context, apk)
        }
    }

    fun dismiss() {
        job?.cancel()
        _state.value = UpdateState.Idle
    }
}

/** The question whether to look for updates (once), the offer of a newer build and the download. */
@Composable
fun UpdateDialog() {
    if (!AppUpdates.enabled) return
    val state by AppUpdates.state.collectAsState()
    val asked by Graph.prefs.askedUpdates.flow.collectAsState()
    val context = LocalContext.current.applicationContext
    if (!asked) {
        fun answer(yes: Boolean) {
            Graph.prefs.checkUpdates.value = yes
            Graph.prefs.askedUpdates.value = true
            if (yes) AppUpdates.check(context, manual = false)
        }
        AlertDialog(
            onDismissRequest = {},
            title = { Text(stringResource(R.string.updates)) },
            text = { Text(stringResource(R.string.check_updates_question)) },
            confirmButton = { TextButton({ answer(true) }) { Text(stringResource(R.string.yes)) } },
            dismissButton = { TextButton({ answer(false) }) { Text(stringResource(R.string.no)) } },
        )
        return
    }
    when (val s = state) {
        is UpdateState.Available -> {
            val metered = context.getSystemService(ConnectivityManager::class.java)?.isActiveNetworkMetered == true
            AlertDialog(
                onDismissRequest = AppUpdates::dismiss,
                title = { Text(stringResource(R.string.update_title)) },
                text = {
                    Column {
                        Text(stringResource(R.string.update_text, s.release.version, s.release.megabytes()))
                        if (metered) {
                            Spacer(Modifier.height(8.dp))
                            Text(stringResource(R.string.update_metered))
                        }
                    }
                },
                confirmButton = {
                    TextButton({ AppUpdates.download(context, s.release) }) { Text(stringResource(R.string.update_download)) }
                },
                dismissButton = { TextButton(AppUpdates::dismiss) { Text(stringResource(R.string.update_later)) } },
            )
        }
        is UpdateState.Downloading -> AlertDialog(
            onDismissRequest = {},
            title = { Text(stringResource(R.string.update_title)) },
            text = { LinearProgressIndicator(progress = { s.progress }, modifier = Modifier.fillMaxWidth()) },
            confirmButton = {},
            dismissButton = { TextButton(AppUpdates::dismiss) { Text(stringResource(R.string.cancel)) } },
        )
        else -> {}
    }
}
