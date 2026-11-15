package net.xfbs.specto

import android.app.UiAutomation
import android.content.res.Configuration
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.click
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performKeyInput
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.pressKey
import androidx.compose.ui.test.swipeLeft
import androidx.compose.ui.test.swipeRight
import androidx.compose.ui.test.swipeUp
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Reading a plain document: turning pages by swipe and key, the page grid,
 * full screen, the layouts for narrow and wide windows, and what is
 * remembered. They run on phones and tablets alike; where the two differ,
 * tests ask [wide].
 */
@RunWith(AndroidJUnit4::class)
class ReaderTest : AppTest() {
    private fun openPages() = open("pages.pdf", shows = "1 / 12")

    @Test
    fun opensOnTheFirstPage() {
        openPages()
        compose.onNodeWithText("pages.pdf").assertIsDisplayed()
        compose.onNodeWithText("1 / 12").assertIsDisplayed()
    }

    // --- turning pages ------------------------------------------------------

    @Test
    fun swipingTurnsOnePage() {
        openPages()
        page.performTouchInput { swipeLeft() }
        waitForText("2 / 12")
        page.performTouchInput { swipeLeft() }
        waitForText("3 / 12")
        page.performTouchInput { swipeRight() }
        waitForText("2 / 12")
    }

    @Test
    fun aShortSlowDragReturnsToThePage() {
        openPages()
        page.performTouchInput {
            down(center)
            // A quarter of the width, slowly, then held still: no flick.
            repeat(10) { moveBy(Offset(-width / 40f, 0f), delayMillis = 50) }
            advanceEventTime(300)
            up()
        }
        compose.waitForIdle()
        compose.onNodeWithText("1 / 12").assertIsDisplayed()
    }

    @Test
    fun aShortFlickTurnsThePage() {
        openPages()
        page.performTouchInput {
            down(center)
            // The same quarter of the width, but fast and released in motion.
            repeat(5) { moveBy(Offset(-width / 20f, 0f), delayMillis = 8) }
            up()
        }
        waitForText("2 / 12")
    }

    @Test
    fun keysTurnPages() {
        openPages()
        // The page has the focus from the start; no tap needed first.
        page.performKeyInput { pressKey(Key.DirectionRight) }
        waitForText("2 / 12")
        page.performKeyInput { pressKey(Key.PageDown) }
        waitForText("3 / 12")
        page.performKeyInput { pressKey(Key.Spacebar) }
        waitForText("4 / 12")
        // On a fitted page there is nothing to scroll: down turns the page too.
        page.performKeyInput { pressKey(Key.DirectionDown) }
        waitForText("5 / 12")
        page.performKeyInput { pressKey(Key.DirectionLeft) }
        waitForText("4 / 12")
        page.performKeyInput { pressKey(Key.Backspace) }
        waitForText("3 / 12")
        page.performKeyInput { pressKey(Key.MoveEnd) }
        waitForText("12 / 12")
        // Nothing beyond the last page.
        page.performKeyInput { pressKey(Key.DirectionRight) }
        compose.waitForIdle()
        compose.onNodeWithText("12 / 12").assertIsDisplayed()
        page.performKeyInput { pressKey(Key.MoveHome) }
        waitForText("1 / 12")
    }

    @Test
    fun goToPageFromTheMenu() {
        openPages()
        goTo("9", range = "1–12")
        waitForText("9 / 12")
    }

    @Test
    fun continuousScrollingKeepsThePage() {
        openPages()
        goTo("6", range = "1–12")
        waitForText("6 / 12")
        view("Continuous")
        waitForText("6 / 12")
        // Now the pages scroll up and down.
        page.performTouchInput { swipeUp() }
        waitUntilGone("6 / 12")
        page.performKeyInput { pressKey(Key.MoveEnd) }
        waitForText("12 / 12")
        view("Single page")
        waitForText("12 / 12")
    }

    @Test
    fun theViewMenuTicksWhatIsChosen() {
        openPages()
        compose.onNodeWithContentDescription("View: Single page").performClick()
        chosen("Single page").assertIsDisplayed()
        chosen("Fit page").assertIsDisplayed()
        chosen("Continuous").assertDoesNotExist()
        compose.onNodeWithText("Fit width").performClick()
        compose.onNodeWithTag(VIEW_BUTTON_TAG).performClick()
        chosen("Fit width").assertIsDisplayed()
        // Each layout starts with its own fit: the width when scrolling, the
        // whole page otherwise.
        compose.onNodeWithText("Fit page").performClick()
        view("Continuous")
        compose.onNodeWithContentDescription("View: Continuous").performClick()
        chosen("Fit width").assertIsDisplayed()
        compose.onNodeWithText("Single page").performClick()
        compose.onNodeWithTag(VIEW_BUTTON_TAG).performClick()
        chosen("Fit page").assertIsDisplayed()
    }

    @Test
    fun fittedToTheWidthATallPageScrollsBeforeItTurns() {
        // On a narrow window a page fitted to the width is all there anyway.
        assumeTrue("needs a window of at least $WIDE_WINDOW_DP dp", wide)
        openPages()
        view("Fit width")
        page.performKeyInput { pressKey(Key.PageDown) }
        compose.waitForIdle()
        compose.onNodeWithText("1 / 12").assertIsDisplayed()
        // A few screenfuls on, the page ends and the next one comes.
        compose.waitUntil(WAIT_MS) {
            page.performKeyInput { pressKey(Key.PageDown) }
            compose.waitForIdle()
            compose.onAllNodesWithText("2 / 12").fetchSemanticsNodes().isNotEmpty()
        }
        // The fit belongs to the document.
        reopen("pages.pdf")
        waitForText("2 / 12")
        compose.onNodeWithTag(VIEW_BUTTON_TAG).performClick()
        chosen("Fit width").assertIsDisplayed()
    }

    // --- pages, full screen, the strip --------------------------------------

    @Test
    fun thePageGridJumpsToAPage() {
        openPages()
        // Already there beside the document on a wide window.
        if (!wide) compose.onNodeWithContentDescription("Contents and pages").performClick()
        // The sidebar's single column shows fewer pages at once than the grid.
        val target = if (wide) "3" else "7"
        waitForText(target)
        compose.onNodeWithText(target).performClick()
        waitForText("$target / 12")
    }

    @Test
    fun aTapTogglesFullScreen() {
        openPages()
        page.performTouchInput { click(center) }
        // The bars and the page indicator leave ...
        waitUntilGone("pages.pdf")
        waitUntilGone("1 / 12")
        compose.waitUntil(WAIT_MS) {
            compose.onAllNodesWithContentDescription("More").fetchSemanticsNodes().isEmpty() &&
                compose.onAllNodesWithTag(SIDEBAR_TAG).fetchSemanticsNodes().isEmpty()
        }
        // ... and another tap brings them back.
        page.performTouchInput { click(center) }
        waitForText("pages.pdf")
        waitForText("1 / 12")
    }

    @Test
    fun scrollingTheStripFlipsThroughThePages() {
        assumeTrue("the strip gives way to the sidebar on wide windows", !wide)
        openPages()
        val strip = compose.onNodeWithTag(STRIP_TAG)
        strip.performTouchInput { swipeLeft() }
        waitUntilGone("1 / 12")
        // And back to the start.
        strip.performTouchInput { swipeRight() }
        strip.performTouchInput { swipeRight() }
        waitForText("1 / 12")
    }

    // --- narrow and wide windows --------------------------------------------

    @Test
    fun aWideWindowShowsPagesBesideTheDocument() {
        assumeTrue("needs a window of at least $WIDE_WINDOW_DP dp", wide)
        openPages()
        // Both at once, and without the thumbnail strip's job being done twice.
        compose.onNodeWithTag(SIDEBAR_TAG).assertIsDisplayed()
        page.assertIsDisplayed()
        compose.onNodeWithTag(STRIP_TAG).assertDoesNotExist()
        val sidebarWidth = compose.onNodeWithTag(SIDEBAR_TAG).fetchSemanticsNode().size.width
        val pageWidth = page.fetchSemanticsNode().size.width
        assertTrue("the document gets most of the width", pageWidth > 2 * sidebarWidth)

        // Picking a page keeps the sidebar; the button folds it away, and the
        // document takes the room.
        compose.onNodeWithText("3").performClick()
        waitForText("3 / 12")
        compose.onNodeWithTag(SIDEBAR_TAG).assertIsDisplayed()
        compose.onNodeWithContentDescription("Contents and pages").performClick()
        compose.waitUntil(WAIT_MS) { compose.onAllNodesWithTag(SIDEBAR_TAG).fetchSemanticsNodes().isEmpty() }
        compose.waitForIdle()
        assertTrue(page.fetchSemanticsNode().size.width > pageWidth)
        compose.onNodeWithText("3 / 12").assertIsDisplayed()
    }

    @Test
    fun aClosedSidebarStaysClosedForTheDocument() {
        assumeTrue("needs a window of at least $WIDE_WINDOW_DP dp", wide)
        openPages()
        compose.onNodeWithContentDescription("Contents and pages").performClick()
        compose.waitUntil(WAIT_MS) { compose.onAllNodesWithTag(SIDEBAR_TAG).fetchSemanticsNodes().isEmpty() }
        reopen("pages.pdf")
        waitForText("1 / 12")
        compose.waitForIdle()
        compose.onNodeWithTag(SIDEBAR_TAG).assertDoesNotExist()
        // The strip is there instead.
        compose.onNodeWithTag(STRIP_TAG).assertIsDisplayed()
    }

    @Test
    fun aNarrowWindowShowsPagesFullScreen() {
        assumeTrue("needs a window narrower than $WIDE_WINDOW_DP dp", !wide)
        openPages()
        compose.onNodeWithContentDescription("Contents and pages").performClick()
        waitForText("7")
        // In place of the document, until a page is picked or Back is pressed.
        compose.onNodeWithTag(PAGE_VIEW_TAG).assertDoesNotExist()
        compose.onNodeWithContentDescription("Back to document").performClick()
        waitForText("1 / 12")
        page.assertIsDisplayed()
    }

    @Test
    fun twoPagesSideBySideOnAWideWindow() {
        assumeTrue("needs a window of at least $WIDE_WINDOW_DP dp", wide)
        openPages()
        view("Two pages")
        waitForText("1–2 / 12")
        // A swipe now turns a pair of pages.
        page.performTouchInput { swipeLeft() }
        waitForText("3–4 / 12")
        // With the first page alone, the pairs are 2–3, 4–5, …
        view("First page alone")
        waitForText("2–3 / 12")
        page.performTouchInput { swipeLeft() }
        waitForText("4–5 / 12")
        // Back to one page at a time, on the same page.
        view("Single page")
        waitForText("4 / 12")
        page.performTouchInput { swipeLeft() }
        waitForText("5 / 12")
    }

    @Test
    fun noTwoPagesOnANarrowWindow() {
        assumeTrue("needs a window narrower than $WIDE_WINDOW_DP dp", !wide)
        openPages()
        compose.onNodeWithTag(VIEW_BUTTON_TAG).performClick()
        compose.onNodeWithText("Continuous").assertIsDisplayed()
        compose.onNodeWithText("Two pages").assertDoesNotExist()
        compose.onNodeWithText("First page alone").assertDoesNotExist()
    }

    @Test
    fun rotatingKeepsThePage() {
        openPages()
        page.performKeyInput { pressKey(Key.DirectionRight) }
        page.performKeyInput { pressKey(Key.DirectionRight) }
        waitForText("3 / 12")
        fun landscape(): Boolean {
            var orientation = Configuration.ORIENTATION_UNDEFINED
            scenario?.onActivity { orientation = it.resources.configuration.orientation }
            return orientation == Configuration.ORIENTATION_LANDSCAPE
        }
        val before = landscape()
        // Turning the device itself: large screens ignore an app's own
        // orientation requests.
        val device = instrumentation.uiAutomation
        try {
            device.setRotation(UiAutomation.ROTATION_FREEZE_90)
            // The window changes shape (on a tablet, between the sidebar and
            // the phone layout); the document doesn't reload.
            compose.waitUntil(WAIT_MS) { landscape() != before }
            waitForText("3 / 12")
            page.assertIsDisplayed()
            page.performTouchInput { swipeLeft() }
            waitForText("4 / 12")
        } finally {
            device.setRotation(UiAutomation.ROTATION_FREEZE_0)
            device.setRotation(UiAutomation.ROTATION_UNFREEZE)
        }
        compose.waitUntil(WAIT_MS) { landscape() == before }
        waitForText("4 / 12")
    }

    // --- what is remembered -------------------------------------------------

    @Test
    fun reopeningContinuesWhereTheReaderLeftOff() {
        openPages()
        page.performTouchInput { swipeLeft() }
        waitForText("2 / 12")
        page.performTouchInput { swipeLeft() }
        waitForText("3 / 12")
        // Leaving saves the position; the same file opens there again.
        reopen("pages.pdf")
        waitForText("3 / 12")
    }

    @Test
    fun twoPagesAreRememberedForTheDocument() {
        assumeTrue("needs a window of at least $WIDE_WINDOW_DP dp", wide)
        openPages()
        view("Two pages")
        page.performTouchInput { swipeLeft() }
        waitForText("3–4 / 12")
        // Leave and come back: still on that pair, still two pages at a time.
        reopen("pages.pdf")
        waitForText("3–4 / 12")
        page.performTouchInput { swipeLeft() }
        waitForText("5–6 / 12")
    }

    @Test
    fun recentDocumentsOnTheStartScreen() {
        openPages()
        page.performKeyInput { pressKey(Key.DirectionRight) }
        waitForText("2 / 12")
        menu("Close")
        waitForText("Recent")
        // Listed with where it was left; tapping it continues there.
        compose.onNodeWithText("Page 2 of 12", substring = true).assertIsDisplayed()
        compose.onNodeWithText("pages.pdf").performClick()
        waitForText("2 / 12")
    }
}
