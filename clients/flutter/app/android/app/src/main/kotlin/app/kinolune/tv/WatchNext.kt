// SPDX-License-Identifier: MPL-2.0
package app.kinolune.tv

import android.annotation.SuppressLint
import android.content.Context
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.util.Log
import androidx.tvprovider.media.tv.TvContractCompat
import androidx.tvprovider.media.tv.WatchNextProgram
import org.json.JSONObject
import java.io.File

/**
 * The system Watch Next row of Android TV and Google TV (PLAY-AND-009, ADR-NNNN), written through the TV provider
 * with `androidx.tvprovider`. Only this app's own programs are read and written (API 26+ needs no permission for
 * that). Runs off the main thread: provider calls block.
 */
object WatchNext {
    private const val TAG = "WatchNext"
    private const val PREFS = "tv_watch_next"
    private const val DISMISSED = "dismissed"

    /** Whether this device has a Watch Next row to write to. */
    fun supported(context: Context): Boolean =
        Build.VERSION.SDK_INT >= Build.VERSION_CODES.O &&
            context.packageManager.hasSystemFeature(PackageManager.FEATURE_LEANBACK)

    /** Makes the row show [entries]: inserts, rewrites and deletes this app's programs. */
    @SuppressLint("RestrictedApi")
    fun publish(context: Context, entries: List<WatchNextEntry>) {
        if (!supported(context)) return
        val resolver = context.contentResolver
        val published = mutableListOf<PublishedProgram>()
        try {
            resolver.query(
                TvContractCompat.WatchNextPrograms.CONTENT_URI,
                WatchNextProgram.PROJECTION,
                null,
                null,
                null,
            )?.use { c ->
                while (c.moveToNext()) {
                    val p = WatchNextProgram.fromCursor(c)
                    published += PublishedProgram(
                        rowId = p.id,
                        key = p.internalProviderId,
                        browsable = p.isBrowsable,
                        engagementMs = p.lastEngagementTimeUtcMillis,
                    )
                }
            }
        } catch (e: Exception) {
            // No TV provider (a phone with the leanback feature faked, a launcher without Watch Next).
            Log.w(TAG, "cannot read the Watch Next programs: $e")
            return
        }
        val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        val changes = WatchNextPlan.plan(entries, published, readDismissed(prefs.getString(DISMISSED, null)))
        var failed = 0
        for (row in changes.deletes) {
            try {
                resolver.delete(TvContractCompat.buildWatchNextProgramUri(row), null, null)
            } catch (e: Exception) {
                failed++
                Log.w(TAG, "delete $row: $e")
            }
        }
        for ((row, e) in changes.updates) {
            try {
                resolver.update(TvContractCompat.buildWatchNextProgramUri(row), program(context, e).toContentValues(), null, null)
            } catch (ex: Exception) {
                failed++
                Log.w(TAG, "update ${e.key}: $ex")
            }
        }
        for (e in changes.inserts) {
            try {
                resolver.insert(TvContractCompat.WatchNextPrograms.CONTENT_URI, program(context, e).toContentValues())
            } catch (ex: Exception) {
                failed++
                Log.w(TAG, "insert ${e.key}: $ex")
            }
        }
        prefs.edit().putString(DISMISSED, writeDismissed(changes.dismissed)).apply()
        Log.i(
            TAG,
            "Watch Next: ${changes.inserts.size} added, ${changes.updates.size} updated, " +
                "${changes.deletes.size} removed, ${changes.dismissed.size} dismissed" +
                if (failed > 0) ", $failed failed" else "",
        )
    }

    private fun program(context: Context, e: WatchNextEntry): WatchNextProgram {
        val b = WatchNextProgram.Builder()
            .setType(
                when (e.kind) {
                    WatchNextEntry.Kind.MOVIE -> TvContractCompat.PreviewProgramColumns.TYPE_MOVIE
                    WatchNextEntry.Kind.EPISODE -> TvContractCompat.PreviewProgramColumns.TYPE_TV_EPISODE
                    WatchNextEntry.Kind.CLIP -> TvContractCompat.PreviewProgramColumns.TYPE_CLIP
                },
            )
            .setWatchNextType(
                when (e.next) {
                    WatchNextEntry.Next.CONTINUE -> TvContractCompat.WatchNextPrograms.WATCH_NEXT_TYPE_CONTINUE
                    WatchNextEntry.Next.NEXT -> TvContractCompat.WatchNextPrograms.WATCH_NEXT_TYPE_NEXT
                },
            )
            .setLastEngagementTimeUtcMillis(e.engagementMs)
            .setTitle(e.title)
            .setInternalProviderId(e.key)
            .setIntentUri(Uri.parse(e.link))
            .setPosterArtAspectRatio(TvContractCompat.PreviewProgramColumns.ASPECT_RATIO_16_9)
        e.episodeTitle?.let { b.setEpisodeTitle(it) }
        e.season?.let { b.setSeasonNumber(it) }
        e.episode?.let { b.setEpisodeNumber(it) }
        if (e.durationMs > 0) b.setDurationMillis(e.durationMs.coerceAtMost(Int.MAX_VALUE.toLong()).toInt())
        if (e.next == WatchNextEntry.Next.CONTINUE && e.positionMs > 0) {
            b.setLastPlaybackPositionMillis(e.positionMs.coerceAtMost(Int.MAX_VALUE.toLong()).toInt())
        }
        artUri(context, e.art)?.let { b.setPosterArtUri(it) }
        return b.build()
    }

    /** An http(s) URL as is; a file the app rendered for the launcher through [TvArtProvider]. */
    private fun artUri(context: Context, art: String?): Uri? {
        if (art.isNullOrEmpty()) return null
        if (art.startsWith("https://") || art.startsWith("http://")) return Uri.parse(art)
        return TvArtProvider.uriFor(context, File(art))
    }

    internal fun readDismissed(json: String?): Map<String, Long> {
        if (json.isNullOrEmpty()) return emptyMap()
        return try {
            val o = JSONObject(json)
            o.keys().asSequence().associateWith { o.getLong(it) }
        } catch (_: Exception) {
            emptyMap()
        }
    }

    internal fun writeDismissed(m: Map<String, Long>): String = JSONObject(m as Map<*, *>).toString()
}
