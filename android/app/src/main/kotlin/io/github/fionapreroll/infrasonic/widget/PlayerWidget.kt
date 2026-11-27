// SPDX-License-Identifier: GPL-3.0-only
package io.github.fionapreroll.infrasonic.widget

import android.app.PendingIntent
import android.appwidget.AppWidgetManager
import android.appwidget.AppWidgetProvider
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.view.KeyEvent
import android.widget.RemoteViews
import androidx.annotation.OptIn
import androidx.media3.common.util.UnstableApi
import androidx.media3.session.MediaButtonReceiver
import io.github.fionapreroll.infrasonic.R
import io.github.fionapreroll.infrasonic.data.Graph
import io.github.fionapreroll.infrasonic.ui.MainActivity
import kotlinx.coroutines.launch

/**
 * Home screen widget: cover, title and the three buttons. The buttons send media button
 * events, which start the playback service if needed (and resume the last queue).
 */
class PlayerWidget : AppWidgetProvider() {
    override fun onUpdate(context: Context, manager: AppWidgetManager, ids: IntArray) {
        update(context)
    }

    companion object {
        private const val COVER_SIZE = 256
        private var listening = false

        /** Keep widgets in step with the player; call once the app runs. */
        fun follow(context: Context) {
            if (listening) return
            listening = true
            val app = context.applicationContext
            Graph.player.addListener { update(app) }
        }

        @OptIn(UnstableApi::class)
        private fun button(context: Context, keyCode: Int): PendingIntent {
            val intent = Intent(Intent.ACTION_MEDIA_BUTTON)
                .setComponent(ComponentName(context, MediaButtonReceiver::class.java))
                .putExtra(Intent.EXTRA_KEY_EVENT, KeyEvent(KeyEvent.ACTION_DOWN, keyCode))
            return PendingIntent.getBroadcast(context, keyCode, intent, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        }

        fun update(context: Context) {
            val manager = AppWidgetManager.getInstance(context)
            val ids = manager.getAppWidgetIds(ComponentName(context, PlayerWidget::class.java))
            if (ids.isEmpty()) return
            Graph.init(context.applicationContext as android.app.Application)
            val st = Graph.player.state.value
            val song = st?.current?.song
            val views = RemoteViews(context.packageName, R.layout.widget_player)
            views.setTextViewText(R.id.widget_title, song?.title ?: context.getString(R.string.nothing_playing))
            views.setTextViewText(R.id.widget_artist, song?.artist ?: "")
            // The cover as a bitmap: the launcher draws the widget and has no access to the
            // app's covers. Not loaded yet: show the placeholder and update again once it is.
            val coverId = song?.coverArt
            val cover = coverId?.let { Graph.covers.cached(it, COVER_SIZE) }
            if (cover != null) views.setImageViewBitmap(R.id.widget_cover, cover) else views.setImageViewResource(R.id.widget_cover, R.drawable.ic_album)
            if (coverId != null && cover == null) {
                val app = context.applicationContext
                Graph.scope.launch { if (Graph.covers.bitmap(coverId, COVER_SIZE) != null) update(app) }
            }
            views.setImageViewResource(
                R.id.widget_play,
                if (Graph.player.isPlaying) R.drawable.ic_pause else R.drawable.ic_play,
            )
            views.setOnClickPendingIntent(R.id.widget_play, button(context, KeyEvent.KEYCODE_MEDIA_PLAY_PAUSE))
            views.setOnClickPendingIntent(R.id.widget_next, button(context, KeyEvent.KEYCODE_MEDIA_NEXT))
            views.setOnClickPendingIntent(R.id.widget_previous, button(context, KeyEvent.KEYCODE_MEDIA_PREVIOUS))
            val open = PendingIntent.getActivity(
                context, 0, Intent(context, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE,
            )
            views.setOnClickPendingIntent(R.id.widget_root, open)
            manager.updateAppWidget(ids, views)
        }
    }
}
