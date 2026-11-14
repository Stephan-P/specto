package net.xfbs.pdfrs

import android.graphics.Bitmap
import android.os.Handler
import android.os.Looper
import android.util.Log
import androidx.compose.animation.core.AnimationState
import androidx.compose.animation.core.DecayAnimationSpec
import androidx.compose.animation.core.VectorConverter
import androidx.compose.animation.core.animate
import androidx.compose.animation.core.animateDecay
import androidx.compose.animation.core.tween
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.geometry.Offset
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.math.pow
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch
import uniffi.pdfrs_ffi.Frame
import uniffi.pdfrs_ffi.ViewListener
import uniffi.pdfrs_ffi.ViewSession
import uniffi.pdfrs_ffi.ZoomMode

private const val TAG = "pdfrs"

/** How far a double tap zooms in, relative to fit width. */
private const val DOUBLE_TAP_ZOOM = 2.5

/**
 * One open document: the Rust [ViewSession] (layout, zoom, scrolling, tile
 * planning and rendering) plus the tile bitmaps it tells us to keep.
 *
 * Main thread only, except [tilesReady]. Positions and distances are in dp
 * unless noted. Call [close] when done.
 */
class PdfViewState(pdf: ByteArray, cacheBytes: Long) : AutoCloseable {
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
    private val session = ViewSession.open(pdf, null, cacheBytes.toULong(), listener)
    private val sessionId = session.id().toLong()
    private val bitmaps = HashMap<ULong, Bitmap>()
    private var motion: Job? = null
    private var fitWidthZoom = 1.0

    val pageCount: Int = session.pageCount().toInt()

    /** What to draw. Changes whenever the view or the available tiles change. */
    var frame: Frame? by mutableStateOf(null)
        private set

    fun bitmap(id: ULong): Bitmap? = bitmaps[id]

    fun setViewport(widthDp: Float, heightDp: Float, density: Float) {
        session.setViewport(widthDp.toDouble(), heightDp.toDouble(), density.toDouble())
        refresh()
    }

    fun scrollBy(delta: Offset) {
        session.scrollBy(delta.x.toDouble(), delta.y.toDouble())
        refresh()
    }

    fun zoomBy(factor: Float, anchor: Offset) {
        session.zoomBy(factor.toDouble(), anchor.x.toDouble(), anchor.y.toDouble())
        refresh()
    }

    /** A finger went down: whatever was animating stops where it is. */
    fun stopMotion() {
        motion?.cancel()
        motion = null
    }

    /** Keeps scrolling after a drag. [velocityPx] is the content's velocity in px/s. */
    fun fling(scope: CoroutineScope, velocityPx: Offset, density: Float, decay: DecayAnimationSpec<Offset>) {
        stopMotion()
        motion = scope.launch {
            var last = Offset.Zero
            AnimationState(Offset.VectorConverter, Offset.Zero, velocityPx).animateDecay(decay) {
                scrollBy((value - last) / density)
                last = value
            }
        }
    }

    /** Zooms in around [anchor], or back out to fit width if already zoomed in. */
    fun doubleTap(scope: CoroutineScope, anchor: Offset) {
        val current = frame ?: return
        val zoomIn = current.zoom < fitWidthZoom * 1.05
        val ratio = (if (zoomIn) fitWidthZoom * DOUBLE_TAP_ZOOM else fitWidthZoom) / current.zoom
        stopMotion()
        motion = scope.launch {
            var applied = 1.0
            animate(0f, 1f, animationSpec = tween(220)) { t, _ ->
                val target = ratio.pow(t.toDouble())
                zoomBy((target / applied).toFloat(), anchor)
                applied = target
            }
            if (!zoomIn) {
                // Back to following the window width (e.g. on rotation).
                session.setZoomMode(ZoomMode.FIT_WIDTH)
                refresh()
            }
        }
    }

    /** Drops every tile that isn't on screen (`onTrimMemory`). */
    fun trimMemory() {
        if (closed) return
        session.trimMemory()
        refresh()
        Log.i(TAG, "trimmed to ${bitmaps.size} tiles, ${bitmaps.values.sumOf { it.allocationByteCount } shr 20} MB")
    }

    private fun collectTiles() {
        if (closed) return
        for (tile in session.takeFinished()) {
            val bitmap = Bitmap.createBitmap(tile.width.toInt(), tile.height.toInt(), Bitmap.Config.ARGB_8888)
            if (NativeBitmap.copyTile(sessionId, tile.id.toLong(), bitmap) == 0) {
                bitmap.prepareToDraw()
                bitmaps[tile.id] = bitmap
            } else {
                bitmap.recycle()
            }
        }
        refresh()
    }

    private fun refresh() {
        if (closed) return
        val next = session.frame()
        // Free the pixels now rather than whenever the GC gets to them. The
        // new frame never draws an evicted tile, and a frame already handed to
        // the render thread keeps its own reference to the pixels.
        for (id in next.evicted) bitmaps.remove(id)?.recycle()
        if (next.zoomMode == ZoomMode.FIT_WIDTH) fitWidthZoom = next.zoom
        frame = next
    }

    override fun close() {
        closed = true
        stopMotion()
        frame = null
        bitmaps.values.forEach(Bitmap::recycle)
        bitmaps.clear()
        // Also releases the listener, which the session holds on to.
        session.close()
    }
}
