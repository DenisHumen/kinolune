// SPDX-License-Identifier: MPL-2.0
package app.kinolune.player.media

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class SessionSnapshotTest {
    private val state = mapOf(
        "kind" to "music",
        "id" to "track-1",
        "title" to "Song",
        "subtitle" to "Artist",
        "album" to "",
        "artworkUri" to "/data/cover.jpg",
        "durationMs" to 200_000,
        "positionMs" to 10_000,
        "playing" to true,
        "rate" to 1.0,
        "hasNext" to true,
        "seekable" to true,
    )

    @Test
    fun parsesDartMap() {
        val s = SessionSnapshot.fromMap(state, nowMs = 1_000)
        assertEquals("Song", s.title)
        assertEquals("Artist", s.subtitle)
        assertEquals(null, s.album) // empty strings are absent
        assertEquals(200_000L, s.durationMs)
        assertTrue(s.hasNext && s.seekable && s.isMusic)
    }

    @Test
    fun extrapolatesWhilePlaying() {
        val s = SessionSnapshot.fromMap(state, nowMs = 1_000)
        assertEquals(12_000L, s.positionAt(3_000))
        assertEquals(200_000L, s.positionAt(10_000_000)) // capped at the duration
        val paused = s.copy(playing = false)
        assertEquals(10_000L, paused.positionAt(3_000))
        val fast = s.copy(rate = 2f)
        assertEquals(14_000L, fast.positionAt(3_000))
    }
}
