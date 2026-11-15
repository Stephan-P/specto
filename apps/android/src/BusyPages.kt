package net.xfbs.specto

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import kotlin.math.roundToInt

/** Identifies the spinner over a page that takes long to draw to UI tests. */
const val BUSY_PAGE_TAG = "busy-page"

/** The plate under a spinner, which keeps it visible on any page. */
private val PLATE = 56.dp
private val SPINNER = 32.dp
private val PLATE_SHAPE = RoundedCornerShape(12.dp)

/**
 * A spinner over each page that takes long to draw and isn't sharp yet
 * ([ViewFrame.pageIsBusy]), in the middle of the part of it that is on
 * screen, as on the desktop. Such pages are rare (huge scans, mostly): any
 * other page is sharp within milliseconds and never shows one.
 *
 * Fills the page view, on top of it, and takes no touches. While no page is
 * busy there is nothing here, and so nothing animating.
 */
@Composable
fun BusyPages(state: PdfViewState, modifier: Modifier = Modifier) {
    val frame = state.frame ?: return
    if ((0 until frame.pageCount).none(frame::pageIsBusy)) return
    var view by remember { mutableStateOf(IntSize.Zero) }
    val plate = with(LocalDensity.current) { PLATE.toPx() }
    val stillDrawing = stringResource(R.string.still_drawing)
    Box(modifier.fillMaxSize().clipToBounds().onSizeChanged { view = it }) {
        for (i in 0 until frame.pageCount) {
            if (!frame.pageIsBusy(i)) continue
            // What is on screen of the page; a sliver is left alone.
            val left = maxOf(frame.pageLeft(i), 0f)
            val top = maxOf(frame.pageTop(i), 0f)
            val right = minOf(frame.pageLeft(i) + frame.pageWidth(i), view.width.toFloat())
            val bottom = minOf(frame.pageTop(i) + frame.pageHeight(i), view.height.toFloat())
            if (right - left < plate || bottom - top < plate) continue
            // Keyed, so that a spinner keeps turning as pages come and go.
            key(frame.page(i)) {
                Box(
                    Modifier
                        .offset {
                            IntOffset(
                                ((left + right - plate) / 2).roundToInt(),
                                ((top + bottom - plate) / 2).roundToInt(),
                            )
                        }
                        .size(PLATE)
                        .shadow(3.dp, PLATE_SHAPE)
                        .background(MaterialTheme.colorScheme.surfaceContainerHigh, PLATE_SHAPE)
                        .testTag(BUSY_PAGE_TAG)
                        .semantics { contentDescription = stillDrawing },
                    contentAlignment = Alignment.Center,
                ) {
                    CircularProgressIndicator(Modifier.size(SPINNER), strokeWidth = 3.dp)
                }
            }
        }
    }
}
