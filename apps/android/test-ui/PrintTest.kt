package net.xfbs.specto

import android.graphics.pdf.PdfRenderer
import android.os.ParcelFileDescriptor
import android.print.PrintManager
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.longClick
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.performTouchInput
import androidx.test.ext.junit.runners.AndroidJUnit4
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * "Print": what the app hands to the system's print dialog.
 *
 * The dialog itself belongs to the system. The tests look at the print job it was opened for and at
 * the copy of the document made for it, read here with Android's own PDF reader, as the print
 * service does, and then leave the dialog with Back.
 */
@RunWith(AndroidJUnit4::class)
class PrintTest : AppTest() {
    private val target get() = instrumentation.targetContext

    /**
     * Waits for a print job called [name] and returns the page count of the document made for it.
     */
    private fun pagesHandedOver(name: String): Int {
        val manager = target.getSystemService(PrintManager::class.java)
        compose.waitUntil(WAIT_MS) { manager.printJobs.any { it.info.label == name } }
        val copy = File(target.cacheDir, "printing/$name")
        assertTrue("the copy for the print service is there", copy.isFile)
        // Throws for a document that wants a password.
        return PdfRenderer(
            ParcelFileDescriptor.open(copy, ParcelFileDescriptor.MODE_READ_ONLY),
        ).use {
            it.pageCount
        }
    }

    /** Leaves the system's print dialog with Back, and waits for the app to be back. */
    private fun leavePrintDialog() {
        // Once the dialog is up, or Back would go to the app instead.
        compose.waitUntil(WAIT_MS) { !appFocused() }
        shell("input keyevent KEYCODE_BACK")
        compose.waitUntil(WAIT_MS) { appFocused() }
    }

    private fun appFocused(): Boolean {
        var focused = false
        scenario!!.onActivity { focused = it.hasWindowFocus() }
        return focused
    }

    @Test
    fun printingHandsOverTheDocumentAsEdited() {
        open("pages.pdf", shows = "1 / 12")
        if (!wide) compose.onNodeWithContentDescription("Contents and pages").performClick()
        waitForText("2")
        compose.onNodeWithText("2").performTouchInput { longClick() }
        compose.onNodeWithContentDescription("Delete pages").performClick()
        if (!wide) compose.onNodeWithContentDescription("Back to document").performClick()
        waitForText("1 / 11")
        menu("Print")
        assertEquals(11, pagesHandedOver("pages.pdf"))
        leavePrintDialog()
        // Printing saved nothing: the edits are still only here.
        compose.onNodeWithText("Edited").assertIsDisplayed()
        compose.onNodeWithText("1 / 11").assertIsDisplayed()
    }

    @Test
    fun aProtectedDocumentIsPrintedWithoutItsPassword() {
        open("locked.pdf", shows = "Password required")
        compose.onNodeWithText("Password").performTextInput("secret")
        compose.onNodeWithText("Open").performClick()
        waitForText("1 / 3")
        menu("Print")
        assertEquals(3, pagesHandedOver("locked.pdf"))
        leavePrintDialog()
        compose.onNodeWithText("1 / 3").assertIsDisplayed()
    }
}
