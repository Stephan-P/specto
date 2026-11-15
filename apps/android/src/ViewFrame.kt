package net.xfbs.specto

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
    /** Text is selected, on screen or not. */
    val selection: Boolean,
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
    private val pageBusy: BooleanArray,
    private val tileStart: IntArray,
    private val tileIds: LongArray,
    private val tileRects: FloatArray,
    private val tileExact: BooleanArray,
    private val thumbnailFirst: Int,
    private val thumbnails: LongArray,
    /** Eight numbers per highlight: x and y of its four corners. */
    private val highlights: FloatArray,
    /** Why each is highlighted: one of the `HIGHLIGHT_*` constants. */
    private val highlightKinds: IntArray,
    /**
     * Where the selection starts, for its handle: the edge of its first
     * line as bottom x, bottom y, top x, top y. Null if that isn't on screen.
     */
    val selectionStart: FloatArray?,
    /** Where the selection ends, likewise. */
    val selectionEnd: FloatArray?,
    /** Five numbers per shape: its page, its flags, fill and stroke colour, and its first segment. */
    private val shapeHeads: IntArray,
    private val shapeWidths: FloatArray,
    /** Where each shape's outline starts: x and y. */
    private val shapeStarts: FloatArray,
    private val segmentCurved: BooleanArray,
    /** Six numbers per segment: x and y of up to three points. */
    private val segmentPoints: FloatArray,
    /**
     * The box around the annotation that is picked, as x and y of its four
     * corners, around it. Null if none is picked, or it isn't on screen.
     */
    val picked: FloatArray?,
) {
    /**
     * How many shapes there are to draw over the pages, under the
     * highlights: the annotations that aren't part of the pages as rendered
     * (those made or changed since the file was read). Each is an outline,
     * filled, drawn along with a pen, or both, and belongs to one page.
     */
    val shapeCount: Int get() = shapeWidths.size

    /** Which of the visible pages shape [i] is on (as for [page]): it is clipped to that page. */
    fun shapePage(i: Int): Int = shapeHeads[5 * i]

    fun shapeIsFilled(i: Int): Boolean = shapeHeads[5 * i + 1] and 1 != 0
    fun shapeIsStroked(i: Int): Boolean = shapeHeads[5 * i + 1] and 2 != 0

    /** The fill darkens what is under it, as a highlighter does, instead of covering it. */
    fun shapeDarkens(i: Int): Boolean = shapeHeads[5 * i + 1] and 4 != 0

    /** The outline ends where it starts. */
    fun shapeIsClosed(i: Int): Boolean = shapeHeads[5 * i + 1] and 8 != 0

    /** The pen is round: round ends and joins. */
    fun shapeIsRound(i: Int): Boolean = shapeHeads[5 * i + 1] and 16 != 0

    /** Colours as 0xAARRGGBB. */
    fun shapeFill(i: Int): Int = shapeHeads[5 * i + 2]
    fun shapeStroke(i: Int): Int = shapeHeads[5 * i + 3]
    fun shapeStrokeWidth(i: Int): Float = shapeWidths[i]

    fun shapeStartX(i: Int): Float = shapeStarts[2 * i]
    fun shapeStartY(i: Int): Float = shapeStarts[2 * i + 1]

    /** The pieces of shape [i]'s outline, each from where the one before ended: indices for the `segment*` accessors. */
    fun segments(i: Int): IntRange {
        val end = if (i + 1 < shapeCount) shapeHeads[5 * (i + 1) + 4] else segmentCurved.size
        return shapeHeads[5 * i + 4] until end
    }

    /** A cubic curve, with control points 0 and 1 and its end as point 2; else a line to point 0. */
    fun segmentIsCurve(s: Int): Boolean = segmentCurved[s]

    fun segmentX(s: Int, point: Int): Float = segmentPoints[6 * s + 2 * point]
    fun segmentY(s: Int, point: Int): Float = segmentPoints[6 * s + 2 * point + 1]

    /** What [picked] covers, as left, top, right, bottom. */
    fun pickedBounds(): FloatArray? {
        val p = picked ?: return null
        val xs = floatArrayOf(p[0], p[2], p[4], p[6])
        val ys = floatArrayOf(p[1], p[3], p[5], p[7])
        return floatArrayOf(xs.min(), ys.min(), xs.max(), ys.max())
    }

    /**
     * How many stretches of text on screen are highlighted, because they
     * are selected or the search found them; one per line. Each is a
     * quadrilateral (lines can be rotated) with the corners 0: start
     * bottom, 1: end bottom, 2: end top, 3: start top.
     */
    val highlightCount: Int get() = highlightKinds.size

    /** [HIGHLIGHT_SELECTION], [HIGHLIGHT_MATCH] or [HIGHLIGHT_CURRENT_MATCH]. */
    fun highlightKind(i: Int): Int = highlightKinds[i]

    fun highlightX(i: Int, corner: Int): Float = highlights[8 * i + 2 * corner]
    fun highlightY(i: Int, corner: Int): Float = highlights[8 * i + 2 * corner + 1]

    /** What the selected text on screen covers, as left, top, right, bottom; null if there is none. */
    fun selectionBounds(): FloatArray? {
        var bounds: FloatArray? = null
        for (h in highlightKinds.indices) {
            if (highlightKinds[h] != HIGHLIGHT_SELECTION) continue
            val b = bounds
                ?: floatArrayOf(Float.MAX_VALUE, Float.MAX_VALUE, -Float.MAX_VALUE, -Float.MAX_VALUE).also { bounds = it }
            for (i in 8 * h until 8 * h + 8 step 2) {
                b[0] = minOf(b[0], highlights[i])
                b[1] = minOf(b[1], highlights[i + 1])
                b[2] = maxOf(b[2], highlights[i])
                b[3] = maxOf(b[3], highlights[i + 1])
            }
        }
        return bounds
    }

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

    /**
     * Whether the [i]th visible page takes long to render (huge images,
     * mostly) and isn't sharp yet: it gets a spinner. Other pages are sharp
     * within milliseconds and never say so.
     */
    fun pageIsBusy(i: Int): Boolean = pageBusy[i]

    /** Whether [i] is one of the pages being looked at ([currentPage] to [lastPage]). */
    private fun isCurrent(i: Int): Boolean = pageIndex[i] in currentPage..lastPage

    /** The top edge of the pages being looked at; 0 if none of them is visible. */
    fun currentTop(): Float =
        (0 until pageCount).filter(::isCurrent).minOfOrNull { pageTop(it) } ?: 0f

    /** Their bottom edge. */
    fun currentBottom(): Float =
        (0 until pageCount).filter(::isCurrent).maxOfOrNull { pageTop(it) + pageHeight(it) } ?: 0f

    /** Whether they are no larger than [width] × [height] pixels, so that there is nowhere to pan to. */
    fun currentFitsIn(width: Float, height: Float): Boolean {
        val pages = (0 until pageCount).filter(::isCurrent)
        if (pages.isEmpty()) return true
        val across = pages.maxOf { pageLeft(it) + pageWidth(it) } - pages.minOf { pageLeft(it) }
        return across <= width && currentBottom() - currentTop() <= height
    }

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
        /** Kinds of highlight, as [highlightKind] reports them. */
        const val HIGHLIGHT_SELECTION = 0
        const val HIGHLIGHT_MATCH = 1
        const val HIGHLIGHT_CURRENT_MATCH = 2

        private const val PAGE_BYTES = 32
        private const val TILE_BYTES = 32
        private const val SEGMENT_BYTES = 32

        /** Reads the layout documented in crates/specto-ffi/src/hot.rs ([b] in native byte order). */
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
            val pageBusy = BooleanArray(pages)
            val tileStart = IntArray(pages + 1)
            val tileIds = LongArray(maxTiles)
            val tileRects = FloatArray(4 * maxTiles)
            val tileExact = BooleanArray(maxTiles)
            var t = 0
            for (p in 0 until pages) {
                pageIndex[p] = b.getInt()
                val tiles = b.getInt()
                for (k in 0 until 4) pageRects[4 * p + k] = b.getFloat()
                pageBusy[p] = b.getInt() and 1 != 0
                b.getInt()
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
            val highlightKinds = IntArray(b.getInt())
            val highlights = FloatArray(8 * highlightKinds.size)
            val edges = b.getInt()
            for (i in highlightKinds.indices) {
                highlightKinds[i] = b.getInt()
                b.getInt()
                for (k in 0 until 8) highlights[8 * i + k] = b.getFloat()
            }
            // Always there, but only meaningful if its bit is set.
            fun edge(present: Boolean): FloatArray? {
                val edge = FloatArray(4) { b.getFloat() }
                return edge.takeIf { present }
            }
            val selectionStart = edge(edges and 1 != 0)
            val selectionEnd = edge(edges and 2 != 0)

            val shapes = b.getInt()
            b.getInt()
            // Segments are at most what is left.
            val maxSegments = (length - b.position()) / SEGMENT_BYTES
            val shapeHeads = IntArray(5 * shapes)
            val shapeWidths = FloatArray(shapes)
            val shapeStarts = FloatArray(2 * shapes)
            val curved = BooleanArray(maxSegments)
            val points = FloatArray(6 * maxSegments)
            var s = 0
            for (i in 0 until shapes) {
                for (k in 0 until 4) shapeHeads[5 * i + k] = b.getInt()
                shapeHeads[5 * i + 4] = s
                shapeWidths[i] = b.getFloat()
                val segments = b.getInt()
                shapeStarts[2 * i] = b.getFloat()
                shapeStarts[2 * i + 1] = b.getFloat()
                repeat(segments) {
                    curved[s] = b.getInt() == 1
                    b.getInt()
                    for (k in 0 until 6) points[6 * s + k] = b.getFloat()
                    s++
                }
            }
            // Always there, but only meaningful if its bit is set.
            val outline = FloatArray(8) { b.getFloat() }
            return ViewFrame(
                complete = flags and 1 != 0,
                settled = flags and 2 != 0,
                paged = flags and 4 != 0,
                twoPages = flags and 8 != 0,
                coverAlone = flags and 16 != 0,
                selection = flags and 32 != 0,
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
                pageBusy = pageBusy,
                tileStart = tileStart,
                tileIds = tileIds,
                tileRects = tileRects,
                tileExact = tileExact,
                thumbnailFirst = thumbnailFirst,
                thumbnails = thumbnails,
                highlights = highlights,
                highlightKinds = highlightKinds,
                selectionStart = selectionStart,
                selectionEnd = selectionEnd,
                shapeHeads = shapeHeads,
                shapeWidths = shapeWidths,
                shapeStarts = shapeStarts,
                segmentCurved = curved.copyOf(s),
                segmentPoints = points,
                picked = outline.takeIf { flags and 64 != 0 },
            )
        }
    }
}
