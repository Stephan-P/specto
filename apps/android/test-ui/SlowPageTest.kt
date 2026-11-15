package net.xfbs.specto

import androidx.compose.ui.input.key.Key
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.click
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.performKeyInput
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.pressKey
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Pages that take long to draw.
 *
 * The first page of `slow.pdf` is one: a lot of small curves, and marked as heavy from the start by
 * an image its resources name (and nothing draws). Its second page is an ordinary one.
 */
@RunWith(AndroidJUnit4::class)
class SlowPageTest : AppTest() {
    private fun spinners() = compose.onAllNodesWithTag(BUSY_PAGE_TAG)

    @Test
    fun aSlowPageHasASpinnerUntilItIsSharp() {
        open("slow.pdf", shows = "1 / 2")
        compose.waitUntil(WAIT_MS) { spinners().fetchSemanticsNodes().isNotEmpty() }
        // It is in the middle of the page and lets touches through: a tap there hides the bars like
        // a tap anywhere else, and another one brings them back.
        page.performTouchInput { click(center) }
        compose.waitUntil(WAIT_MS) { compose.onAllNodesWithContentDescription("More").fetchSemanticsNodes().isEmpty() }
        page.performTouchInput { click(center) }
        compose.waitUntil(WAIT_MS) { compose.onAllNodesWithContentDescription("More").fetchSemanticsNodes().isNotEmpty() }
        // Drawn piece by piece, and an emulator in CI takes its time over each.
        compose.waitUntil(SHARP_MS) { spinners().fetchSemanticsNodes().isEmpty() }

        // An ordinary page never has one, not even before it is sharp.
        page.performKeyInput { pressKey(Key.DirectionRight) }
        waitForText("2 / 2")
        spinners().assertCountEquals(0)
        // And the slow one is kept: going back shows it at once.
        page.performKeyInput { pressKey(Key.DirectionLeft) }
        waitForText("1 / 2")
        spinners().assertCountEquals(0)
    }

    private companion object {
        /** How long the slow page may take to become sharp. */
        const val SHARP_MS = 180_000L
    }
}
