// SPDX-License-Identifier: MPL-2.0
package app.kinolune.tv

/**
 * A card of the system Watch Next row (PLAY-AND-009, ADR-NNNN) as Dart sends it: an entry of the app's Continue
 * row ([Next.CONTINUE]) or of its Up Next row ([Next.NEXT]).
 */
data class WatchNextEntry(
    /** Stable key of the entry (the library item or Twitch VOD id); the program's internal provider id. */
    val key: String,
    val kind: Kind,
    val next: Next,
    val title: String,
    val episodeTitle: String? = null,
    val season: Int? = null,
    val episode: Int? = null,
    val positionMs: Long = 0,
    val durationMs: Long = 0,
    /** When it was last played (Up Next: when the episode before it was finished), UTC milliseconds. */
    val engagementMs: Long = 0,
    /** Card art: an http(s) URL or a file the app rendered for the launcher (see [TvArtProvider]). */
    val art: String? = null,
    /** What the card opens: a `kinolune://` link (see TvBridge). */
    val link: String,
) {
    enum class Kind { MOVIE, EPISODE, CLIP }
    enum class Next { CONTINUE, NEXT }

    companion object {
        fun fromMap(m: Map<*, *>): WatchNextEntry? {
            val key = m["key"] as? String ?: return null
            val title = m["title"] as? String ?: return null
            val link = m["link"] as? String ?: return null
            fun long(k: String) = (m[k] as? Number)?.toLong() ?: 0
            return WatchNextEntry(
                key = key,
                kind = when (m["kind"]) {
                    "episode" -> Kind.EPISODE
                    "clip" -> Kind.CLIP
                    else -> Kind.MOVIE
                },
                next = if (m["next"] == "next") Next.NEXT else Next.CONTINUE,
                title = title,
                episodeTitle = m["episodeTitle"] as? String,
                season = (m["season"] as? Number)?.toInt(),
                episode = (m["episode"] as? Number)?.toInt(),
                positionMs = long("positionMs"),
                durationMs = long("durationMs"),
                engagementMs = long("engagementMs"),
                art = m["art"] as? String,
                link = link,
            )
        }
    }
}

/** A Watch Next program of this app already in the TV provider. */
data class PublishedProgram(
    val rowId: Long,
    /** The internal provider id: [WatchNextEntry.key]; null for rows the app did not write this way. */
    val key: String?,
    /** False once the user removed the card from the row in the launcher. */
    val browsable: Boolean,
    val engagementMs: Long,
)

/** What to change in the TV provider to show [WatchNextEntry]s. */
data class WatchNextChanges(
    val inserts: List<WatchNextEntry>,
    /** Rows to rewrite with the entry (position, art, title may have changed). */
    val updates: List<Pair<Long, WatchNextEntry>>,
    val deletes: List<Long>,
    /** Keys the user removed from the row, with the engagement they were removed at. */
    val dismissed: Map<String, Long>,
)

/**
 * The diff between the row the app wants and what the TV provider holds (pure, unit-tested).
 *
 * * Entries are matched to programs by key; a key's extra rows are deleted.
 * * A card the user removed in the launcher (browsable = 0) is deleted and remembered in [WatchNextChanges.dismissed]
 *   with its engagement time: the entry comes back only once it is played again (a later engagement), as a removed
 *   Watch Next card does in other apps. Remembered keys the app no longer sends are forgotten.
 * * At most [limit] entries, most recent engagement first.
 */
object WatchNextPlan {
    /** Watch Next rows of one app beyond about this many are cut by the launchers anyway. */
    const val LIMIT = 30

    fun plan(
        wanted: List<WatchNextEntry>,
        published: List<PublishedProgram>,
        dismissed: Map<String, Long>,
        limit: Int = LIMIT,
    ): WatchNextChanges {
        val stillDismissed = dismissed.toMutableMap()
        val deletes = mutableListOf<Long>()
        val live = mutableMapOf<String, PublishedProgram>()
        for (p in published) {
            val key = p.key
            when {
                key == null -> deletes += p.rowId
                !p.browsable -> {
                    stillDismissed[key] = maxOf(stillDismissed[key] ?: 0, p.engagementMs)
                    deletes += p.rowId
                }
                live.containsKey(key) -> deletes += p.rowId
                else -> live[key] = p
            }
        }
        val shown = wanted
            .distinctBy { it.key }
            .filter { e -> stillDismissed[e.key]?.let { e.engagementMs > it } ?: true }
            .sortedByDescending { it.engagementMs }
            .take(limit)
        val wantedKeys = wanted.mapTo(HashSet()) { it.key }
        // Played again after it was removed: the key is no longer dismissed.
        for (e in shown) stillDismissed.remove(e.key)
        stillDismissed.keys.retainAll(wantedKeys)
        val shownKeys = shown.mapTo(HashSet()) { it.key }
        val inserts = mutableListOf<WatchNextEntry>()
        val updates = mutableListOf<Pair<Long, WatchNextEntry>>()
        for (e in shown) {
            val p = live[e.key]
            if (p == null) inserts += e else updates += p.rowId to e
        }
        for ((key, p) in live) if (key !in shownKeys) deletes += p.rowId
        return WatchNextChanges(inserts, updates, deletes, stillDismissed)
    }
}
