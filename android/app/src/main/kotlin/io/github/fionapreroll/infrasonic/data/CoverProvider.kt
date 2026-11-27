// SPDX-License-Identifier: GPL-3.0-only
package io.github.fionapreroll.infrasonic.data

import android.content.ContentProvider
import android.content.ContentValues
import android.database.Cursor
import android.net.Uri
import android.os.ParcelFileDescriptor
import java.io.FileNotFoundException

/** Read-only cover art for the notification, Android Auto and the widget. */
class CoverProvider : ContentProvider() {
    override fun onCreate() = true

    override fun openFile(uri: Uri, mode: String): ParcelFileDescriptor {
        if (mode != "r") throw SecurityException("read only")
        val app = context?.applicationContext as? android.app.Application ?: throw FileNotFoundException()
        Graph.init(app)
        val id = uri.lastPathSegment ?: throw FileNotFoundException()
        val size = uri.getQueryParameter("size")?.toIntOrNull()?.coerceIn(32, 1024) ?: 512
        val f = Graph.covers.fileBlocking(id, size) ?: throw FileNotFoundException(id)
        return ParcelFileDescriptor.open(f, ParcelFileDescriptor.MODE_READ_ONLY)
    }

    override fun getType(uri: Uri) = "image/*"
    override fun query(uri: Uri, p: Array<out String>?, s: String?, a: Array<out String>?, o: String?): Cursor? = null
    override fun insert(uri: Uri, values: ContentValues?): Uri? = null
    override fun delete(uri: Uri, s: String?, a: Array<out String>?) = 0
    override fun update(uri: Uri, v: ContentValues?, s: String?, a: Array<out String>?) = 0
}
