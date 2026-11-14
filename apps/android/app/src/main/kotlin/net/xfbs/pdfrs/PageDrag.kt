package net.xfbs.pdfrs

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect

/**
 * Where dragged pages would land: beside the cell of page [cell], before it
 * or after it.
 */
data class PageDrop(val cell: Int, val before: Boolean) {
    /** The position the pages are moved to: they end up before the page now there. */
    val position: Int get() = if (before) cell else cell + 1
}

/**
 * Where pages dragged to [pointer] would land among [cells] (the thumbnails
 * on screen, by page): beside the nearest one, on the side of its middle
 * the pointer is on. Pages follow each other left to right, or with
 * [oneColumn] top to bottom. Null if there are no cells.
 */
internal fun pageDrop(pointer: Offset, cells: Map<Int, Rect>, oneColumn: Boolean): PageDrop? {
    val nearest = cells.minByOrNull { (_, cell) ->
        val dx = maxOf(cell.left - pointer.x, 0f, pointer.x - cell.right)
        val dy = maxOf(cell.top - pointer.y, 0f, pointer.y - cell.bottom)
        dx * dx + dy * dy
    } ?: return null
    val cell = nearest.value
    // Below the last row or above the first, the nearest cell is the one in
    // the pointer's column, which is the wrong measure there: after
    // everything, or before it.
    val (first, last) = cells.keys.min() to cells.keys.max()
    if (!oneColumn && pointer.y > cells.getValue(last).bottom) return PageDrop(last, before = false)
    if (!oneColumn && pointer.y < cells.getValue(first).top) return PageDrop(first, before = true)
    val before = if (oneColumn) pointer.y < cell.center.y else pointer.x < cell.center.x
    return PageDrop(nearest.key, before)
}

/** What the page grid knows while its pages are being dragged about. */
internal class PageDragState {
    /** Where each thumbnail on screen is, by page, in the coordinates of the grid's root. */
    val cells = mutableStateMapOf<Int, Rect>()

    /** Where the finger dragging pages is, in the same coordinates; null while nothing is dragged. */
    var pointer: Offset? by mutableStateOf(null)

    fun drop(oneColumn: Boolean): PageDrop? = pointer?.let { pageDrop(it, cells, oneColumn) }
}
