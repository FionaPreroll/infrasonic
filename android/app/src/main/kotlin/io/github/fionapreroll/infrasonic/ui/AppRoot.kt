// SPDX-License-Identifier: GPL-3.0-only
@file:OptIn(ExperimentalMaterial3Api::class)

package io.github.fionapreroll.infrasonic.ui

import android.net.Uri
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.QueueMusic
import androidx.compose.material.icons.filled.Album
import androidx.compose.material.icons.filled.CloudOff
import androidx.compose.material.icons.filled.MoreHoriz
import androidx.compose.material.icons.filled.Pause
import androidx.compose.material.icons.filled.Person
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.filled.SkipNext
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.navigation.NavGraph.Companion.findStartDestination
import androidx.navigation.NavHostController
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.currentBackStackEntryAsState
import androidx.navigation.compose.rememberNavController
import io.github.fionapreroll.infrasonic.update.UpdateDialog
import io.github.fionapreroll.infrasonic.R
import io.github.fionapreroll.infrasonic.core.PlayState
import io.github.fionapreroll.infrasonic.data.Connection
import io.github.fionapreroll.infrasonic.data.Graph
import io.github.fionapreroll.infrasonic.ui.components.Cover
import io.github.fionapreroll.infrasonic.ui.screens.AlbumScreen
import io.github.fionapreroll.infrasonic.ui.screens.ArtistScreen
import io.github.fionapreroll.infrasonic.ui.screens.ArtistsScreen
import io.github.fionapreroll.infrasonic.ui.screens.BookmarksScreen
import io.github.fionapreroll.infrasonic.ui.screens.DownloadsScreen
import io.github.fionapreroll.infrasonic.ui.screens.FolderScreen
import io.github.fionapreroll.infrasonic.ui.screens.GenreScreen
import io.github.fionapreroll.infrasonic.ui.screens.GenresScreen
import io.github.fionapreroll.infrasonic.ui.screens.HomeScreen
import io.github.fionapreroll.infrasonic.ui.screens.JukeboxScreen
import io.github.fionapreroll.infrasonic.ui.screens.MoreScreen
import io.github.fionapreroll.infrasonic.ui.screens.NowPlayingScreen
import io.github.fionapreroll.infrasonic.ui.screens.PlaylistScreen
import io.github.fionapreroll.infrasonic.ui.screens.PlaylistsScreen
import io.github.fionapreroll.infrasonic.ui.screens.PodcastScreen
import io.github.fionapreroll.infrasonic.ui.screens.PodcastsScreen
import io.github.fionapreroll.infrasonic.ui.screens.SearchScreen
import io.github.fionapreroll.infrasonic.ui.screens.ServerEditScreen
import io.github.fionapreroll.infrasonic.ui.screens.ServersScreen
import io.github.fionapreroll.infrasonic.ui.screens.SettingsScreen
import io.github.fionapreroll.infrasonic.ui.screens.SharesScreen
import kotlinx.coroutines.launch

val LocalNav = staticCompositionLocalOf<NavHostController> { error("no navigation") }

object Routes {
    const val HOME = "home"
    const val ARTISTS = "artists"
    const val PLAYLISTS = "playlists"
    const val SEARCH = "search"
    const val MORE = "more"
    const val NOW = "now"
    const val SERVERS = "servers"
    const val GENRES = "genres"
    const val PODCASTS = "podcasts"
    const val BOOKMARKS = "bookmarks"
    const val SHARES = "shares"
    const val DOWNLOADS = "downloads"
    const val JUKEBOX = "jukebox"
    const val SETTINGS = "settings"
    const val NEW_SERVER = "new"
    private fun e(s: String) = Uri.encode(s)
    fun server(id: String) = "server/${e(id)}"
    fun album(id: String) = "album/${e(id)}"
    fun artist(id: String) = "artist/${e(id)}"
    fun playlist(id: String) = "playlist/${e(id)}"
    fun genre(name: String) = "genre/${e(name)}"
    fun folder(id: String?) = "folder/${e(id ?: "-")}"
    fun podcast(id: String) = "podcast/${e(id)}"
}

private data class Tab(val route: String, val label: Int, val icon: ImageVector)

private val tabs = listOf(
    Tab(Routes.HOME, R.string.albums, Icons.Default.Album),
    Tab(Routes.ARTISTS, R.string.artists, Icons.Default.Person),
    Tab(Routes.PLAYLISTS, R.string.playlists, Icons.AutoMirrored.Filled.QueueMusic),
    Tab(Routes.SEARCH, R.string.search, Icons.Default.Search),
    Tab(Routes.MORE, R.string.more, Icons.Default.MoreHoriz),
)

@Composable
fun AppRoot() {
    val nav = rememberNavController()
    val start = remember { if (Graph.servers.profiles.value.isEmpty()) Routes.server(Routes.NEW_SERVER) else Routes.HOME }
    val snackbar = remember { SnackbarHostState() }
    LaunchedEffect(Unit) { Graph.player.messages.collect { snackbar.showSnackbar(it) } }
    UpdateDialog()
    CompositionLocalProvider(LocalNav provides nav) {
        val entry by nav.currentBackStackEntryAsState()
        val route = entry?.destination?.route ?: ""
        val fullScreen = route == Routes.NOW || route.startsWith("server/")
        Scaffold(
            snackbarHost = { SnackbarHost(snackbar) },
            bottomBar = {
                if (!fullScreen) {
                    Column {
                        MiniPlayer { nav.navigate(Routes.NOW) }
                        NavigationBar {
                            tabs.forEach { t ->
                                NavigationBarItem(
                                    selected = route == t.route,
                                    onClick = {
                                        nav.navigate(t.route) {
                                            popUpTo(nav.graph.findStartDestination().id) { saveState = true }
                                            launchSingleTop = true
                                            restoreState = true
                                        }
                                    },
                                    icon = { Icon(t.icon, null) },
                                    label = { Text(stringResource(t.label), maxLines = 1) },
                                )
                            }
                        }
                    }
                }
            },
        ) { padding ->
            Column(Modifier.fillMaxSize().padding(padding)) {
                if (!fullScreen) ConnectionBanner()
                NavHost(nav, start, Modifier.weight(1f)) {
                    composable(Routes.HOME) { HomeScreen() }
                    composable(Routes.ARTISTS) { ArtistsScreen() }
                    composable(Routes.PLAYLISTS) { PlaylistsScreen() }
                    composable(Routes.SEARCH) { SearchScreen() }
                    composable(Routes.MORE) { MoreScreen() }
                    composable(Routes.NOW) { NowPlayingScreen() }
                    composable(Routes.SERVERS) { ServersScreen() }
                    composable("server/{id}") { ServerEditScreen(it.arg("id")) }
                    composable("album/{id}") { AlbumScreen(it.arg("id")) }
                    composable("artist/{id}") { ArtistScreen(it.arg("id")) }
                    composable("playlist/{id}") { PlaylistScreen(it.arg("id")) }
                    composable(Routes.GENRES) { GenresScreen() }
                    composable("genre/{name}") { GenreScreen(it.arg("name")) }
                    composable("folder/{id}") { FolderScreen(it.arg("id").takeIf { id -> id != "-" }) }
                    composable(Routes.PODCASTS) { PodcastsScreen() }
                    composable("podcast/{id}") { PodcastScreen(it.arg("id")) }
                    composable(Routes.BOOKMARKS) { BookmarksScreen() }
                    composable(Routes.SHARES) { SharesScreen() }
                    composable(Routes.DOWNLOADS) { DownloadsScreen() }
                    composable(Routes.JUKEBOX) { JukeboxScreen() }
                    composable(Routes.SETTINGS) { SettingsScreen() }
                }
            }
        }
    }
}

private fun androidx.navigation.NavBackStackEntry.arg(name: String) = Uri.decode(arguments?.getString(name) ?: "")

/** A screen with a top bar; `back` shows the up arrow. */
@Composable
fun Screen(
    title: String,
    back: Boolean = true,
    actions: @Composable RowScope.() -> Unit = {},
    content: @Composable ColumnScope.() -> Unit,
) {
    val nav = LocalNav.current
    Column(Modifier.fillMaxSize()) {
        TopAppBar(
            title = { Text(title, maxLines = 1, overflow = TextOverflow.Ellipsis) },
            navigationIcon = {
                if (back) {
                    IconButton(onClick = { nav.popBackStack() }) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, stringResource(R.string.back))
                    }
                }
            },
            actions = actions,
            windowInsets = WindowInsets(0),
        )
        content()
    }
}

@Composable
private fun ConnectionBanner() {
    val c by Graph.servers.connection.collectAsState()
    val nav = LocalNav.current
    val scope = rememberCoroutineScope()
    val text = when (val x = c) {
        is Connection.Offline -> stringResource(R.string.offline_banner)
        is Connection.Failed -> stringResource(R.string.connect_failed, x.profile.name)
        is Connection.Connecting -> stringResource(R.string.connecting, x.profile.name)
        else -> null
    } ?: return
    Surface(color = MaterialTheme.colorScheme.secondaryContainer) {
        Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            Icon(Icons.Default.CloudOff, null, Modifier.size(18.dp))
            Text(text, Modifier.weight(1f).padding(start = 8.dp), style = MaterialTheme.typography.bodySmall)
            when (c) {
                is Connection.Offline -> if (!Graph.prefs.offlineMode.value) {
                    TextButton(onClick = { scope.launch { Graph.servers.connectActive() } }) { Text(stringResource(R.string.retry)) }
                }
                is Connection.Failed -> TextButton(onClick = { nav.navigate(Routes.SERVERS) }) { Text(stringResource(R.string.servers)) }
                else -> {}
            }
        }
    }
}

@Composable
private fun MiniPlayer(onOpen: () -> Unit) {
    val st by Graph.player.state.collectAsState()
    val position by Graph.player.position.collectAsState()
    val item = st?.current ?: return
    Surface(color = MaterialTheme.colorScheme.surfaceContainerHigh) {
        Column {
            val d = st?.duration ?: item.song.duration?.toDouble()
            if (d != null && d > 0) {
                LinearProgressIndicator(progress = { (position / d).toFloat().coerceIn(0f, 1f) }, modifier = Modifier.fillMaxWidth())
            }
            Row(
                Modifier.fillMaxWidth().testTag("miniplayer").clickable(onClick = onOpen).padding(8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Cover(item.song.coverArt, Modifier.size(44.dp), size = 96, corner = 4.dp)
                Column(Modifier.weight(1f).padding(horizontal = 12.dp)) {
                    Text(item.song.title, maxLines = 1, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.bodyMedium)
                    Text(item.song.artist, maxLines = 1, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.bodySmall)
                }
                val playing = st?.state == PlayState.PLAYING || st?.state == PlayState.BUFFERING
                IconButton(onClick = { Graph.player.togglePause() }) {
                    Icon(if (playing) Icons.Default.Pause else Icons.Default.PlayArrow, stringResource(if (playing) R.string.pause else R.string.play))
                }
                IconButton(onClick = { Graph.player.next() }) { Icon(Icons.Default.SkipNext, stringResource(R.string.next)) }
            }
        }
    }
}

/** Navigation bar insets for screens without the bottom bar. */
fun Modifier.navBarPadding() = this.navigationBarsPadding()
