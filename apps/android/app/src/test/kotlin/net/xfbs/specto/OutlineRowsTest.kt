package net.xfbs.specto

import org.junit.Assert.assertEquals
import org.junit.Test
import uniffi.pdfrs_ffi.OutlineEntry

class OutlineRowsTest {
    private fun entry(title: String, depth: Int, hasChildren: Boolean = false) =
        OutlineEntry(title, depth.toUInt(), page = null, hasChildren = hasChildren)

    /**
     * ```
     * 0 A
     * 1   A.1
     * 2   A.2
     * 3     A.2.a
     * 4 B
     * 5 C
     * 6   C.1
     * ```
     */
    private val entries = listOf(
        entry("A", 0, hasChildren = true),
        entry("A.1", 1),
        entry("A.2", 1, hasChildren = true),
        entry("A.2.a", 2),
        entry("B", 0),
        entry("C", 0, hasChildren = true),
        entry("C.1", 1),
    )

    @Test
    fun collapsedShowsTheTopLevel() {
        assertEquals(listOf(0, 4, 5), visibleOutlineRows(entries, emptySet()))
    }

    @Test
    fun expandingShowsChildrenButNotGrandchildren() {
        assertEquals(listOf(0, 1, 2, 4, 5), visibleOutlineRows(entries, setOf(0)))
        assertEquals(listOf(0, 1, 2, 3, 4, 5), visibleOutlineRows(entries, setOf(0, 2)))
    }

    @Test
    fun anExpandedEntryInsideACollapsedOneStaysHidden() {
        assertEquals(listOf(0, 4, 5), visibleOutlineRows(entries, setOf(2)))
    }

    @Test
    fun everythingExpanded() {
        assertEquals(entries.indices.toList(), visibleOutlineRows(entries, setOf(0, 2, 5)))
    }

    @Test
    fun emptyOutline() {
        assertEquals(emptyList<Int>(), visibleOutlineRows(emptyList(), emptySet()))
    }
}
