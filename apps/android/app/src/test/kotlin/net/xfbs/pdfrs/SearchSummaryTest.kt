package net.xfbs.pdfrs

import org.junit.Assert.assertEquals
import org.junit.Test
import uniffi.pdfrs_ffi.SearchStatus

class SearchSummaryTest {
    private fun status(matches: Int, current: Int?, searched: Int, pages: Int = 100) =
        SearchStatus(matches.toUInt(), current?.toUInt(), searched.toUInt(), pages.toUInt())

    @Test
    fun noSearchSaysNothing() {
        assertEquals("", searchSummary(null))
    }

    @Test
    fun countsFromOne() {
        assertEquals("1 of 17", searchSummary(status(17, 0, 100)))
        assertEquals("17 of 17", searchSummary(status(17, 16, 100)))
    }

    @Test
    fun saysWhenMoreMayCome() {
        assertEquals("3 of 17…", searchSummary(status(17, 2, 40)))
        // Nothing yet is not the same as nothing at all.
        assertEquals("…", searchSummary(status(0, null, 40)))
        assertEquals("No matches", searchSummary(status(0, null, 100)))
    }
}
