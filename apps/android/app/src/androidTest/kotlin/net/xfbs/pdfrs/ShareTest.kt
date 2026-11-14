package net.xfbs.pdfrs

import android.app.Activity
import android.app.Instrumentation
import android.content.Intent
import android.net.Uri
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasStateDescription
import androidx.compose.ui.test.longClick
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performTouchInput
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

/**
 * "Share…": what the app hands to the system's share sheet. The sheet itself
 * belongs to the system and never opens here; the request for it is caught
 * on its way out.
 *
 * The tests open documents by file path, which other apps may not be given,
 * so what they see shared is the copy the app makes for such cases. Passing
 * on a document that came from another app can only be checked by hand.
 */
@RunWith(AndroidJUnit4::class)
class ShareTest : AppTest() {
    /** Catches share sheets on their way to the system and answers in its place. */
    private class ShareSheets : Instrumentation.ActivityMonitor() {
        @Volatile
        var last: Intent? = null

        override fun onStartActivity(intent: Intent): Instrumentation.ActivityResult? {
            if (intent.action != Intent.ACTION_CHOOSER) return null
            last = intent
            return Instrumentation.ActivityResult(Activity.RESULT_CANCELED, null)
        }
    }

    private val sheets = ShareSheets()

    @Before
    fun catchShareSheets() = instrumentation.addMonitor(sheets)

    @After
    fun letShareSheetsThrough() = instrumentation.removeMonitor(sheets)

    @Suppress("DEPRECATION") // The typed variants need Android 13.
    @Test
    fun sharingOffersTheDocumentToOtherApps() {
        open("pages.pdf", shows = "1 / 12")
        menu("Share…")
        compose.waitUntil(WAIT_MS) { sheets.last != null }

        val send = sheets.last!!.getParcelableExtra<Intent>(Intent.EXTRA_INTENT)!!
        assertEquals(Intent.ACTION_SEND, send.action)
        assertEquals("application/pdf", send.type)
        assertTrue("may be read by the receiver", send.flags and Intent.FLAG_GRANT_READ_URI_PERMISSION != 0)
        val stream = send.getParcelableExtra<Uri>(Intent.EXTRA_STREAM)!!
        assertEquals("content", stream.scheme)
        assertEquals("under its own name", "pages.pdf", stream.lastPathSegment)
        assertEquals(stream, send.clipData!!.getItemAt(0).uri)

        val shared = instrumentation.targetContext.contentResolver.openInputStream(stream)!!.use { it.readBytes() }
        val original = instrumentation.context.assets.open("pages.pdf").use { it.readBytes() }
        assertArrayEquals(original, shared)
        // Still reading afterwards.
        compose.onNodeWithText("1 / 12").assertIsDisplayed()
    }

    @Test
    fun aDocumentThatIsGoneCannotBeShared() {
        open("pages.pdf", shows = "1 / 12")
        java.io.File(instrumentation.targetContext.cacheDir, "pages.pdf").delete()
        menu("Share…")
        // Nothing to hand over: no share sheet, and the reader stays put.
        compose.waitForIdle()
        Thread.sleep(1_000)
        assertEquals(null, sheets.last)
        compose.onNodeWithText("1 / 12").assertIsDisplayed()
    }

    @Suppress("DEPRECATION")
    @Test
    fun selectedTextCanBeShared() {
        open("pages.pdf", shows = "1 / 12")
        // "Page", in the heading "Page 1 of 12".
        page.performTouchInput { longClick(pagePoint(85f, fromTop = 62f)) }
        compose.waitUntil(WAIT_MS) { selected() }
        textMenu("Share")
        compose.waitUntil(WAIT_MS) { sheets.last != null }

        val send = sheets.last!!.getParcelableExtra<Intent>(Intent.EXTRA_INTENT)!!
        assertEquals(Intent.ACTION_SEND, send.action)
        assertEquals("text/plain", send.type)
        assertEquals("Page", send.getStringExtra(Intent.EXTRA_TEXT))
        assertEquals("only the text", null, send.getParcelableExtra<Uri>(Intent.EXTRA_STREAM))
        // Sharing lets go of the selection, as copying does.
        compose.waitUntil(WAIT_MS) { !selected() }
    }

    private fun selected(): Boolean =
        compose.onAllNodes(hasStateDescription("Text selected")).fetchSemanticsNodes().isNotEmpty()
}
