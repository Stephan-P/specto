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
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.flow.distinctUntilChanged

/** Shape of a grid cell; pages are fitted into it whatever their own shape. */
private const val CELL_ASPECT = 0.72f

/** Thumbnails to prepare beyond the visible ones, for scrolling down. */
private const val PREFETCH = 6

/**
 * All pages as a grid of thumbnails; tapping one calls [onPick] with its
 * zero-based index. The thumbnails are the page view's own low-resolution
 * placeholder tiles, so nothing is rendered twice.
 */
@Composable
fun PagesGrid(state: PdfViewState, onPick: (Int) -> Unit, modifier: Modifier = Modifier) {
    val current = state.frame?.currentPage ?: 0
    val grid = rememberLazyGridState(initialFirstVisibleItemIndex = current)
    val paint = remember { Paint().apply { isFilterBitmap = true } }

    LaunchedEffect(state, grid) {
        snapshotFlow {
            val visible = grid.layoutInfo.visibleItemsInfo
            if (visible.isEmpty()) 0 to 0 else visible.first().index to visible.size + PREFETCH
        }.distinctUntilChanged().collect { (first, count) -> state.showThumbnails(first, count) }
    }
    DisposableEffect(state) {
        onDispose { state.showThumbnails(0, 0) }
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
                    val bitmap = state.frame?.thumbnailId(page)?.takeIf { it != 0L }?.let(state::bitmap)
                    val outline =
                        if (page == current) {
                            Modifier.border(3.dp, MaterialTheme.colorScheme.primary)
                        } else {
                            Modifier.border(1.dp, MaterialTheme.colorScheme.outlineVariant)
                        }
                    Box(
                        Modifier
                            .aspectRatio(aspect, matchHeightConstraintsFirst = aspect < CELL_ASPECT)
                            .fillMaxSize()
                            .background(Color.White)
                            .drawBehind {
                                // Not `Image`: the tile cache may recycle a bitmap,
                                // and drawing a recycled one would throw.
                                if (bitmap != null && !bitmap.isRecycled) {
                                    drawIntoCanvas {
                                        it.nativeCanvas.drawBitmap(
                                            bitmap, null, RectF(0f, 0f, size.width, size.height), paint,
                                        )
                                    }
                                }
                            }
                            .then(outline),
                    )
                }
                Text(
                    "${page + 1}",
                    Modifier.padding(top = 4.dp),
                    style = MaterialTheme.typography.labelMedium,
                    color = if (page == current) MaterialTheme.colorScheme.primary else Color.Unspecified,
                )
            }
        }
    }
}
