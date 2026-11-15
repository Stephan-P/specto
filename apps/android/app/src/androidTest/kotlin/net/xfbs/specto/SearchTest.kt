package net.xfbs.specto

import androidx.compose.ui.input.key.Key
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performKeyInput
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.performTextReplacement
import androidx.compose.ui.test.pressKey
import androidx.compose.ui.test.withKeyDown
import androidx.test.espresso.Espresso
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Searching `pages.pdf`: every page is headed "Page n of 12" and has forty
 * lines of "the quick brown fox …". The highlights are drawn on the canvas,
 * so the tests go by the count in the search bar and by the page the view
 * lands on.
 */
@RunWith(AndroidJUnit4::class)
class SearchTest : AppTest() {
    private val field get() = compose.onNodeWithTag(SEARCH_FIELD_TAG)

    private fun openAndSearch(text: String) {
        open("pages.pdf", shows = "1 / 12")
        compose.onNodeWithContentDescription("Search").performClick()
        field.performTextInput(text)
    }

    private fun next() = compose.onNodeWithContentDescription("Next match").performClick()

    private fun previous() = compose.onNodeWithContentDescription("Previous match").performClick()

    @Test
    fun searchingGoesToTheFirstMatch() {
        openAndSearch("page 7 of")
        waitForText("1 of 1")
        waitForText("7 / 12")
    }

    @Test
    fun steppingThroughTheMatchesGoesAround() {
        openAndSearch("of 12")
        waitForText("1 of 12")
        next()
        waitForText("2 of 12")
        waitForText("2 / 12")
        // Before the first one comes the last.
        previous()
        previous()
        waitForText("12 of 12")
        waitForText("12 / 12")
        next()
        waitForText("1 of 12")
        waitForText("1 / 12")
    }

    @Test
    fun theWholeDocumentIsSearchedWhateverTheCase() {
        // Forty lines on each of twelve pages.
        openAndSearch("QUICK Brown")
        waitForText("1 of 480")
        // Typing on narrows it down again.
        field.performTextReplacement("Page 12 of")
        waitForText("1 of 1")
        waitForText("12 / 12")
    }

    @Test
    fun nothingFoundSaysSo() {
        openAndSearch("zebra")
        waitForText("No matches")
        compose.onNodeWithContentDescription("Next match").assertIsNotEnabled()
        compose.onNodeWithContentDescription("Previous match").assertIsNotEnabled()
    }

    @Test
    fun closingReturnsToReading() {
        openAndSearch("page 3 of")
        waitForText("3 / 12")
        compose.onNodeWithContentDescription("Close search").performClick()
        // The usual bar is back, on the page the search led to, and keys
        // turn pages again.
        compose.onNodeWithContentDescription("More").assertIsDisplayed()
        field.assertDoesNotExist()
        compose.onNodeWithText("3 / 12").assertIsDisplayed()
        page.performKeyInput { pressKey(Key.DirectionRight) }
        waitForText("4 / 12")

        // Ctrl+F opens it as well; Back first puts the keyboard away, then closes.
        page.performKeyInput { withKeyDown(Key.CtrlLeft) { pressKey(Key.F) } }
        field.assertIsDisplayed()
        Espresso.closeSoftKeyboard()
        Espresso.pressBack()
        compose.waitUntil(WAIT_MS) {
            compose.onAllNodes(androidx.compose.ui.test.hasTestTag(SEARCH_FIELD_TAG)).fetchSemanticsNodes().isEmpty()
        }
        compose.onNodeWithText("4 / 12").assertIsDisplayed()
    }
}
