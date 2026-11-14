package net.xfbs.pdfrs

import java.nio.ByteBuffer
import java.nio.ByteOrder
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Decodes `crates/pdfrs-ffi/fixtures/frame.bin`: the bytes Rust writes for a
 * known frame (`fixture_frame` in the crate's tests, which also checks that
 * the file is current). If the layout changes on the Rust side, that test
 * fails first; this one fails if the Kotlin decoder is not updated to match.
 */
class ViewFrameTest {
    private val frame: ViewFrame = run {
        val bytes = checkNotNull(javaClass.getResourceAsStream("/frame.bin")) { "fixture missing" }.readBytes()
        ViewFrame.decode(ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN), bytes.size)
    }

    @Test
    fun header() {
        assertFalse(frame.complete)
        assertTrue(frame.settled)
        assertTrue(frame.paged)
        assertTrue(frame.twoPages)
        assertFalse(frame.coverAlone)
        assertEquals(4, frame.currentPage)
        assertEquals(5, frame.lastPage)
        assertEquals(180, frame.settlesInMs)
        assertEquals(0.6645856618881226, frame.zoom, 0.0)
        assertEquals(1.744537353515625, frame.deviceScale, 0.0)
        assertEquals(1.5, frame.zoomFactor, 0.0)
        assertArrayEquals(longArrayOf(3, 9_000_000_000), frame.evicted)
    }

    @Test
    fun pagesAndTiles() {
        assertEquals(2, frame.pageCount)

        assertEquals(4, frame.page(0))
        assertEquals(21f, frame.pageLeft(0))
        assertEquals(-300f, frame.pageTop(0))
        assertEquals(1038f, frame.pageWidth(0))
        assertEquals(1469f, frame.pageHeight(0))
        assertEquals(0..1, frame.tiles(0))

        assertEquals(7L, frame.tileId(0))
        assertTrue(frame.tileIsExact(0))
        assertEquals(21f, frame.tileLeft(0))
        assertEquals(-300f, frame.tileTop(0))
        assertEquals(533f, frame.tileRight(0))
        assertEquals(212f, frame.tileBottom(0))

        // An id beyond 32 bits, and a fractional destination.
        assertEquals(1L shl 40, frame.tileId(1))
        assertFalse(frame.tileIsExact(1))
        assertEquals(533f, frame.tileLeft(1))
        assertEquals(996.5f, frame.tileRight(1))
        assertEquals(163.25f, frame.tileBottom(1))

        // A page without tiles yet.
        assertEquals(5, frame.page(1))
        assertEquals(1190f, frame.pageTop(1))
        assertTrue(frame.tiles(1).isEmpty())
    }

    @Test
    fun thumbnails() {
        // Announced for pages 2–4; page 2 is not rendered yet.
        assertEquals(0L, frame.thumbnailId(2))
        assertEquals(11L, frame.thumbnailId(3))
        assertEquals(12L, frame.thumbnailId(4))
        // Outside the announced range.
        assertEquals(0L, frame.thumbnailId(1))
        assertEquals(0L, frame.thumbnailId(5))
        assertTrue(frame.sameThumbnailsAs(frame))
        assertFalse(frame.sameThumbnailsAs(null))
    }
}
