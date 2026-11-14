package net.xfbs.pdfrs

import android.content.ClipData
import android.content.ClipboardManager
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.click
import androidx.compose.ui.test.hasStateDescription
import androidx.compose.ui.test.longClick
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performKeyInput
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.pressKey
import androidx.compose.ui.test.withKeyDown
import androidx.test.espresso.Espresso
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Selecting and copying text. Every page of `pages.pdf` is headed "Page n of
 * 12" in bold 22 pt type, starting 62 pt from the left with its baseline
 * 70 pt below the top ("Page" ends at about 115 pt, "of" spans about 139 to
 * 160, the line ends at about 190); nothing is printed in the lowest part
 * of a page.
 *
 * The highlight and the handles are drawn on the canvas and the copy menu
 * is the system's, so the tests see none of them: they go by whether the
 * page says that text is selected, and by what lands on the clipboard
 * (copied with Ctrl+C, which does what the menu's "Copy" does).
 */
@RunWith(AndroidJUnit4::class)
class SelectionTest : AppTest() {
    private val clipboard: ClipboardManager
        get() = instrumentation.targetContext.getSystemService(ClipboardManager::class.java)

    @Before
    fun emptyClipboard() = instrumentation.runOnMainSync {
        clipboard.setPrimaryClip(ClipData.newPlainText("", ""))
    }

    private fun openPages() = open("pages.pdf", shows = "1 / 12")

    private fun isSelected(): Boolean =
        compose.onAllNodes(hasStateDescription("Text selected")).fetchSemanticsNodes().isNotEmpty()

    private fun waitForSelection(present: Boolean = true) = compose.waitUntil(WAIT_MS) { isSelected() == present }

    private fun copied(): String {
        var text = ""
        instrumentation.runOnMainSync { text = clipboard.primaryClip?.getItemAt(0)?.text?.toString().orEmpty() }
        return text
    }

    /** Copies the selection with the keyboard and returns what was copied. */
    private fun copy(): String {
        page.performKeyInput { withKeyDown(Key.CtrlLeft) { pressKey(Key.C) } }
        // Copying lets go of the selection.
        waitForSelection(false)
        compose.waitUntil(WAIT_MS) { copied().isNotEmpty() }
        return copied()
    }

    /** A point on the heading's line, [x] points from the page's left edge. */
    private fun androidx.compose.ui.test.TouchInjectionScope.heading(x: Float) = pagePoint(x, fromTop = 62f)

    @Test
    fun aLongPressSelectsAWord() {
        openPages()
        page.performTouchInput { longClick(heading(85f)) }
        waitForSelection()
        assertEquals("Page", copy())
    }

    @Test
    fun aLongPressBesideTheTextSelectsNothing() {
        openPages()
        page.performTouchInput { longClick(pagePoint(300f, fromTop = 800f)) }
        compose.waitForIdle()
        assertEquals(false, isSelected())
        // And it is not taken for a tap either: the bars are still there.
        compose.onNodeWithContentDescription("More").assertIsDisplayed()
    }

    @Test
    fun movingOnAfterTheLongPressSelectsMoreWords() {
        openPages()
        page.performTouchInput { down(heading(85f)) }
        waitForSelection()
        page.performTouchInput {
            moveTo(heading(125f))
            moveTo(heading(150f))
            up()
        }
        assertEquals("Page 1 of", copy())
    }

    @Test
    fun theHandlesMoveTheEnds() {
        openPages()
        page.performTouchInput { longClick(heading(150f)) }
        waitForSelection()
        // The handles hang below the line: the end one to the right of the
        // word's end, the start one to the left of its start.
        page.performTouchInput {
            val handle = Offset(10 * density, 10 * density)
            down(pagePoint(160f, fromTop = 75f) + handle)
            moveTo(pagePoint(200f, fromTop = 75f) + handle)
            // Well beyond the end of the line.
            moveTo(pagePoint(260f, fromTop = 75f) + handle)
            up()
        }
        page.performTouchInput {
            val handle = Offset(-10 * density, 10 * density)
            down(pagePoint(139f, fromTop = 75f) + handle)
            moveTo(pagePoint(130f, fromTop = 75f) + handle)
            // Just after the start of "1".
            moveTo(pagePoint(124f, fromTop = 75f) + handle)
            up()
        }
        assertTrue(isSelected())
        assertEquals("1 of 12", copy())
    }

    @Test
    fun draggingToTheEdgeScrollsOnWhenScrollingContinuously() {
        openPages()
        view("Continuous")
        // Scrolling, pages are fitted to the width and the first one starts
        // at the top: "Page", in its heading.
        page.performTouchInput {
            val margin = 8 * density
            val zoom = (width - 2 * margin) / 595f
            down(Offset(margin + 85f * zoom, margin + 62f * zoom))
        }
        waitForSelection()
        // Holding the finger at the bottom edge keeps the view scrolling for
        // as long as it stays there, so time is moved on by hand here.
        compose.mainClock.autoAdvance = false
        try {
            page.performTouchInput {
                moveTo(Offset(centerX, centerY))
                moveTo(Offset(centerX, bottom - 5f))
            }
            compose.mainClock.advanceTimeBy(1_500)
            page.performTouchInput { up() }
        } finally {
            compose.mainClock.autoAdvance = true
        }
        waitUntilGone("1 / 12")
        // From the first page's heading into a later page.
        val text = copy()
        assertTrue(text.take(40), text.startsWith("Page 1 of 12\n"))
        assertTrue("Page 2 of 12\n" in text)
    }

    @Test
    fun aTapOrBackLetsGoOfTheSelection() {
        openPages()
        page.performTouchInput { longClick(heading(85f)) }
        waitForSelection()
        // The tap only drops the selection: the bars stay.
        page.performTouchInput { click(pagePoint(300f, fromTop = 800f)) }
        waitForSelection(false)
        compose.onNodeWithContentDescription("More").assertIsDisplayed()

        page.performTouchInput { longClick(heading(85f)) }
        waitForSelection()
        Espresso.pressBack()
        waitForSelection(false)
        // Still reading.
        compose.onNodeWithText("1 / 12").assertIsDisplayed()
    }

    @Test
    fun everythingCanBeSelectedAndCopied() {
        openPages()
        page.performKeyInput { withKeyDown(Key.CtrlLeft) { pressKey(Key.A) } }
        waitForSelection()
        val text = copy()
        assertTrue(text.take(40), text.startsWith("Page 1 of 12\n"))
        assertTrue("Page 12 of 12\n" in text)
        // Twelve pages of a heading and forty lines each.
        assertEquals(12 * 41, text.lines().size)
    }
}
