// SPDX-License-Identifier: MPL-2.0
package app.kinolune.tv

import android.content.ContentProvider
import android.content.ContentValues
import android.content.Context
import android.database.Cursor
import android.net.Uri
import android.os.ParcelFileDescriptor
import java.io.File
import java.io.FileNotFoundException

/**
 * Card art of the Watch Next row (PLAY-AND-009, ADR-NNNN) for the launcher, which runs in another process and
 * cannot read the app's files.
 *
 * Read-only and limited to the flat directory `cache/tv_art`, where the app writes only artwork it rendered or
 * copied for the row (no media, no paths from outside): the launcher's package is not known in advance, so the
 * provider is exported without a permission instead of granting URIs to it.
 */
class TvArtProvider : ContentProvider() {
    companion object {
        const val DIR = "tv_art"
        private val NAME = Regex("[A-Za-z0-9_-]{1,64}\\.(png|jpg|jpeg|webp)")

        fun authority(context: Context) = "${context.packageName}.tvart"

        /** The content URI of [file], or null when it is not a launcher art file. */
        fun uriFor(context: Context, file: File): Uri? {
            val dir = File(context.cacheDir, DIR).canonicalFile
            val f = file.canonicalFile
            if (f.parentFile != dir || !NAME.matches(f.name) || !f.isFile) return null
            // The modification time busts the launcher's image cache when the art is redrawn.
            return Uri.Builder().scheme("content").authority(authority(context))
                .appendPath(f.name).appendQueryParameter("v", f.lastModified().toString()).build()
        }

        internal fun mimeOf(name: String): String = when (name.substringAfterLast('.').lowercase()) {
            "jpg", "jpeg" -> "image/jpeg"
            "webp" -> "image/webp"
            else -> "image/png"
        }
    }

    override fun onCreate(): Boolean = true

    private fun fileOf(uri: Uri): File {
        val name = uri.pathSegments.singleOrNull()
        if (name == null || !NAME.matches(name)) throw FileNotFoundException(uri.toString())
        val f = File(File(context!!.cacheDir, DIR), name)
        if (!f.isFile) throw FileNotFoundException(uri.toString())
        return f
    }

    override fun openFile(uri: Uri, mode: String): ParcelFileDescriptor {
        if (mode != "r") throw SecurityException("read-only")
        return ParcelFileDescriptor.open(fileOf(uri), ParcelFileDescriptor.MODE_READ_ONLY)
    }

    override fun getType(uri: Uri): String? = uri.lastPathSegment?.let { mimeOf(it) }

    override fun query(uri: Uri, p: Array<out String>?, s: String?, a: Array<out String>?, o: String?): Cursor? = null
    override fun insert(uri: Uri, values: ContentValues?): Uri? = throw SecurityException("read-only")
    override fun delete(uri: Uri, s: String?, a: Array<out String>?): Int = throw SecurityException("read-only")
    override fun update(uri: Uri, v: ContentValues?, s: String?, a: Array<out String>?): Int =
        throw SecurityException("read-only")
}
