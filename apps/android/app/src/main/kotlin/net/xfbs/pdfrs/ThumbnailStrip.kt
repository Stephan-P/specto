package net.xfbs.pdfrs

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
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
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.distinctUntilChanged

/** Height of a thumbnail in the strip. */
private val THUMBNAIL_HEIGHT = 56.dp

/** How long the page must stay the same before the strip scrolls to it. */
private const val FOLLOW_DELAY_MS = 300L

/** Very wide or very tall pages are shown in a cell no more extreme than this. */
private val ASPECT_RANGE = 0.5f..1.6f

/**
 * A row of page thumbnails along the bottom: scroll it to look around, tap a
 * page to go there. It follows the page view, keeping the current page in
 * the middle, unless it is being scrolled by hand.
 */
@Composable
fun ThumbnailStrip(state: PdfViewState, modifier: Modifier = Modifier) {
    val current = state.currentPage
    val list = rememberLazyListState(initialFirstVisibleItemIndex = (current - 3).coerceAtLeast(0))
    val owner = remember { Any() }
    val thumbnailPx = with(LocalDensity.current) { THUMBNAIL_HEIGHT.toPx() }

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
            val info = list.layoutInfo
            val width = thumbnailPx * state.pageAspect(page).coerceIn(ASPECT_RANGE)
            val middle = (info.viewportEndOffset + info.viewportStartOffset - width) / 2
            // Offsets are relative to the start of the row's content area.
            list.animateScrollToItem(page, -(middle - info.beforeContentPadding).toInt())
        }
    }

    Surface(modifier, color = MaterialTheme.colorScheme.surfaceContainer) {
        LazyRow(
            Modifier.fillMaxWidth().navigationBarsPadding().height(THUMBNAIL_HEIGHT + 16.dp),
            state = list,
            contentPadding = PaddingValues(horizontal = 12.dp, vertical = 8.dp),
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
