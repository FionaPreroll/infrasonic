// SPDX-License-Identifier: GPL-3.0-only
package io.github.fionapreroll.infrasonic.visualizer

import android.content.Context
import io.github.fionapreroll.infrasonic.BuildConfig
import io.github.fionapreroll.infrasonic.data.Graph
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.ByteArrayOutputStream
import java.io.File
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URL
import java.security.MessageDigest
import kotlin.coroutines.coroutineContext

sealed interface BundleState {
    data object Idle : BundleState
    data object Checking : BundleState
    /** A bundle newer than the installed one (or the first one) can be downloaded. */
    data class Available(val bundle: PresetBundle) : BundleState
    data class Downloading(val bundle: PresetBundle, val progress: Float) : BundleState
    data object Installing : BundleState
    data object Failed : BundleState
}

/**
 * The visualizer's presets: 24 starters shipped in the app (assets/visualizer) and, if the user asks for it, the full
 * collection from the Lactea fork's releases. Nothing is downloaded and no release is looked at unless the user taps
 * "Download" or switches on the check for new collections. A download must have the size and SHA-256 its index
 * names, and every file in it the size and SHA-256 of the bundle's manifest ([PresetBundle.install]); a bundle is
 * unpacked next to the old one and replaces it only once it is complete.
 */
object Presets {
    private const val TIMEOUT_MS = 15_000
    private const val STARTER_VERSION = 1

    private val _state = MutableStateFlow<BundleState>(BundleState.Idle)
    val state: StateFlow<BundleState> = _state.asStateFlow()
    private val _installed = MutableStateFlow(0)
    /** Version of the installed bundle, 0 = only the starters. */
    val installed: StateFlow<Int> = _installed.asStateFlow()
    private val _count = MutableStateFlow(0)
    /** Presets the visualizer reads. */
    val count: StateFlow<Int> = _count.asStateFlow()
    private var job: Job? = null

    private fun root(context: Context) = File(context.filesDir, "visualizer")
    private fun starter(context: Context) = File(root(context), "starter")
    private fun bundleDir(context: Context, version: Int) = File(root(context), "bundle-$version")

    /** Install the starters if needed and point the visualizer at the collections. Call off the main thread. */
    suspend fun prepare(context: Context) = withContext(Dispatchers.IO) {
        installStarters(context)
        _installed.value = Graph.prefs.visualizerBundle.value.takeIf { bundleDir(context, it).isDirectory } ?: 0
        apply(context)
    }

    /** At the app's start: look for a newer bundle if the user asked for that. */
    fun onStart(context: Context) {
        if (Graph.prefs.visualizerUpdates.value) check(context, manual = false)
    }

    /** Ask the release for its newest bundle; [BundleState.Available] if it is newer than the installed one. */
    fun check(context: Context, manual: Boolean) {
        if (_state.value != BundleState.Idle && _state.value != BundleState.Failed) return
        _state.value = BundleState.Checking
        job = Graph.scope.launch {
            val bundle = latest()
            _state.value = when {
                bundle == null -> if (manual) BundleState.Failed else BundleState.Idle
                bundle.version > _installed.value -> BundleState.Available(bundle)
                else -> BundleState.Idle
            }
        }
    }

    /** Download, check and install `bundle`, then use it. */
    fun download(context: Context, bundle: PresetBundle) {
        _state.value = BundleState.Downloading(bundle, 0f)
        job = Graph.scope.launch {
            val ok = withContext(Dispatchers.IO) {
                val archive = File(context.cacheDir, "lactea-presets.zip")
                try {
                    fetch(bundle, archive) { p -> _state.value = BundleState.Downloading(bundle, p) }
                    _state.value = BundleState.Installing
                    val dest = bundleDir(context, bundle.version)
                    val tmp = File(root(context), "bundle-${bundle.version}.tmp")
                    PresetBundle.install(archive, bundle, tmp)
                    dest.deleteRecursively()
                    if (!tmp.renameTo(dest)) throw IOException("cannot move the bundle into place")
                    val old = _installed.value
                    Graph.prefs.visualizerBundle.value = bundle.version
                    _installed.value = bundle.version
                    apply(context)
                    if (old != 0 && old != bundle.version) bundleDir(context, old).deleteRecursively()
                    true
                } catch (e: IOException) {
                    false
                } finally {
                    archive.delete()
                }
            }
            _state.value = if (ok) BundleState.Idle else BundleState.Failed
        }
    }

    /** Back to the starters only. */
    fun remove(context: Context) {
        job?.cancel()
        Graph.scope.launch(Dispatchers.IO) {
            val old = _installed.value
            Graph.prefs.visualizerBundle.value = 0
            _installed.value = 0
            apply(context)
            if (old != 0) bundleDir(context, old).deleteRecursively()
            _state.value = BundleState.Idle
        }
    }

    fun dismiss() {
        job?.cancel()
        _state.value = BundleState.Idle
    }

    private fun apply(context: Context) {
        val roots = listOfNotNull(
            starter(context).path,
            _installed.value.takeIf { it > 0 }?.let { bundleDir(context, it).path },
        )
        _count.value = Graph.core.visualizerSetCollections(roots).toInt()
    }

    /** Copy the shipped starters into files/visualizer/starter (once per starter set). */
    private fun installStarters(context: Context) {
        val dir = starter(context)
        val marker = File(dir, ".version")
        if (marker.isFile && marker.readText() == STARTER_VERSION.toString()) return
        dir.deleteRecursively()
        fun copy(asset: String, to: File) {
            val children = context.assets.list(asset).orEmpty()
            if (children.isEmpty()) {
                to.parentFile?.mkdirs()
                context.assets.open(asset).use { input -> to.outputStream().use { input.copyTo(it) } }
            } else {
                for (c in children) copy("$asset/$c", File(to, c))
            }
        }
        copy("visualizer", dir)
        marker.writeText(STARTER_VERSION.toString())
    }

    /** The release's index, or null if it did not answer or is no bundle index. */
    private suspend fun latest(): PresetBundle? = withContext(Dispatchers.IO) {
        try {
            open(BuildConfig.PRESETS_URL + "/latest/download/lactea-presets.json").useConnection { c ->
                c.inputStream.use { input ->
                    val out = ByteArrayOutputStream()
                    val buf = ByteArray(1024)
                    while (true) {
                        val n = input.read(buf)
                        if (n < 0) break
                        out.write(buf, 0, n)
                        if (out.size() > PresetBundle.MAX_INDEX) return@useConnection null
                    }
                    PresetBundle.parse(out.toString(Charsets.UTF_8.name()), BuildConfig.PRESETS_URL)
                }
            }
        } catch (e: IOException) {
            null
        }
    }

    /** Write exactly `bundle.bytes` bytes with the right SHA-256 to `file`, or throw. */
    private suspend fun fetch(bundle: PresetBundle, file: File, progress: (Float) -> Unit) {
        val ok = open(bundle.url).useConnection { c ->
            val sha = MessageDigest.getInstance("SHA-256")
            var total = 0L
            var shown = -1
            c.inputStream.use { input ->
                file.outputStream().use { out ->
                    val buf = ByteArray(64 * 1024)
                    while (true) {
                        coroutineContext.ensureActive()
                        val n = input.read(buf)
                        if (n < 0) break
                        total += n
                        if (total > bundle.bytes) return@useConnection false
                        sha.update(buf, 0, n)
                        out.write(buf, 0, n)
                        val percent = (total * 100 / bundle.bytes).toInt()
                        if (percent != shown) {
                            shown = percent
                            progress(percent / 100f)
                        }
                    }
                }
            }
            total == bundle.bytes && PresetBundle.hex(sha.digest()) == bundle.sha256
        } ?: false
        if (!ok) throw IOException("the download is not the bundle the index names")
    }

    /** A GET over HTTPS; GitHub answers with a redirect to its storage, which must stay on HTTPS. */
    private fun open(url: String): HttpURLConnection {
        val c = URL(url).openConnection() as HttpURLConnection
        c.connectTimeout = TIMEOUT_MS
        c.readTimeout = TIMEOUT_MS
        c.useCaches = false
        c.instanceFollowRedirects = true
        return c
    }

    private inline fun <T> HttpURLConnection.useConnection(block: (HttpURLConnection) -> T?): T? = try {
        if (responseCode != HttpURLConnection.HTTP_OK || url.protocol != "https") null else block(this)
    } finally {
        disconnect()
    }
}
