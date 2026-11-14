package net.xfbs.pdfrs

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.distinctUntilChanged

/** Identifies the strip to UI tests. */
const val STRIP_TAG = "thumbnail-strip"

/** Height of a thumbnail in the strip. */
private val THUMBNAIL_HEIGHT = 56.dp

/** How long the page must stay the same before the strip scrolls to it. */
private const val FOLLOW_DELAY_MS = 300L

/** Very wide or very tall pages are shown in a cell no more extreme than this. */
private val ASPECT_RANGE = 0.5f..1.6f

/**
 * A row of page thumbnails along the bottom, with the current page in the
 * middle. Scrolling it flips through the document: whichever page passes the
 * middle is shown. Tapping a thumbnail goes there. When pages are turned in
 * the page view, the strip follows.
 */
@Composable
fun ThumbnailStrip(state: PdfViewState, modifier: Modifier = Modifier) {
    val current = state.currentPage
    // The row is padded by half its width at both ends, so that an item at
    // the start of the content sits in the middle.
    val list = rememberLazyListState(initialFirstVisibleItemIndex = current)
    val owner = remember { Any() }
    // Set while the strip scrolls by itself to follow the page view, as
    // opposed to being scrolled by hand.
    val following = remember { booleanArrayOf(false) }

    LaunchedEffect(state, list) {
        snapshotFlow {
            val visible = list.layoutInfo.visibleItemsInfo
            if (visible.isEmpty()) 0 to 0 else visible.first().index to visible.size + THUMBNAIL_PREFETCH
        }.distinctUntilChanged().collect { (first, count) -> state.showThumbnails(owner, first, count) }
    }
    DisposableEffect(state) {
        onDispose { state.hideThumbnails(owner) }
    }
    LaunchedEffect(state, list) {
        // Follow once the page view has come to rest: scrolling the strip
        // while a page is still sliding costs frames, and flipping through
        // several pages would restart it each time.
        snapshotFlow { state.currentPage }.collectLatest { page ->
            delay(FOLLOW_DELAY_MS)
            if (list.isScrollInProgress) return@collectLatest
            following[0] = true
            try {
                list.animateScrollToItem(page)
            } finally {
                following[0] = false
            }
        }
    }
    LaunchedEffect(state, list) {
        // Scrubbing: while the strip is scrolled by hand (or still gliding
        // from it), the page passing the middle is the one shown.
        snapshotFlow {
            if (!list.isScrollInProgress || following[0]) return@snapshotFlow -1
            val info = list.layoutInfo
            val middle = (info.viewportStartOffset + info.viewportEndOffset) / 2
            info.visibleItemsInfo.minByOrNull { kotlin.math.abs(it.offset + it.size / 2 - middle) }?.index ?: -1
        }.distinctUntilChanged().collect { page ->
            if (page >= 0 && page != state.currentPage) state.goToPage(page)
        }
    }

    Surface(modifier.testTag(STRIP_TAG), color = MaterialTheme.colorScheme.surfaceContainer) {
        BoxWithConstraints(Modifier.fillMaxWidth().navigationBarsPadding()) {
        LazyRow(
            Modifier.fillMaxWidth().height(THUMBNAIL_HEIGHT + 16.dp),
            state = list,
            contentPadding = PaddingValues(
                horizontal = (maxWidth - THUMBNAIL_HEIGHT * state.pageAspect(current).coerceIn(ASPECT_RANGE)) / 2,
                vertical = 8.dp,
            ),
            horizontalArrangement = Arrangement.spacedBy(6.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            items(state.pageCount, key = { it }) { page ->
                PageThumbnail(
                    state,
                    page,
                    selected = page == current,
                    Modifier
                        .fillMaxHeight()
                        .aspectRatio(state.pageAspect(page).coerceIn(ASPECT_RANGE))
                        .clickable { state.goToPage(page) },
                    selectedBorder = 2.dp,
                )
            }
        }
        }
    }
}
