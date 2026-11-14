package net.xfbs.pdfrs

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.click
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.performTouchInput
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Test
import org.junit.runner.RunWith

/**
 * What a document can bring along: an outline, links, printed page numbers
 * (`book.pdf`: i–iii, 1–9, A-1, A-2) and a password (`locked.pdf`).
 */
@RunWith(AndroidJUnit4::class)
class DocumentTest : AppTest() {
    private val firstPage = "i (1 / 14)"

    @Test
    fun printedPageNumbers() {
        open("book.pdf", shows = firstPage)
        // A printed number wins over a position: "3" is the sixth page.
        goTo("iii", range = "i–A-2")
        waitForText("iii (3 / 14)")
        goTo("A-2", range = "i–A-2")
        waitForText("A-2 (14 / 14)")
        goTo("3", range = "i–A-2")
        waitForText("3 (6 / 14)")
        // A number that is no printed one is a position.
        goTo("13", range = "i–A-2")
        waitForText("A-1 (13 / 14)")
    }

    @Test
    fun theContentsJumpToASection() {
        open("book.pdf", shows = firstPage)
        // On a wide window the sidebar is already there.
        if (!wide) compose.onNodeWithContentDescription("Contents and pages").performClick()
        compose.onNodeWithText("Contents").performClick()
        waitForText("2.1 Zoom and scroll")
        compose.onNodeWithText("2.1 Zoom and scroll").performClick()
        waitForText("3 (6 / 14)")
        page.assertIsDisplayed()
    }

    /**
     * Taps the point of the current page that is ([x], [fromTop]) points from
     * its top-left corner. The pages are A4 and shown fitted and centred.
     */
    private fun tapOnPage(x: Float, fromTop: Float) {
        page.performTouchInput {
            val margin = 8 * density
            val zoom = minOf((width - 2 * margin) / 595f, (height - 2 * margin) / 842f)
            val left = (width - 595f * zoom) / 2
            val top = (height - 842f * zoom) / 2
            click(Offset(left + x * zoom, top + fromTop * zoom))
        }
    }

    @Test
    fun aLinkInTheDocumentIsFollowed() {
        open("book.pdf", shows = firstPage)
        // The table of contents' line "2.2 Pages and contents ........ 7".
        tapOnPage(x = 200f, fromTop = 842f - 537f)
        waitForText("4 (7 / 14)")
        // Every other page links back, bottom left.
        tapOnPage(x = 100f, fromTop = 842f - 64f)
        waitForText(firstPage)
    }

    @Test
    fun aLinkToTheWebAsksFirst() {
        open("book.pdf", shows = firstPage)
        tapOnPage(x = 200f, fromTop = 842f - 375f)
        waitForText("Open this link?")
        compose.onNodeWithText("https://example.com/pdfrs").assertIsDisplayed()
        compose.onNodeWithText("Cancel").performClick()
        waitUntilGone("Open this link?")
        compose.onNodeWithText(firstPage).assertIsDisplayed()
    }

    @Test
    fun aPasswordProtectedDocument() {
        open("locked.pdf", shows = "Password required")
        compose.onNodeWithText("Password").performTextInput("nope")
        compose.onNodeWithText("Open").performClick()
        waitForText("That password didn't work.")
        compose.onNodeWithText("Password").performTextInput("secret")
        compose.onNodeWithText("Open").performClick()
        waitForText("1 / 3")
        compose.onNodeWithText("locked.pdf").assertIsDisplayed()
    }

    @Test
    fun cancellingThePasswordExplains() {
        open("locked.pdf", shows = "Password required")
        compose.onNodeWithText("Cancel").performClick()
        waitForText("Couldn't open this file")
        compose.onNodeWithText("Open another PDF").assertIsDisplayed()
    }
}
