// SPDX-License-Identifier: MPL-2.0
package app.kinolune.player.media

/**
 * What the Dart player last reported to the system session (PLAY-AND-008), see `MediaSessionInfo.toMap` in
 * `pa_player/lib/src/system/media_session.dart`. [receivedAtMs] is `SystemClock.elapsedRealtime()` of the
 * report: the session extrapolates the playhead from it while playing.
 */
data class SessionSnapshot(
    val kind: String = "music",
    val id: String = "",
    val title: String = "",
    val subtitle: String? = null,
    val album: String? = null,
    val artworkUri: String? = null,
    val durationMs: Long = 0,
    val positionMs: Long = 0,
    val playing: Boolean = false,
    val buffering: Boolean = false,
    val ended: Boolean = false,
    val rate: Float = 1f,
    val hasNext: Boolean = false,
    val hasPrevious: Boolean = false,
    val seekable: Boolean = false,
    val receivedAtMs: Long = 0,
) {
    val isEmpty: Boolean get() = id.isEmpty() && title.isEmpty()
    val isLive: Boolean get() = kind == "live"
    val isMusic: Boolean get() = kind == "music"

    /** The playhead at [nowMs]: moves on from the report while playing. */
    fun positionAt(nowMs: Long): Long {
        if (!playing || buffering || ended) return positionMs
        val moved = positionMs + ((nowMs - receivedAtMs).coerceAtLeast(0) * rate).toLong()
        return if (durationMs > 0) moved.coerceAtMost(durationMs) else moved
    }

    companion object {
        val EMPTY = SessionSnapshot()

        fun fromMap(m: Map<*, *>, nowMs: Long): SessionSnapshot = SessionSnapshot(
            kind = m["kind"] as? String ?: "music",
            id = m["id"] as? String ?: "",
            title = m["title"] as? String ?: "",
            subtitle = (m["subtitle"] as? String)?.takeIf { it.isNotEmpty() },
            album = (m["album"] as? String)?.takeIf { it.isNotEmpty() },
            artworkUri = (m["artworkUri"] as? String)?.takeIf { it.isNotEmpty() },
            durationMs = (m["durationMs"] as? Number)?.toLong() ?: 0,
            positionMs = (m["positionMs"] as? Number)?.toLong() ?: 0,
            playing = m["playing"] == true,
            buffering = m["buffering"] == true,
            ended = m["ended"] == true,
            rate = (m["rate"] as? Number)?.toFloat()?.takeIf { it > 0f } ?: 1f,
            hasNext = m["hasNext"] == true,
            hasPrevious = m["hasPrevious"] == true,
            seekable = m["seekable"] == true,
            receivedAtMs = nowMs,
        )
    }
}
