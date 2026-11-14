package net.xfbs.pdfrs

import android.graphics.Paint
import android.graphics.RectF
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.rememberLazyGridState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Check
import androidx.compose.material3.Icon
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
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
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
 * (the caller sets the aspect ratio), outlined, more strongly if [selected]
 * (the page being read).
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
    /** Picked to act on (see [PagesGrid]): tinted, with a tick in its corner. */
    picked: Boolean = false,
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
    ) {
        if (picked) {
            val accent = MaterialTheme.colorScheme.primary
            Box(Modifier.matchParentSize().background(accent.copy(alpha = 0.25f)))
            Icon(
                Icons.Filled.Check,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.onPrimary,
                modifier = Modifier
                    .align(Alignment.TopStart)
                    .padding(6.dp)
                    .size(22.dp)
                    .background(accent, CircleShape)
                    .padding(3.dp),
            )
        }
    }
}

/**
 * All pages as a grid of thumbnails; tapping one calls [onPick] with its
 * zero-based index.
 *
 * A long press picks a page instead, to act on: from then on, until none is
 * picked any more, a tap picks further pages or lets go of them. Which pages
 * are picked is in [PdfViewState.selectedPages], and what can be done with
 * them is in the bar above (see [SelectionBar]).
 */
@Composable
fun PagesGrid(
    state: PdfViewState,
    onPick: (Int) -> Unit,
    modifier: Modifier = Modifier,
    /**
     * Shown beside the document rather than in its place: one column of
     * thumbnails, which scrolls to keep the current page in view.
     */
    beside: Boolean = false,
) {
    val current = state.currentPage
    val grid = rememberLazyGridState(initialFirstVisibleItemIndex = current)
    val owner = remember { Any() }
    val haptics = LocalHapticFeedback.current
    val picked = state.selectedPages
    val picking = picked.isNotEmpty()

    LaunchedEffect(state, grid) {
        snapshotFlow {
            val visible = grid.layoutInfo.visibleItemsInfo
            if (visible.isEmpty()) 0 to 0 else visible.first().index to visible.size + THUMBNAIL_PREFETCH
        }.distinctUntilChanged().collect { (first, count) -> state.showThumbnails(owner, first, count) }
    }
    DisposableEffect(state) {
        onDispose { state.hideThumbnails(owner) }
    }
    if (beside) {
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
        columns = if (beside) GridCells.Fixed(1) else GridCells.Adaptive(minSize = 104.dp),
        state = grid,
        modifier = modifier,
        // One column would make thumbnails as wide as the sidebar: keep them
        // at a size where a few fit above each other.
        contentPadding = if (beside) PaddingValues(horizontal = 72.dp, vertical = 16.dp) else PaddingValues(16.dp),
        horizontalArrangement = Arrangement.spacedBy(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        items(state.pageCount, key = { it }) { page ->
            Column(
                Modifier
                    .combinedClickable(
                        onLongClick = {
                            haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                            state.togglePage(page)
                        },
                        onClick = { if (picking) state.togglePage(page) else onPick(page) },
                    )
                    // Screen readers (and UI tests) hear which pages are picked.
                    .then(if (picking) Modifier.semantics { selected = page in picked } else Modifier),
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
                        picked = page in picked,
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
