package net.xfbs.pdfrs

import android.os.ParcelFileDescriptor
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import java.io.File
import java.io.RandomAccessFile
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.pdfrs_ffi.PdfFile

/**
 * Files too large to want in memory: Rust maps them (from 64 MB) instead of
 * reading them, and the app hands it the file itself, not its bytes.
 */
@RunWith(AndroidJUnit4::class)
class LargeFileTest : AppTest() {
    private val large: File get() = File(instrumentation.targetContext.cacheDir, "large.pdf")

    /**
     * A one-page PDF of a little over 64 MB. Nearly all of it is one unused
     * stream, written as a hole in the file, so it takes next to no space
     * or time and reads as zeros.
     */
    @Before
    fun writeLargeFile() {
        val padding = 64L shl 20
        RandomAccessFile(large, "rw").use { out ->
            out.setLength(0)
            val offsets = LongArray(5)
            fun put(text: String) = out.write(text.toByteArray(Charsets.US_ASCII))
            fun begin(number: Int, body: String) {
                offsets[number] = out.filePointer
                put("$number 0 obj\n$body")
            }
            put("%PDF-1.7\n")
            begin(1, "<< /Type /Catalog /Pages 2 0 R >>\nendobj\n")
            begin(2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n")
            begin(3, "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] >>\nendobj\n")
            begin(4, "<< /Length $padding >>\nstream\n")
            out.seek(out.filePointer + padding)
            put("\nendstream\nendobj\n")
            val xref = out.filePointer
            put("xref\n0 5\n0000000000 65535 f \n")
            for (number in 1..4) put("%010d 00000 n \n".format(offsets[number]))
            put("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n$xref\n%%EOF\n")
        }
    }

    @After
    fun removeLargeFile() {
        large.delete()
    }

    @Test
    fun aLargeFileIsMappedRatherThanRead() {
        val descriptor = ParcelFileDescriptor.open(large, ParcelFileDescriptor.MODE_READ_ONLY)
        PdfFile.fromFd(descriptor.detachFd()).use { file ->
            assertTrue(file.isMapped())
            assertEquals(large.length(), file.size().toLong())
            assertEquals("%PDF-1.7", String(file.read(0u, 8u)))
            assertEquals("%%EOF\n", String(file.read(file.size() - 6u, 100u)))
        }
    }

    @Test
    fun aSmallFileIsRead() {
        val small = File(instrumentation.targetContext.cacheDir, "small.pdf")
        instrumentation.context.assets.open("pages.pdf").use { input ->
            small.outputStream().use { input.copyTo(it) }
        }
        val descriptor = ParcelFileDescriptor.open(small, ParcelFileDescriptor.MODE_READ_ONLY)
        PdfFile.fromFd(descriptor.detachFd()).use { file ->
            assertEquals(false, file.isMapped())
            assertEquals(small.length(), file.size().toLong())
        }
        small.delete()
    }

    @Test
    fun theAppOpensALargeFile() {
        reopen("large.pdf")
        waitForText("large.pdf")
        // One blank page; its details come from the file all the same.
        menu("Properties")
        waitForText("210 × 297 mm (A4)")
        compose.onNodeWithText("File size").assertIsDisplayed()
        compose.onNodeWithText("Close").performClick()
        // Leaving and coming back finds it again (it is recognised by parts
        // of its content, without reading all of it).
        menu("Close")
        waitForText("Recent")
        compose.onNodeWithText("large.pdf").performClick()
        waitUntilGone("Recent")
        compose.onNodeWithText("large.pdf").assertIsDisplayed()
    }
}
