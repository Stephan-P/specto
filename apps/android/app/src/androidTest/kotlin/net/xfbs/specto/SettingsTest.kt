package net.xfbs.specto

import android.view.KeyEvent
import android.view.WindowManager
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsOff
import androidx.compose.ui.test.assertIsOn
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith

/** Settings, from the ⋮ menu: what holds for the whole app, and what the switches there do. */
@RunWith(AndroidJUnit4::class)
class SettingsTest : AppTest() {
    private fun openPages() = open("pages.pdf", shows = "1 / 12")

    /** Flips the switch called [name] in Settings and returns to the document. */
    private fun flip(name: String) {
        menu("Settings")
        compose.onNodeWithText(name).performClick()
        compose.onNodeWithContentDescription("Back").performClick()
        waitUntilGone("Settings")
    }

    private fun keepsScreenOn(): Boolean {
        var flags = 0
        scenario!!.onActivity { flags = it.window.attributes.flags }
        return flags and WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON != 0
    }

    private fun press(key: Int) {
        // Keys go to the window that has the focus: not before a closing menu gave it back.
        compose.waitUntil(WAIT_MS) {
            var focused = false
            scenario!!.onActivity { focused = it.hasWindowFocus() }
            focused
        }
        instrumentation.sendKeyDownUpSync(key)
        compose.waitForIdle()
    }

    @Test
    fun switchesAreOffAtFirstAndStayAsSet() {
        openPages()
        menu("Settings")
        compose.onNodeWithText("Keep the screen on").assertIsOff()
        compose.onNodeWithText("Volume keys turn pages").assertIsOff().performClick()
        compose.onNodeWithText("Volume keys turn pages").assertIsOn()
        // Back returns to the document, where it was.
        compose.onNodeWithContentDescription("Back").performClick()
        waitForText("1 / 12")
        // Kept for the next time the app is opened.
        reopen("pages.pdf")
        waitForText("1 / 12")
        menu("Settings")
        compose.onNodeWithText("Volume keys turn pages").assertIsOn()
        compose.onNodeWithText("Keep the screen on").assertIsOff()
    }

    @Test
    fun theScreenIsKeptOnWhileReadingIfAskedTo() {
        openPages()
        assertEquals(false, keepsScreenOn())
        flip("Keep the screen on")
        assertEquals(true, keepsScreenOn())
        // Only while there is something to read.
        menu("Close")
        waitForText("Open PDF")
        assertEquals(false, keepsScreenOn())
    }

    @Test
    fun volumeKeysTurnPagesIfAskedTo() {
        openPages()
        flip("Volume keys turn pages")
        press(KeyEvent.KEYCODE_VOLUME_DOWN)
        waitForText("2 / 12")
        press(KeyEvent.KEYCODE_VOLUME_DOWN)
        waitForText("3 / 12")
        press(KeyEvent.KEYCODE_VOLUME_UP)
        waitForText("2 / 12")
        // Switched off again, they are the system's.
        flip("Volume keys turn pages")
        press(KeyEvent.KEYCODE_VOLUME_UP)
        compose.onNodeWithText("2 / 12").assertIsDisplayed()
    }
}
