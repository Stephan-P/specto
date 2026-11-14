package net.xfbs.pdfrs

import android.graphics.Paint
import android.graphics.RectF
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.rememberLazyGridState
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.drawIntoCanvas
import androidx.compose.ui.graphics.nativeCanvas
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.distinctUntilChanged

/** Shape of a grid cell; pages are fitted into it whatever their own shape. */
private const val CELL_ASPECT = 0.72f

/** How long the page must stay the same before a following grid scrolls to it. */
private const val FOLLOW_DELAY_MS = 300L

/** Thumbnails to prepare beyond the visible ones, for scrolling on. */
const val THUMBNAIL_PREFETCH = 6

private val thumbnailPaint = Paint().apply { isFilterBitmap = true }

/**
 * The thumbnail of [page] on a white sheet, filling the space it is given
 * (the caller sets the aspect ratio), outlined, more strongly if [selected].
 * Blank until the thumbnail is rendered. The thumbnails are the page view's
 * own low-resolution placeholder tiles, so nothing is rendered twice; the
 * caller must announce the pages it shows with [PdfViewState.showThumbnails].
 */
@Composable
fun PageThumbnail(
    state: PdfViewState,
    page: Int,
    selected: Boolean,
    modifier: Modifier = Modifier,
    selectedBorder: Dp = 3.dp,
) {
    val tile = state.thumbnailId(page)
    val bitmap = if (tile != 0L) state.bitmap(tile) else null
    val outline =
        if (selected) {
            Modifier.border(selectedBorder, MaterialTheme.colorScheme.primary)
        } else {
            Modifier.border(1.dp, MaterialTheme.colorScheme.outlineVariant)
        }
    Box(
        modifier
            .background(Color.White)
            .drawBehind {
                // Not `Image`: the tile cache may recycle a bitmap, and
                // drawing a recycled one would throw.
                if (bitmap != null && !bitmap.isRecycled) {
                    drawIntoCanvas {
                        it.nativeCanvas.drawBitmap(
                            bitmap, null, RectF(0f, 0f, size.width, size.height), thumbnailPaint,
                        )
                    }
                }
            }
            .then(outline),
    )
}

/**
 * All pages as a grid of thumbnails; tapping one calls [onPick] with its
 * zero-based index.
 */
@Composable
fun PagesGrid(
    state: PdfViewState,
    onPick: (Int) -> Unit,
    modifier: Modifier = Modifier,
    /** Scroll to keep the current page in view as the reader moves on. */
    follow: Boolean = false,
) {
    val current = state.currentPage
    val grid = rememberLazyGridState(initialFirstVisibleItemIndex = current)
    val owner = remember { Any() }

    LaunchedEffect(state, grid) {
        snapshotFlow {
            val visible = grid.layoutInfo.visibleItemsInfo
            if (visible.isEmpty()) 0 to 0 else visible.first().index to visible.size + THUMBNAIL_PREFETCH
        }.distinctUntilChanged().collect { (first, count) -> state.showThumbnails(owner, first, count) }
    }
    DisposableEffect(state) {
        onDispose { state.hideThumbnails(owner) }
    }
    if (follow) {
        LaunchedEffect(state, grid) {
            snapshotFlow { state.currentPage }.collectLatest { page ->
                // Once the page view rests, and only if the page isn't fully in view.
                delay(FOLLOW_DELAY_MS)
                val info = grid.layoutInfo
                val item = info.visibleItemsInfo.firstOrNull { it.index == page }
                val inView = item != null &&
                    item.offset.y >= info.viewportStartOffset &&
                    item.offset.y + item.size.height <= info.viewportEndOffset
                if (!inView && !grid.isScrollInProgress) grid.animateScrollToItem(page)
            }
        }
    }

    LazyVerticalGrid(
        columns = GridCells.Adaptive(minSize = 104.dp),
        state = grid,
        modifier = modifier,
        contentPadding = PaddingValues(16.dp),
        horizontalArrangement = Arrangement.spacedBy(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        items(state.pageCount, key = { it }) { page ->
            Column(
                Modifier.clickable { onPick(page) },
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                Box(Modifier.fillMaxWidth().aspectRatio(CELL_ASPECT), contentAlignment = Alignment.Center) {
                    val aspect = state.pageAspect(page)
                    PageThumbnail(
                        state,
                        page,
                        selected = page == current,
                        Modifier
                            .aspectRatio(aspect, matchHeightConstraintsFirst = aspect < CELL_ASPECT)
                            .fillMaxSize(),
                    )
                }
                Text(
                    state.pageLabel(page),
                    Modifier.padding(top = 4.dp),
                    style = MaterialTheme.typography.labelMedium,
                    color = if (page == current) MaterialTheme.colorScheme.primary else Color.Unspecified,
                    maxLines = 1,
                )
            }
        }
    }
}
