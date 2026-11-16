package net.xfbs.specto

import androidx.annotation.StringRes
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Check
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.PathBuilder
import androidx.compose.ui.graphics.vector.path
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp

/** How pages are laid out and moved through. */
enum class ViewLayout(@param:StringRes val label: Int, val paged: Boolean, val twoPages: Boolean) {
    SinglePage(R.string.layout_single_page, paged = true, twoPages = false),
    Continuous(R.string.layout_continuous, paged = false, twoPages = false),
    TwoPages(R.string.layout_two_pages, paged = true, twoPages = true),
    TwoPagesContinuous(R.string.layout_two_pages_continuous, paged = false, twoPages = true),
    ;

    companion object {
        fun of(paged: Boolean, twoPages: Boolean): ViewLayout =
            entries.first { it.paged == paged && it.twoPages == twoPages }

        /**
         * What a window can show: two pages side by side need a [wide] one.
         *
         * (A narrow window shows the two-page layouts one page at a time.)
         */
        fun choices(wide: Boolean): List<ViewLayout> =
            if (wide) entries else entries.filter { !it.twoPages }
    }
}

/**
 * The layout this window shows: one that needs more room than there is falls back to single pages.
 */
fun PdfViewState.layout(wide: Boolean): ViewLayout = ViewLayout.of(paged, twoPages && wide)

/**
 * Switches to [layout].
 *
 * On a narrow window, which only offers single pages, a wish for two pages is kept for when there
 * is room again.
 */
fun PdfViewState.show(layout: ViewLayout, wide: Boolean) {
    paged = layout.paged
    if (wide) setSpread(layout.twoPages, coverAlone)
}

/** Identifies the view button to UI tests. Its description names the current layout. */
const val VIEW_BUTTON_TAG = "view-button"

/**
 * The top bar's view button and its menu: everything about how this document is shown, all of it
 * remembered for the document. The button's icon is the current layout.
 */
@Composable
fun ViewMenu(state: PdfViewState, wide: Boolean) {
    var open by remember { mutableStateOf(false) }
    val current = state.layout(wide)
    IconButton(onClick = { open = true }, modifier = Modifier.testTag(VIEW_BUTTON_TAG)) {
        Icon(
            current.icon,
            contentDescription = stringResource(
                R.string.view_description,
                stringResource(current.label),
            ),
        )
    }
    DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
        for (layout in ViewLayout.choices(wide)) {
            Choice(stringResource(layout.label), chosen = layout == current, icon = layout.icon) {
                open = false
                state.show(layout, wide)
            }
        }
        if (current.twoPages) {
            HorizontalDivider()
            Choice(stringResource(R.string.first_page_alone), chosen = state.coverAlone) {
                open = false
                state.setSpread(true, !state.coverAlone)
            }
        }
        HorizontalDivider()
        Choice(stringResource(R.string.fit_page), chosen = !state.fitWidth) {
            open = false
            state.fit(width = false)
        }
        Choice(stringResource(R.string.fit_width), chosen = state.fitWidth) {
            open = false
            state.fit(width = true)
        }
    }
}

/** A menu item that is ticked when [chosen]. */
@Composable
private fun Choice(label: String, chosen: Boolean, icon: ImageVector? = null, onClick: () -> Unit) {
    DropdownMenuItem(
        text = { Text(label) },
        leadingIcon = icon?.let { { Icon(it, contentDescription = null) } },
        trailingIcon = {
            if (chosen) {
                Icon(
                    Icons.Filled.Check,
                    contentDescription = stringResource(R.string.chosen),
                )
            }
        },
        onClick = onClick,
    )
}

// --- icons ------------------------------------------------------------------
//
// Little pictures of the layouts: whole pages for the paged ones, and for the continuous ones a
// page between the edges of its neighbors.

private fun layoutIcon(name: String, pages: PathBuilder.() -> Unit): ImageVector =
    ImageVector.Builder(name, 24.dp, 24.dp, 24f, 24f).apply {
        path(
            stroke = SolidColor(androidx.compose.ui.graphics.Color.Black),
            strokeLineWidth = 2f,
            strokeLineCap = StrokeCap.Round,
            strokeLineJoin = StrokeJoin.Round,
            pathBuilder = pages,
        )
    }.build()

/** A whole page between [left] and [right]. */
private fun PathBuilder.page(left: Float, right: Float) {
    moveTo(left, 4f)
    horizontalLineTo(right)
    verticalLineTo(20f)
    horizontalLineTo(left)
    close()
}

/** A page with the edges of the pages before and after it, which scrolling brings in. */
private fun PathBuilder.scrollingPages(left: Float, right: Float) {
    moveTo(left, 7f)
    horizontalLineTo(right)
    verticalLineTo(17f)
    horizontalLineTo(left)
    close()
    moveTo(left, 3f)
    horizontalLineTo(right)
    moveTo(left, 21f)
    horizontalLineTo(right)
}

private val singlePageIcon = layoutIcon("SinglePage") { page(6f, 18f) }
private val continuousIcon = layoutIcon("Continuous") { scrollingPages(6f, 18f) }
private val twoPagesIcon = layoutIcon("TwoPages") {
    page(2f, 10f)
    page(14f, 22f)
}
private val twoPagesContinuousIcon = layoutIcon("TwoPagesContinuous") {
    scrollingPages(2f, 10f)
    scrollingPages(14f, 22f)
}

val ViewLayout.icon: ImageVector
    get() = when (this) {
        ViewLayout.SinglePage -> singlePageIcon
        ViewLayout.Continuous -> continuousIcon
        ViewLayout.TwoPages -> twoPagesIcon
        ViewLayout.TwoPagesContinuous -> twoPagesContinuousIcon
    }
