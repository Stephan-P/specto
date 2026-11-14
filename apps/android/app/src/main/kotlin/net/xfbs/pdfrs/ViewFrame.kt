package net.xfbs.pdfrs

import java.nio.ByteBuffer

/**
 * Everything needed to draw the view once, as planned in Rust. Rectangles are
 * in device pixels relative to the view's top-left corner.
 *
 * Kept in flat arrays rather than objects per page and tile: one of these is
 * created for every frame while scrolling.
 */
class ViewFrame private constructor(
    /** Every visible tile is sharp. */
    val complete: Boolean,
    /** Pages are drawn 1:1 from tiles at the exact device scale. */
    val settled: Boolean,
    /** One page at a time, swiped left and right (otherwise: one scrolling column). */
    val paged: Boolean,
    /** Pages are shown two side by side. */
    val twoPages: Boolean,
    /** With two pages: the first page stands on its own. */
    val coverAlone: Boolean,
    /** Zoom relative to the fitted state of the view mode: 1.0 when fitted. */
    val zoomFactor: Double,
    /** Zero-based index of the page the user is looking at. */
    val currentPage: Int,
    /** The last page shown with [currentPage]: itself, or with two pages the second of the pair. */
    val lastPage: Int,
    /** Plan another frame after this many milliseconds, or -1. */
    val settlesInMs: Int,
    /** dp per PDF point. */
    val zoom: Double,
    /** Device pixels per PDF point. */
    val deviceScale: Double,
    /** Tiles dropped from the cache: release their bitmaps. */
    val evicted: LongArray,
    val pageCount: Int,
    private val pageIndex: IntArray,
    private val pageRects: FloatArray,
    private val tileStart: IntArray,
    private val tileIds: LongArray,
    private val tileRects: FloatArray,
    private val tileExact: BooleanArray,
    private val thumbnailFirst: Int,
    private val thumbnails: LongArray,
) {
    /**
     * The tile to show as thumbnail of [page] (zero-based), or 0 if it isn't
     * rendered yet or [page] is outside the range last passed to
     * [PdfViewState.showThumbnails].
     */
    fun thumbnailId(page: Int): Long = thumbnails.getOrElse(page - thumbnailFirst) { 0 }

    /** Whether [other] reports the same thumbnails for the same pages. */
    fun sameThumbnailsAs(other: ViewFrame?): Boolean =
        other != null && thumbnailFirst == other.thumbnailFirst && thumbnails.contentEquals(other.thumbnails)

    /** Zero-based page index of the [i]th visible page. */
    fun page(i: Int): Int = pageIndex[i]

    fun pageLeft(i: Int): Float = pageRects[4 * i]
    fun pageTop(i: Int): Float = pageRects[4 * i + 1]
    fun pageWidth(i: Int): Float = pageRects[4 * i + 2]
    fun pageHeight(i: Int): Float = pageRects[4 * i + 3]

    /** Tiles of the [i]th visible page, to draw in order: indices for the `tile*` accessors. */
    fun tiles(i: Int): IntRange = tileStart[i] until tileStart[i + 1]

    fun tileId(t: Int): Long = tileIds[t]
    fun tileLeft(t: Int): Float = tileRects[4 * t]
    fun tileTop(t: Int): Float = tileRects[4 * t + 1]
    fun tileRight(t: Int): Float = tileRects[4 * t] + tileRects[4 * t + 2]
    fun tileBottom(t: Int): Float = tileRects[4 * t + 1] + tileRects[4 * t + 3]

    /** The tile maps 1:1 onto device pixels: draw without filtering. */
    fun tileIsExact(t: Int): Boolean = tileExact[t]

    companion object {
        private const val PAGE_BYTES = 24
        private const val TILE_BYTES = 32

        /** Reads the layout documented in crates/pdfrs-ffi/src/hot.rs ([b] in native byte order). */
        fun decode(b: ByteBuffer, length: Int): ViewFrame {
            b.position(0)
            val flags = b.getInt()
            b.getInt() // zoom mode
            val currentPage = b.getInt()
            val settlesInMs = b.getInt()
            val lastPage = b.getInt()
            b.getInt()
            val zoom = b.getDouble()
            val deviceScale = b.getDouble()
            val zoomFactor = b.getDouble()
            val evicted = LongArray(b.getInt())
            val pages = b.getInt()
            for (i in evicted.indices) evicted[i] = b.getLong()

            // Tiles are at most what is left, so the arrays can be sized up front.
            val maxTiles = (length - b.position() - pages * PAGE_BYTES) / TILE_BYTES + 1
            val pageIndex = IntArray(pages)
            val pageRects = FloatArray(4 * pages)
            val tileStart = IntArray(pages + 1)
            val tileIds = LongArray(maxTiles)
            val tileRects = FloatArray(4 * maxTiles)
            val tileExact = BooleanArray(maxTiles)
            var t = 0
            for (p in 0 until pages) {
                pageIndex[p] = b.getInt()
                val tiles = b.getInt()
                for (k in 0 until 4) pageRects[4 * p + k] = b.getFloat()
                tileStart[p] = t
                repeat(tiles) {
                    tileIds[t] = b.getLong()
                    for (k in 0 until 4) tileRects[4 * t + k] = b.getFloat()
                    tileExact[t] = b.getInt() != 0
                    b.getInt()
                    t++
                }
            }
            tileStart[pages] = t
            val thumbnailFirst = b.getInt()
            val thumbnails = LongArray(b.getInt())
            for (i in thumbnails.indices) thumbnails[i] = b.getLong()
            return ViewFrame(
                complete = flags and 1 != 0,
                settled = flags and 2 != 0,
                paged = flags and 4 != 0,
                twoPages = flags and 8 != 0,
                coverAlone = flags and 16 != 0,
                zoomFactor = zoomFactor,
                currentPage = currentPage,
                lastPage = lastPage,
                settlesInMs = settlesInMs,
                zoom = zoom,
                deviceScale = deviceScale,
                evicted = evicted,
                pageCount = pages,
                pageIndex = pageIndex,
                pageRects = pageRects,
                tileStart = tileStart,
                tileIds = tileIds,
                tileRects = tileRects,
                tileExact = tileExact,
                thumbnailFirst = thumbnailFirst,
                thumbnails = thumbnails,
            )
        }
    }
}
