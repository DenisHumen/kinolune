// SPDX-License-Identifier: MPL-2.0
package app.kinolune.tv

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class WatchNextPlanTest {
    private fun entry(key: String, at: Long, next: WatchNextEntry.Next = WatchNextEntry.Next.CONTINUE) =
        WatchNextEntry(key = key, kind = WatchNextEntry.Kind.MOVIE, next = next, title = key, engagementMs = at, link = "kinolune://play/$key")

    @Test
    fun newEntriesAreInsertedMostRecentFirst() {
        val c = WatchNextPlan.plan(listOf(entry("a", 1), entry("b", 3), entry("c", 2)), emptyList(), emptyMap())
        assertEquals(listOf("b", "c", "a"), c.inserts.map { it.key })
        assertTrue(c.updates.isEmpty())
        assertTrue(c.deletes.isEmpty())
    }

    @Test
    fun knownEntriesAreUpdatedAndGoneOnesDeleted() {
        val published = listOf(
            PublishedProgram(10, "a", true, 1),
            PublishedProgram(11, "gone", true, 1),
            PublishedProgram(12, "a", true, 1), // a duplicate row of the same key
            PublishedProgram(13, null, true, 1), // not written by this app's sync
        )
        val c = WatchNextPlan.plan(listOf(entry("a", 5)), published, emptyMap())
        assertTrue(c.inserts.isEmpty())
        assertEquals(listOf(10L to "a"), c.updates.map { it.first to it.second.key })
        assertEquals(setOf(11L, 12L, 13L), c.deletes.toSet())
    }

    @Test
    fun aCardRemovedInTheLauncherStaysAwayUntilPlayedAgain() {
        val removed = listOf(PublishedProgram(20, "a", browsable = false, engagementMs = 5))
        var c = WatchNextPlan.plan(listOf(entry("a", 5)), removed, emptyMap())
        assertEquals(listOf(20L), c.deletes)
        assertTrue(c.inserts.isEmpty())
        assertEquals(mapOf("a" to 5L), c.dismissed)

        // Next sync: the row is gone, the entry is still the same.
        c = WatchNextPlan.plan(listOf(entry("a", 5)), emptyList(), c.dismissed)
        assertTrue(c.inserts.isEmpty())
        assertEquals(mapOf("a" to 5L), c.dismissed)

        // Played again: back in the row, no longer dismissed.
        c = WatchNextPlan.plan(listOf(entry("a", 9)), emptyList(), c.dismissed)
        assertEquals(listOf("a"), c.inserts.map { it.key })
        assertNull(c.dismissed["a"])
    }

    @Test
    fun dismissalsOfEntriesNoLongerSentAreForgotten() {
        val c = WatchNextPlan.plan(listOf(entry("b", 1)), emptyList(), mapOf("a" to 5L))
        assertTrue(c.dismissed.isEmpty())
    }

    @Test
    fun theRowIsCappedAndDuplicatesDropped() {
        val wanted = (1..40).map { entry("k$it", it.toLong()) } + entry("k40", 0)
        val c = WatchNextPlan.plan(wanted, emptyList(), emptyMap(), limit = 30)
        assertEquals(30, c.inserts.size)
        assertEquals("k40", c.inserts.first().key)
        assertEquals(40L, c.inserts.first().engagementMs)
    }

    @Test
    fun entriesComeFromDartMaps() {
        val e = WatchNextEntry.fromMap(
            mapOf(
                "key" to "ep1", "kind" to "episode", "next" to "next", "title" to "Show", "season" to 2,
                "episode" to 3, "positionMs" to 1000, "durationMs" to 2000L, "engagementMs" to 7L,
                "art" to "https://example.org/a.jpg", "link" to "kinolune://play/ep1",
            ),
        )!!
        assertEquals(WatchNextEntry.Kind.EPISODE, e.kind)
        assertEquals(WatchNextEntry.Next.NEXT, e.next)
        assertEquals(2, e.season)
        assertEquals(1000L, e.positionMs)
        assertNull(WatchNextEntry.fromMap(mapOf("key" to "x")))
    }

    @Test
    fun intentsBecomeAppLinks() {
        assertEquals("kinolune://play/a", TvLinks.linkOf("android.intent.action.VIEW", "kinolune://play/a", null))
        assertNull(TvLinks.linkOf("android.intent.action.VIEW", "https://example.org", null))
        assertEquals("kinolune://search?q=star+wars", TvLinks.linkOf("android.intent.action.SEARCH", null, " star wars "))
        assertEquals("kinolune://search?q=%D0%BA%D0%BE%D1%82", TvLinks.linkOf(TvLinks.GMS_SEARCH, null, "кот"))
        assertNull(TvLinks.linkOf("android.intent.action.SEARCH", null, "  "))
        assertNull(TvLinks.linkOf("android.intent.action.MAIN", null, null))
    }
}
