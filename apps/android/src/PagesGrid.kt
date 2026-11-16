package net.xfbs.specto

import android.graphics.Paint
import android.graphics.RectF
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.awaitLongPressOrCancellation
import androidx.compose.foundation.gestures.scrollBy
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
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.drawIntoCanvas
import androidx.compose.ui.graphics.nativeCanvas
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.boundsInRoot
import androidx.compose.ui.layout.layout
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.positionInRoot
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.onLongClick
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import kotlin.math.roundToInt
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.distinctUntilChanged

/** Shape of a grid cell. Pages are fitted into it whatever their own shape. */
private const val CELL_ASPECT = 0.72f

/** How long the page must stay the same before a following grid scrolls to it. */
private const val FOLLOW_DELAY_MS = 300L

/** Identifies the page grid to UI tests. */
const val PAGES_GRID_TAG = "pages-grid"

/**
 * How near the top or bottom edge dragged pages make the grid scroll, and how fast at most (per
 * second).
 */
private val DRAG_SCROLL_ZONE = 56.dp
private val DRAG_SCROLL_SPEED = 900.dp

/** The mark where dragged pages would go: how far beside the thumbnail, and how thick. */
private val DROP_MARK_GAP = 8.dp
private val DROP_MARK_WIDTH = 4.dp

/** How far above the finger the count of dragged pages floats. */
private val DRAG_BADGE_LIFT = 36.dp

private fun Dp.toPx(density: Float): Float = value * density

/** Thumbnails to prepare beyond the visible ones, for scrolling on. */
const val THUMBNAIL_PREFETCH = 6

private val thumbnailPaint = Paint().apply { isFilterBitmap = true }

/** Draws the marks on the thumbnails. One for all of them: they are drawn one after another. */
private val thumbnailShapes = ShapePainter()
private val thumbnailBounds = RectF()

/**
 * The thumbnail of [page] on a white sheet, filling the space it is given (the caller sets the
 * aspect ratio), outlined, more strongly if [selected] (the page being read).
 *
 * Blank until the thumbnail is rendered. The thumbnails are the page view's own low-resolution
 * placeholder tiles, so nothing is rendered twice. Annotations made since the file was read are
 * drawn over them, as in the page view. The caller must announce the pages it shows with
 * [PdfViewState.showThumbnails].
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
    val marks = state.thumbnailFrame
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
                // Not `Image`: the tile cache may recycle a bitmap, and drawing a recycled one
                // would throw.
                if (bitmap != null && !bitmap.isRecycled) {
                    drawIntoCanvas {
                        it.nativeCanvas.drawBitmap(
                            bitmap,
                            null,
                            RectF(0f, 0f, size.width, size.height),
                            thumbnailPaint,
                        )
                    }
                }
                // What was marked since the file was read isn't in the bitmap.
                if (marks != null) {
                    drawIntoCanvas {
                        val canvas = it.nativeCanvas
                        val scale = size.width / ViewFrame.THUMBNAIL_UNITS
                        val count = canvas.save()
                        canvas.scale(scale, scale)
                        thumbnailBounds.set(0f, 0f, ViewFrame.THUMBNAIL_UNITS, size.height / scale)
                        val slot = marks.thumbnailSlot(page)
                        thumbnailShapes.draw(canvas, marks.thumbnailShapes, slot, thumbnailBounds)
                        canvas.restoreToCount(count)
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
 * All pages as a grid of thumbnails. Tapping one calls [onPick] with its zero-based index.
 *
 * A long press picks a page instead, to act on: from then on, until none is picked any more, a tap
 * picks further pages or lets go of them. Which pages are picked is in
 * [PdfViewState.selectedPages], and what can be done with them is in the bar above (see
 * [SelectionBar]).
 *
 * A long press that moves on drags the picked pages: a mark between two thumbnails shows where they
 * would go, the grid scrolls on at its top and bottom edge, and lifting the finger moves them
 * there.
 */
@Composable
fun PagesGrid(
    state: PdfViewState,
    onPick: (Int) -> Unit,
    modifier: Modifier = Modifier,
    /**
     * Shown beside the document rather than in its place: one column of thumbnails, which scrolls
     * to keep the current page in view.
     */
    beside: Boolean = false,
) {
    val current = state.currentPage
    val pick = stringResource(R.string.pick)
    val grid = rememberLazyGridState(initialFirstVisibleItemIndex = current)
    val owner = remember { Any() }
    val haptics = LocalHapticFeedback.current
    val density = LocalDensity.current.density
    val picked = state.selectedPages
    val picking = picked.isNotEmpty()

    LaunchedEffect(state, grid) {
        snapshotFlow {
            val visible = grid.layoutInfo.visibleItemsInfo
            if (visible.isEmpty()) {
                0 to 0
            } else {
                visible.first().index to visible.size + THUMBNAIL_PREFETCH
            }
        }.distinctUntilChanged().collect { (first, count) ->
            state.showThumbnails(owner, first, count)
        }
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

    // Picking and dragging
    val drag = remember(state) { PageDragState() }
    val accent = MaterialTheme.colorScheme.primary
    var origin by remember { mutableStateOf(Offset.Zero) }
    val dragging = drag.pointer != null
    val drop = drag.drop(oneColumn = beside)

    // Dragged to the top or bottom edge, the grid scrolls on.
    LaunchedEffect(drag, dragging) {
        if (!dragging) return@LaunchedEffect
        var last = withFrameNanos { it }
        while (true) {
            val now = withFrameNanos { it }
            val seconds = (now - last) / 1e9f
            last = now
            val y = (drag.pointer ?: break).y - origin.y
            val zone = DRAG_SCROLL_ZONE.toPx(density)
            val height = grid.layoutInfo.viewportSize.height
            val push = when {
                y < zone -> (y - zone) / zone
                y > height - zone -> (y - (height - zone)) / zone
                else -> 0f
            }.coerceIn(-1f, 1f)
            if (push != 0f) grid.scrollBy(push * DRAG_SCROLL_SPEED.toPx(density) * seconds)
        }
    }

    Box(
        modifier
            .onGloballyPositioned { origin = it.positionInRoot() }
            .pointerInput(state, beside) {
                awaitEachGesture {
                    val down = awaitFirstDown(requireUnconsumed = false)
                    // Canceled when the finger lifts or the grid scrolls first.
                    val held = awaitLongPressOrCancellation(down.id) ?: return@awaitEachGesture
                    val at = held.position + origin
                    val page = drag.cells.entries.firstOrNull { it.value.contains(at) }?.key
                        ?: return@awaitEachGesture
                    haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                    val wasPicked = page in state.selectedPages
                    if (!wasPicked) state.togglePage(page)
                    var moved = false
                    try {
                        while (true) {
                            // Before the grid sees it, which would scroll.
                            val event = awaitPointerEvent(PointerEventPass.Initial)
                            val change = event.changes.firstOrNull { it.id == down.id } ?: break
                            change.consume()
                            if (!change.pressed) break
                            val to = change.position
                            val far = (to - held.position).getDistance()
                            if (far > viewConfiguration.touchSlop) moved = true
                            if (moved) drag.pointer = to + origin
                        }
                        val target = drag.drop(oneColumn = beside)
                        when {
                            moved -> target?.let {
                                state.movePages(state.selectedPages, it.position)
                            }

                            // A long press on a picked page lets go of it.
                            wasPicked -> state.togglePage(page)
                        }
                    } finally {
                        drag.pointer = null
                    }
                }
            }
            .drawWithContent {
                drawContent()
                // The mark where dragged pages would go: a bar beside the thumbnail they would come
                // before or after.
                val cell = drop?.let {
                    drag.cells[it.cell]
                }?.translate(-origin) ?: return@drawWithContent
                val gap = DROP_MARK_GAP.toPx()
                val width = DROP_MARK_WIDTH.toPx()
                if (beside) {
                    val y = if (drop.before) cell.top - gap else cell.bottom + gap
                    drawLine(
                        accent,
                        Offset(cell.left, y),
                        Offset(cell.right, y),
                        width,
                        StrokeCap.Round,
                    )
                } else {
                    val x = if (drop.before) cell.left - gap else cell.right + gap
                    drawLine(
                        accent,
                        Offset(x, cell.top),
                        Offset(x, cell.bottom),
                        width,
                        StrokeCap.Round,
                    )
                }
            },
    ) {
        LazyVerticalGrid(
            columns = if (beside) GridCells.Fixed(1) else GridCells.Adaptive(minSize = 104.dp),
            state = grid,
            modifier = Modifier.fillMaxSize().testTag(PAGES_GRID_TAG),
            // One column would make thumbnails as wide as the sidebar: keep them at a size where a
            // few fit above each other.
            contentPadding = if (beside) {
                PaddingValues(
                    horizontal = 72.dp,
                    vertical = 16.dp,
                )
            } else {
                PaddingValues(16.dp)
            },
            horizontalArrangement = Arrangement.spacedBy(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            items(state.pageCount, key = { it }) { page ->
                DisposableEffect(page) {
                    onDispose { drag.cells.remove(page) }
                }
                Column(
                    Modifier
                        .onGloballyPositioned { drag.cells[page] = it.boundsInRoot() }
                        .clickable { if (picking) state.togglePage(page) else onPick(page) }
                        .semantics {
                            // Screen readers (and UI tests) hear which pages are picked, and can
                            // pick without the gesture.
                            if (picking) selected = page in picked
                            onLongClick(pick) {
                                state.togglePage(page)
                                true
                            }
                        }
                        // The pages being dragged fade where they still are.
                        .alpha(if (dragging && page in picked) 0.4f else 1f),
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    Box(
                        Modifier.fillMaxWidth().aspectRatio(CELL_ASPECT),
                        contentAlignment = Alignment.Center,
                    ) {
                        val aspect = state.pageAspect(page)
                        PageThumbnail(
                            state,
                            page,
                            selected = page == current,
                            Modifier
                                .aspectRatio(
                                    aspect,
                                    matchHeightConstraintsFirst = aspect < CELL_ASPECT,
                                )
                                .fillMaxSize(),
                            picked = page in picked,
                        )
                    }
                    Text(
                        state.pageLabel(page),
                        Modifier.padding(top = 4.dp),
                        style = MaterialTheme.typography.labelMedium,
                        color = if (page == current) {
                            MaterialTheme.colorScheme.primary
                        } else {
                            Color.Unspecified
                        },
                        maxLines = 1,
                    )
                }
            }
        }
        // What is being dragged: above the finger, and kept on screen.
        drag.pointer?.let { pointer ->
            Text(
                pluralStringResource(R.plurals.pages_dragged, picked.size, picked.size),
                Modifier
                    .layout { measurable, constraints ->
                        val badge = measurable.measure(
                            constraints.copy(minWidth = 0, minHeight = 0),
                        )
                        layout(constraints.maxWidth, constraints.maxHeight) {
                            val x = (pointer.x - origin.x - badge.width / 2f).roundToInt()
                            val lift = DRAG_BADGE_LIFT.toPx() + badge.height
                            val y = (pointer.y - origin.y - lift).roundToInt()
                            badge.place(
                                x.coerceIn(0, maxOf(constraints.maxWidth - badge.width, 0)),
                                y.coerceAtLeast(0),
                            )
                        }
                    }
                    .shadow(4.dp, CircleShape)
                    .background(accent, CircleShape)
                    .padding(horizontal = 12.dp, vertical = 6.dp),
                color = MaterialTheme.colorScheme.onPrimary,
                style = MaterialTheme.typography.labelLarge,
                maxLines = 1,
            )
        }
    }
}
