package net.xfbs.specto

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import org.junit.Assert.assertEquals
import org.junit.Test

/** Where dragged pages land ([pageDrop]). */
class PageDragTest {
    /** Pages 3 to 8 in a grid of three columns: cells 100 wide and 150 high, 20 apart, from (10, 10). */
    private val grid: Map<Int, Rect> = (3..8).associateWith { page ->
        val (column, row) = (page - 3) % 3 to (page - 3) / 3
        Rect(Offset(10f + column * 120f, 10f + row * 170f), androidx.compose.ui.geometry.Size(100f, 150f))
    }

    /** The same pages one below the other. */
    private val column: Map<Int, Rect> = (3..8).associateWith { page ->
        Rect(Offset(50f, 10f + (page - 3) * 170f), androidx.compose.ui.geometry.Size(100f, 150f))
    }

    private fun inGrid(x: Float, y: Float) = pageDrop(Offset(x, y), grid, oneColumn = false)?.position
    private fun inColumn(x: Float, y: Float) = pageDrop(Offset(x, y), column, oneColumn = true)?.position

    @Test
    fun besideTheNearestThumbnailOnTheSideOfItsMiddle() {
        // Page 4 is the middle one of the first row, from x = 130 to 230.
        assertEquals(4, inGrid(150f, 80f))
        assertEquals(5, inGrid(200f, 80f))
        // In the gap between two thumbnails: between them, from either side.
        assertEquals(5, inGrid(235f, 80f))
        assertEquals(5, inGrid(245f, 80f))
        // Between two rows, the nearer one counts.
        assertEquals(4, inGrid(150f, 165f))
        assertEquals(7, inGrid(150f, 175f))
    }

    @Test
    fun atTheEndsOfARow() {
        // Left of the first thumbnail of the second row: before it, which
        // is also after the last one of the first row.
        assertEquals(6, inGrid(0f, 250f))
        assertEquals(6, inGrid(345f, 80f))
        assertEquals(PageDrop(5, before = false), pageDrop(Offset(345f, 80f), grid, oneColumn = false))
        assertEquals(PageDrop(6, before = true), pageDrop(Offset(0f, 250f), grid, oneColumn = false))
    }

    @Test
    fun beyondWhatIsOnScreen() {
        // Above everything: before the first thumbnail on screen, whatever
        // the column; below: after the last.
        assertEquals(3, inGrid(300f, 0f))
        assertEquals(9, inGrid(20f, 400f))
        assertEquals(null, pageDrop(Offset(0f, 0f), emptyMap(), oneColumn = false))
    }

    @Test
    fun inOneColumnPagesFollowDownwards() {
        // Page 4 goes from y = 180 to 330.
        assertEquals(4, inColumn(100f, 200f))
        assertEquals(5, inColumn(100f, 300f))
        // Left or right of the column makes no difference.
        assertEquals(5, inColumn(0f, 300f))
        assertEquals(5, inColumn(500f, 300f))
        assertEquals(3, inColumn(100f, 0f))
        assertEquals(9, inColumn(100f, 5000f))
    }
}
