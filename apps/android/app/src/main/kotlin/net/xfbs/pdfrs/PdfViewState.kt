package net.xfbs.pdfrs

import android.graphics.Bitmap
import android.os.Handler
import android.os.Looper
import android.util.Log
import android.util.LongSparseArray
import androidx.compose.animation.core.AnimationState
import androidx.compose.animation.core.DecayAnimationSpec
import androidx.compose.animation.core.LinearOutSlowInEasing
import androidx.compose.animation.core.VectorConverter
import androidx.compose.animation.core.animate
import androidx.compose.animation.core.animateDecay
import androidx.compose.animation.core.tween
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.geometry.Offset
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.math.pow
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch
import uniffi.pdfrs_ffi.Activated
import uniffi.pdfrs_ffi.OutlineEntry
import uniffi.pdfrs_ffi.ViewListener
import uniffi.pdfrs_ffi.ViewSession
import uniffi.pdfrs_ffi.ViewMode

private const val TAG = "pdfrs"

private fun newBuffer(bytes: Int): ByteBuffer =
    ByteBuffer.allocateDirect(bytes).order(ByteOrder.nativeOrder())

/** How far a double tap zooms in, relative to the fitted view. */
private const val DOUBLE_TAP_ZOOM = 2.5

/** Duration of the slide that brings a swiped page to rest. */
private const val SLIDE_MS = 260

/**
 * One open document: the Rust [ViewSession] (layout, zoom, scrolling, tile
 * planning and rendering) plus the tile bitmaps it tells us to keep.
 *
 * Whatever happens every frame goes through [Native] (plain JNI); the
 * generated UniFFI bindings are only used for occasional calls.
 *
 * Main thread only, except [tilesReady]. Positions and distances are in dp
 * unless noted. Call [close] when done.
 *
 * Throws `PdfException` if the file can't be opened, e.g.
 * `PdfException.PasswordRequired` for a missing or wrong [password].
 */
class PdfViewState(pdf: ByteArray, password: String?, cacheBytes: Long) : AutoCloseable {
    private val main = Handler(Looper.getMainLooper())
    private val tilesPosted = AtomicBoolean(false)
    private var closed = false

    private val listener = object : ViewListener {
        // Called on a Rust render thread.
        override fun tilesReady() {
            if (tilesPosted.compareAndSet(false, true)) {
                main.post {
                    tilesPosted.set(false)
                    collectTiles()
                }
            }
        }
    }
    private val session = ViewSession.open(pdf, password, cacheBytes.toULong(), listener)
    private val sessionId = session.id().toLong()
    private val bitmaps = LongSparseArray<Bitmap>()
    private var buffer = newBuffer(16 * 1024)
    private var motion: Job? = null
    private val settle = Runnable { refresh() }
    private var thumbnailOwner: Any? = null

    val pageCount: Int = session.pageCount().toInt()

    /** Width / height of each page as displayed. */
    private val aspects = session.pageSizes().let { sizes ->
        FloatArray(sizes.size) { sizes[it].width / sizes[it].height }
    }

    fun pageAspect(page: Int): Float = aspects[page]

    /** The outline (table of contents) in reading order; empty if there is none. */
    val outline: List<OutlineEntry> = session.outline()

    init {
        // On a phone, swiping through fitted pages is the default.
        session.setViewMode(ViewMode.PAGED)
    }

    /**
     * One page at a time, swiped left and right, or else one continuously
     * scrolling column. Keeps the current page.
     */
    var paged: Boolean
        get() = frame?.paged ?: true
        set(value) {
            stopMotion()
            session.setViewMode(if (value) ViewMode.PAGED else ViewMode.CONTINUOUS)
            refresh()
        }

    /**
     * What to draw. Changes whenever the view or the available tiles change,
     * i.e. on every step of a scroll: only the page canvas should read it.
     * The bars around it read [currentPage] and [thumbnailId], which change
     * far less often.
     */
    var frame: ViewFrame? by mutableStateOf(null)
        private set

    /** Zero-based index of the page being looked at. */
    var currentPage: Int by mutableIntStateOf(0)
        private set

    /** The frame that last changed which thumbnails are available. */
    private var thumbnails: ViewFrame? by mutableStateOf(null)

    /**
     * The tile to show as thumbnail of [page] (zero-based), or 0 if it isn't
     * rendered yet or wasn't announced with [showThumbnails].
     */
    fun thumbnailId(page: Int): Long = thumbnails?.thumbnailId(page) ?: 0

    fun bitmap(id: Long): Bitmap? = bitmaps[id]

    fun setViewport(widthDp: Float, heightDp: Float, density: Float) {
        session.setViewport(widthDp.toDouble(), heightDp.toDouble(), density.toDouble())
        refresh()
    }

    fun scrollBy(delta: Offset) {
        Native.scrollBy(sessionId, delta.x.toDouble(), delta.y.toDouble())
        refresh()
    }

    fun zoomBy(factor: Float, anchor: Offset) {
        Native.zoomBy(sessionId, factor.toDouble(), anchor.x.toDouble(), anchor.y.toDouble())
        refresh()
    }

    /**
     * A tap at [at]: follows a link there, if any. Links within the document
     * are followed right away ([Activated.Moved]); for an external one
     * ([Activated.Uri]), opening it after asking is up to the caller.
     * [Activated.Nothing] means the tap hit no link.
     */
    fun tap(at: Offset): Activated = finish(session.tap(at.x.toDouble(), at.y.toDouble()))

    /** Goes where entry [index] of [outline] leads; see [tap] for the result. */
    fun openOutlineEntry(index: Int): Activated = finish(session.activateOutline(index.toUInt()))

    private fun finish(activated: Activated): Activated {
        if (activated == Activated.Moved) {
            stopMotion()
            refresh()
        }
        return activated
    }

    /** Shows [page] (zero-based) at the top of the view. */
    fun goToPage(page: Int) {
        stopMotion()
        session.goToPage(page.coerceIn(0, pageCount - 1).toUInt())
        refresh()
    }

    /**
     * Tells which pages' thumbnails are on screen, so that they get rendered
     * and kept: see [ViewFrame.thumbnailId]. One range at a time: [owner]
     * identifies the caller (the page grid, the thumbnail strip), and the
     * last one to call wins.
     */
    fun showThumbnails(owner: Any, first: Int, count: Int) {
        if (closed) return
        thumbnailOwner = owner
        Native.setThumbnailRange(sessionId, first, count)
        refresh()
    }

    /** [owner] no longer shows thumbnails. Does nothing if another took over since. */
    fun hideThumbnails(owner: Any) {
        if (closed || thumbnailOwner !== owner) return
        thumbnailOwner = null
        Native.setThumbnailRange(sessionId, 0, 0)
        refresh()
    }

    /** A pinch step: zoom and pan together, planning only one frame. */
    fun transformBy(zoom: Float, anchor: Offset, pan: Offset) {
        if (zoom != 1f) Native.zoomBy(sessionId, zoom.toDouble(), anchor.x.toDouble(), anchor.y.toDouble())
        if (pan != Offset.Zero) Native.scrollBy(sessionId, pan.x.toDouble(), pan.y.toDouble())
        refresh()
    }

    /**
     * Brackets a pinch. In between, pages are drawn from a few fixed zoom
     * levels; exact-scale tiles are only rendered once the zoom has come to rest.
     */
    fun beginZoomGesture() {
        session.beginZoomGesture()
    }

    fun endZoomGesture() {
        session.endZoomGesture()
        refresh()
    }

    /** A finger went down: whatever was animating stops where it is. */
    fun stopMotion() {
        motion?.cancel()
        motion = null
    }

    /**
     * The fingers lifted after a drag or pinch. [velocityPx] is the scrolling
     * velocity at that moment in px/s. In paged mode a view left between two
     * pages slides on to one of them (a flick turns the page); otherwise the
     * scrolling carries on and slows down, within the page when paged.
     */
    fun release(scope: CoroutineScope, velocityPx: Offset, density: Float, decay: DecayAnimationSpec<Offset>) {
        stopMotion()
        val current = frame ?: return
        if (current.paged) {
            val snap = session.snapDelta((velocityPx.x / density).toDouble(), (velocityPx.y / density).toDouble())
            if (snap != null) {
                motion = scope.launch { slideBy(Offset(snap.x.toFloat(), snap.y.toFloat())) }
                return
            }
            // A fitted page has nowhere to go.
            if (current.zoomFactor < 1.01) return
        }
        motion = scope.launch {
            var last = Offset.Zero
            AnimationState(Offset.VectorConverter, Offset.Zero, velocityPx).animateDecay(decay) {
                val step = (value - last) / density
                last = value
                Native.scrollInPageBy(sessionId, step.x.toDouble(), step.y.toDouble())
                refresh()
            }
        }
    }

    /** Slides the view by [delta] dp and makes sure it ends at rest. */
    private suspend fun slideBy(delta: Offset) {
        var done = 0f
        animate(0f, 1f, animationSpec = tween(SLIDE_MS, easing = LinearOutSlowInEasing)) { t, _ ->
            scrollBy(delta * (t - done))
            done = t
        }
        // Rounding along the way may leave a fraction of a pixel.
        session.snapDelta(0.0, 0.0)?.let { scrollBy(Offset(it.x.toFloat(), it.y.toFloat())) }
    }

    /** Zooms in around [anchor], or back out to the fitted view if already zoomed in. */
    fun doubleTap(scope: CoroutineScope, anchor: Offset) {
        val current = frame ?: return
        val zoomIn = current.zoomFactor < 1.05
        val ratio = (if (zoomIn) DOUBLE_TAP_ZOOM else 1.0) / current.zoomFactor
        stopMotion()
        motion = scope.launch {
            beginZoomGesture()
            try {
                var applied = 1.0
                animate(0f, 1f, animationSpec = tween(220)) { t, _ ->
                    val target = ratio.pow(t.toDouble())
                    zoomBy((target / applied).toFloat(), anchor)
                    applied = target
                }
                // Back to following the window size (e.g. on rotation).
                if (!zoomIn) session.fit()
            } finally {
                // Also when a touch interrupts the animation.
                if (!closed) endZoomGesture()
            }
        }
    }

    /** Drops every tile that isn't on screen (`onTrimMemory`). */
    fun trimMemory() {
        if (closed) return
        session.trimMemory()
        refresh()
        Log.i(TAG, "trimmed to ${bitmaps.size()} tiles")
    }

    private fun collectTiles() {
        if (closed) return
        // Tiles that don't fit this time are rendered again; 16 bytes each,
        // so the buffer holds about a thousand.
        if (Native.takeFinished(sessionId, buffer) > 0) {
            val b = buffer
            b.position(0)
            val count = b.getInt()
            b.getInt()
            repeat(count) {
                val id = b.getLong()
                val bitmap = Bitmap.createBitmap(b.getInt(), b.getInt(), Bitmap.Config.ARGB_8888)
                if (Native.copyTile(sessionId, id, bitmap) == 0) {
                    bitmap.prepareToDraw()
                    bitmaps.put(id, bitmap)
                } else {
                    bitmap.recycle()
                }
            }
        }
        refresh()
    }

    private fun refresh() {
        if (closed) return
        var length = Native.frame(sessionId, buffer)
        if (length < 0) {
            buffer = newBuffer(-length * 2)
            length = Native.lastFrame(sessionId, buffer)
        }
        if (length <= 0) return
        val next = ViewFrame.decode(buffer, length)
        // Free the pixels now rather than whenever the GC gets to them. The
        // new frame never draws an evicted tile, and a frame already handed to
        // the render thread keeps its own reference to the pixels.
        for (id in next.evicted) {
            bitmaps[id]?.recycle()
            bitmaps.remove(id)
        }
        frame = next
        currentPage = next.currentPage
        if (!next.sameThumbnailsAs(thumbnails)) thumbnails = next
        // After zooming, the view sharpens (switches to exact-scale tiles)
        // only when a frame is planned once the zoom has been still for a while.
        main.removeCallbacks(settle)
        if (next.settlesInMs >= 0) main.postDelayed(settle, next.settlesInMs + 1L)
    }

    override fun close() {
        stopMotion()
        closed = true
        main.removeCallbacks(settle)
        thumbnails = null
        frame = null
        for (i in 0 until bitmaps.size()) bitmaps.valueAt(i).recycle()
        bitmaps.clear()
        // Also releases the listener, which the session holds on to.
        session.close()
    }
}
