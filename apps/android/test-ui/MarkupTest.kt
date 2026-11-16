package net.xfbs.specto

import androidx.compose.ui.graphics.toPixelMap
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.test.click
import androidx.compose.ui.test.hasStateDescription
import androidx.compose.ui.test.longClick
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performTouchInput
import androidx.test.espresso.Espresso
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Marking text: "Highlight", "Underline" and "Strike through" in the menu beside selected text, and
 * "Delete" for a mark that a tap picked.
 *
 * The text is the heading of `pages.pdf` (see `SelectionTest` for where it is).
 *
 * The marks are drawn on the canvas, where tests don't see them. They go by what the app says: that
 * the document is edited, what there is to undo, and that an annotation is picked. That a mark is
 * where the text is, and is saved with the document, is tested in Rust (`marking_text`). One test
 * looks at the pixels of the thumbnails, for the color of a highlight.
 */
@RunWith(AndroidJUnit4::class)
class MarkupTest : AppTest() {
    private fun openPages() = open("pages.pdf", shows = "1 / 12")

    private fun says(state: String): Boolean =
        compose.onAllNodes(hasStateDescription(state)).fetchSemanticsNodes().isNotEmpty()

    private fun waitForPicked(picked: Boolean = true) =
        compose.waitUntil(WAIT_MS) { says("Annotation selected") == picked }

    /** A point on the heading's line, [x] points from the page's left edge. */
    private fun androidx.compose.ui.test.TouchInjectionScope.heading(x: Float) =
        pagePoint(x, fromTop = 62f)

    /** Selects "Page" in the heading and marks it with [item] of the text menu. */
    private fun mark(item: String) {
        page.performTouchInput { longClick(heading(85f)) }
        compose.waitUntil(WAIT_MS) { says("Text selected") }
        textMenu(item)
        // The selection became the mark.
        compose.waitUntil(WAIT_MS) { !says("Text selected") }
        compose.onNodeWithText("Edited").assertIsDisplayed()
    }

    @Test
    fun selectedTextIsHighlightedAndTheMarkDeleted() {
        openPages()
        mark("Highlight")

        // A tap on the mark picks it, and does nothing else: the bars stay.
        page.performTouchInput { click(heading(85f)) }
        waitForPicked()
        compose.onNodeWithContentDescription("More").assertIsDisplayed()
        textMenu("Delete")
        waitForPicked(false)

        // Both were edits, taken back one by one.
        menu("Undo delete annotation")
        menu("Undo add annotation")
        compose.onAllNodesWithText("Edited").assertCountEquals(0)
        // Nothing is left to pick: the tap is one on the page again.
        menu("Redo add annotation")
        compose.onNodeWithText("Edited").assertIsDisplayed()
    }

    @Test
    fun aLineUnderOrThroughTheText() {
        openPages()
        mark("Underline")
        mark("Strike through")
        menu("Undo add annotation")
        menu("Undo add annotation")
        compose.onAllNodesWithText("Edited").assertCountEquals(0)
    }

    @Test
    fun aPickedMarkIsLetGoOfByATapElsewhereOrBack() {
        openPages()
        mark("Highlight")
        page.performTouchInput { click(heading(85f)) }
        waitForPicked()
        // On blank paper: lets go, without hiding the bars.
        page.performTouchInput { click(pagePoint(300f, fromTop = 800f)) }
        waitForPicked(false)
        compose.onNodeWithContentDescription("More").assertIsDisplayed()

        page.performTouchInput { click(heading(85f)) }
        waitForPicked()
        Espresso.pressBack()
        waitForPicked(false)
        compose.onNodeWithText("1 / 12").assertIsDisplayed()
        compose.onNodeWithText("Edited").assertIsDisplayed()
    }

    /**
     * How many pixels of the thumbnails are as yellow as a highlight: of the strip, or of the
     * sidebar where a wide window has that instead.
     */
    private fun yellowInThumbnails(): Int {
        val thumbnails = compose.onNodeWithTag(if (wide) SIDEBAR_TAG else STRIP_TAG)
        val pixels = thumbnails.captureToImage().toPixelMap()
        var count = 0
        for (y in 0 until pixels.height) {
            for (x in 0 until pixels.width) {
                val c = pixels[x, y]
                if (c.red > 0.8f && c.green > 0.7f && c.blue < 0.5f) count++
            }
        }
        return count
    }

    @Test
    fun aHighlightShowsInTheThumbnails() {
        openPages()
        compose.waitForIdle()
        assertEquals(0, yellowInThumbnails())
        mark("Highlight")
        compose.waitUntil(WAIT_MS) { yellowInThumbnails() > 0 }
        menu("Undo add annotation")
        compose.waitUntil(WAIT_MS) { yellowInThumbnails() == 0 }
    }
}
