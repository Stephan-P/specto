package net.xfbs.pdfrs

import android.graphics.Paint
import android.graphics.RectF
import android.util.Log
import androidx.compose.animation.splineBasedDecay
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.calculateCentroid
import androidx.compose.foundation.gestures.calculatePan
import androidx.compose.foundation.gestures.calculateZoom
import androidx.compose.foundation.focusable
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.clipRect
import androidx.compose.ui.graphics.drawscope.drawIntoCanvas
import androidx.compose.ui.graphics.nativeCanvas
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEvent
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.isAltPressed
import androidx.compose.ui.input.key.isCtrlPressed
import androidx.compose.ui.input.key.isMetaPressed
import androidx.compose.ui.input.key.isShiftPressed
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.positionChanged
import androidx.compose.ui.input.pointer.util.VelocityTracker
import androidx.compose.ui.input.pointer.util.addPointerInputChange
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.layout.positionInWindow
import androidx.compose.ui.platform.LocalDensity
import kotlin.math.abs
import kotlinx.coroutines.CoroutineScope
import uniffi.pdfrs_ffi.Activated

/**
 * What keys do while the page has the focus, as on the desktop: arrows left
 * and right, Page Up and Down, Space and Backspace turn pages; arrows up and
 * down scroll; Home and End go to the first and last page; Ctrl with plus,
 * minus and zero zooms. Returns whether the key was used.
 */
private fun PdfViewState.handleKey(scope: CoroutineScope, event: KeyEvent): Boolean {
    if (event.isCtrlPressed) {
        when (event.key) {
            Key.Equals, Key.Plus, Key.NumPadAdd -> zoomStep(zoomIn = true)
            Key.Minus, Key.NumPadSubtract -> zoomStep(zoomIn = false)
            Key.Zero, Key.NumPad0 -> fit()
            else -> return false
        }
        return true
    }
    if (event.isAltPressed || event.isMetaPressed) return false
    when (event.key) {
        Key.DirectionRight -> turnPage(forward = true)
        Key.DirectionLeft, Key.Backspace -> turnPage(forward = false)
        Key.PageDown -> pageBy(scope, forward = true)
        Key.PageUp -> pageBy(scope, forward = false)
        Key.Spacebar -> pageBy(scope, forward = !event.isShiftPressed)
        Key.DirectionDown -> lineBy(forward = true)
        Key.DirectionUp -> lineBy(forward = false)
        Key.MoveHome -> goToPage(0)
        Key.MoveEnd -> goToPage(pageCount - 1)
        else -> return false
    }
    return true
}

/**
 * Draws the frames planned by [state] and turns touches into scrolling or
 * swiping between pages, pinch zoom and double-tap zoom. A tap follows a
 * link; links leaving the document are passed to [onExternalLink], and taps
 * that hit no link to [onTap]. With a keyboard attached, keys turn pages,
 * scroll and zoom. No layout maths happens here.
 */
@Composable
fun PdfView(
    state: PdfViewState,
    modifier: Modifier = Modifier,
    onExternalLink: (String) -> Unit = {},
    onTap: () -> Unit = {},
) {
    val density = LocalDensity.current.density
    val scope = rememberCoroutineScope()
    val exactPaint = remember { Paint().apply { isFilterBitmap = false } }
    val scaledPaint = remember { Paint().apply { isFilterBitmap = true } }
    val dest = remember { RectF() }
    var origin by remember { mutableStateOf(Offset.Zero) }

    // Sharpness probe, off unless `adb shell setprop log.tag.pdfrs DEBUG`:
    // a screenshot's page area should then match
    // `pdfrs render file.pdf -p <page> -s <device scale>` pixel for pixel.
    val frame = state.frame
    if (frame != null && frame.settled && frame.complete && Log.isLoggable("pdfrs", Log.DEBUG)) {
        LaunchedEffect(frame) {
            if (frame.pageCount == 0) return@LaunchedEffect
            Log.d(
                "pdfrs",
                "settled: device scale ${frame.deviceScale}, canvas origin (${origin.x}, ${origin.y}) px, " +
                    "page ${frame.page(0) + 1} at (${frame.pageLeft(0)}, ${frame.pageTop(0)}) " +
                    "${frame.pageWidth(0)}x${frame.pageHeight(0)}",
            )
        }
    }

    // Keys go to whatever has the focus, so the page takes it when it appears.
    val focus = remember { FocusRequester() }
    LaunchedEffect(state) { focus.requestFocus() }

    Canvas(
        modifier
            .clipToBounds()
            .focusRequester(focus)
            .focusable()
            .onKeyEvent { it.type == KeyEventType.KeyDown && state.handleKey(scope, it) }
            .onGloballyPositioned { origin = it.positionInWindow() }
            .onSizeChanged { state.setViewport(it.width / density, it.height / density, density) }
            .pointerInput(state) {
                detectTapGestures(
                    onTap = {
                        when (val hit = state.tap(it / density)) {
                            Activated.Nothing -> onTap()
                            Activated.Moved -> {}
                            is Activated.Uri -> onExternalLink(hit.uri)
                        }
                    },
                    onDoubleTap = { state.doubleTap(scope, it / density) },
                )
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
                    var zooming = false
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
                            if (zoom != 1f && !zooming) {
                                zooming = true
                                state.beginZoomGesture()
                            }
                            state.transformBy(
                                zoom,
                                event.calculateCentroid(useCurrent = false) / density,
                                -pan / density,
                            )
                            event.changes.forEach { if (it.positionChanged()) it.consume() }
                        }
                        // Only a single finger lifting off leads to a fling.
                        val pressed = event.changes.filter { it.pressed }
                        if (event.changes.size == 1) {
                            // Includes the samples batched between frames.
                            velocity.addPointerInputChange(event.changes[0])
                        } else {
                            velocity.resetTracking()
                        }
                    } while (pressed.isNotEmpty())
                    if (zooming) state.endZoomGesture()
                    if (dragging) {
                        val v = velocity.calculateVelocity()
                        state.release(scope, Offset(-v.x, -v.y), density, decay)
                    }
                }
            },
    ) {
        val f = state.frame ?: return@Canvas
        for (p in 0 until f.pageCount) {
            val left = f.pageLeft(p)
            val top = f.pageTop(p)
            val size = Size(f.pageWidth(p), f.pageHeight(p))
            drawRect(Color.White, Offset(left, top), size)
            clipRect(left, top, left + size.width, top + size.height) {
                drawIntoCanvas { canvas ->
                    for (t in f.tiles(p)) {
                        val bitmap = state.bitmap(f.tileId(t)) ?: continue
                        // Destinations can be fractional, so no IntOffset-based drawImage.
                        dest.set(f.tileLeft(t), f.tileTop(t), f.tileRight(t), f.tileBottom(t))
                        val paint = if (f.tileIsExact(t)) exactPaint else scaledPaint
                        canvas.nativeCanvas.drawBitmap(bitmap, null, dest, paint)
                    }
                }
            }
        }
    }
}
