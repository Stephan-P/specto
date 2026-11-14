package net.xfbs.pdfrs

import android.graphics.Paint
import android.graphics.RectF
import androidx.compose.animation.splineBasedDecay
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.calculateCentroid
import androidx.compose.foundation.gestures.calculatePan
import androidx.compose.foundation.gestures.calculateZoom
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.clipRect
import androidx.compose.ui.graphics.drawscope.drawIntoCanvas
import androidx.compose.ui.graphics.nativeCanvas
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.positionChanged
import androidx.compose.ui.input.pointer.util.VelocityTracker
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalDensity
import kotlin.math.abs

/**
 * Draws the frames planned by [state] and turns touches into scrolling
 * (with fling), pinch zoom and double-tap zoom. No layout maths happens here.
 */
@Composable
fun PdfView(state: PdfViewState, modifier: Modifier = Modifier) {
    val density = LocalDensity.current.density
    val scope = rememberCoroutineScope()
    val exactPaint = remember { Paint().apply { isFilterBitmap = false } }
    val scaledPaint = remember { Paint().apply { isFilterBitmap = true } }
    val dest = remember { RectF() }

    Canvas(
        modifier
            .clipToBounds()
            .onSizeChanged { state.setViewport(it.width / density, it.height / density, density) }
            .pointerInput(state) {
                detectTapGestures(onDoubleTap = { state.doubleTap(scope, it / density) })
            }
            .pointerInput(state) {
                val decay = splineBasedDecay<Offset>(this)
                awaitEachGesture {
                    awaitFirstDown(requireUnconsumed = false)
                    state.stopMotion()
                    val velocity = VelocityTracker()
                    var moved = Offset.Zero
                    var zoomed = 1f
                    var dragging = false
                    do {
                        val event = awaitPointerEvent()
                        val pan = event.calculatePan()
                        val zoom = event.calculateZoom()
                        if (!dragging) {
                            // Ignore jitter until the fingers clearly move, so taps stay taps.
                            moved += pan
                            zoomed *= zoom
                            dragging = moved.getDistance() > viewConfiguration.touchSlop ||
                                (event.changes.size > 1 && abs(1 - zoomed) > 0.03f)
                        }
                        if (dragging) {
                            if (zoom != 1f) {
                                state.zoomBy(zoom, event.calculateCentroid(useCurrent = false) / density)
                            }
                            if (pan != Offset.Zero) state.scrollBy(-pan / density)
                            event.changes.forEach { if (it.positionChanged()) it.consume() }
                        }
                        // Only a single finger lifting off leads to a fling.
                        val pressed = event.changes.filter { it.pressed }
                        if (event.changes.size == 1) {
                            val change = event.changes[0]
                            velocity.addPosition(change.uptimeMillis, change.position)
                        } else {
                            velocity.resetTracking()
                        }
                    } while (pressed.isNotEmpty())
                    if (dragging) {
                        val v = velocity.calculateVelocity()
                        state.fling(scope, Offset(-v.x, -v.y), density, decay)
                    }
                }
            },
    ) {
        val frame = state.frame ?: return@Canvas
        for (page in frame.pages) {
            val r = page.rect
            drawRect(Color.White, Offset(r.x, r.y), Size(r.width, r.height))
            clipRect(r.x, r.y, r.x + r.width, r.y + r.height) {
                drawIntoCanvas { canvas ->
                    for (tile in page.tiles) {
                        val bitmap = state.bitmap(tile.id) ?: continue
                        val d = tile.dest
                        dest.set(d.x, d.y, d.x + d.width, d.y + d.height)
                        // Destinations can be fractional, so no IntOffset-based drawImage.
                        val paint = if (tile.exact) exactPaint else scaledPaint
                        canvas.nativeCanvas.drawBitmap(bitmap, null, dest, paint)
                    }
                }
            }
        }
    }
}
