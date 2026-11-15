package net.xfbs.specto

import java.time.ZoneId
import java.util.Locale
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.specto_ffi.DocumentInfo
import uniffi.specto_ffi.PageSize

class PropertiesTest {
    @Test
    fun paperSizesAreNamed() {
        assertEquals("210 × 297 mm (A4)", pageSizeText(595f, 842f, EnglishTexts, Locale.UK))
        assertEquals("210 × 297 mm (A4)", pageSizeText(595.276f, 841.89f, EnglishTexts, Locale.UK))
        assertEquals("297 × 210 mm (A4, landscape)", pageSizeText(842f, 595f, EnglishTexts, Locale.UK))
        assertEquals("148 × 210 mm (A5)", pageSizeText(420f, 595f, EnglishTexts, Locale.UK))
        assertEquals("8.5 × 11 in (Letter)", pageSizeText(612f, 792f, EnglishTexts, Locale.UK))
        assertEquals("11 × 8.5 in (Letter, landscape)", pageSizeText(792f, 612f, EnglishTexts, Locale.UK))
        assertEquals("8.5 × 14 in (Legal)", pageSizeText(612f, 1008f, EnglishTexts, Locale.UK))
    }

    @Test
    fun inchesAreWrittenAsTheReaderWritesNumbers() {
        assertEquals("8,5 × 11 in (Letter)", pageSizeText(612f, 792f, EnglishTexts, Locale.GERMANY))
    }

    @Test
    fun otherSizesAreMeasured() {
        // 6 × 9 inches, a common book size without a name here.
        assertEquals("152 × 229 mm", pageSizeText(432f, 648f, EnglishTexts, Locale.UK))
        assertEquals("254 × 143 mm", pageSizeText(720f, 405f, EnglishTexts, Locale.UK))
    }

    @Test
    fun aDocumentsPageSizeIsItsFirstPages() {
        val a4 = PageSize(595f, 842f)
        assertEquals("210 × 297 mm (A4)", pageSizesText(listOf(a4, a4, PageSize(595.4f, 841.6f)), EnglishTexts, Locale.UK))
        assertEquals(
            "210 × 297 mm (A4); other pages differ",
            pageSizesText(listOf(a4, PageSize(842f, 595f)), EnglishTexts, Locale.UK),
        )
        assertEquals(null, pageSizesText(emptyList(), EnglishTexts, Locale.UK))
    }

    @Test
    fun datesAreShownOnTheReadersClock() {
        val shown = documentDate("2024-03-05T14:22:10+01:00", ZoneId.of("UTC"), Locale.UK)
        assertTrue(shown, "2024" in shown && "Mar" in shown && "13:22" in shown)
        val tokyo = documentDate("2024-03-05T23:30:00+00:00", ZoneId.of("Asia/Tokyo"), Locale.UK)
        assertTrue(tokyo, "6 Mar" in tokyo && "08:30" in tokyo)
    }

    @Test
    fun aDateThatIsNoneIsShownAsItIs() {
        assertEquals("2024-02-31T10:00:00+00:00", documentDate("2024-02-31T10:00:00+00:00"))
        assertEquals("yesterday", documentDate("yesterday"))
    }

    @Test
    fun onlyWhatIsKnownIsListed() {
        val nothing = DocumentInfo(null, null, null, null, null, null, null, null, "1.7")
        assertEquals(
            listOf("File name" to "scan.pdf", "Pages" to "3", "File size" to "1 MB", "PDF version" to "1.7"),
            propertyRows("scan.pdf", nothing, 3, null, "1 MB", EnglishTexts) { it },
        )
        val full = DocumentInfo(
            title = "Notes", author = "Ada", subject = "Engines", keywords = "a, b",
            creator = "ed", producer = "Specto", created = "C", modified = "M", pdfVersion = "2.0",
        )
        assertEquals(
            listOf(
                "File name", "Title", "Author", "Subject", "Keywords", "Created with", "PDF producer",
                "Created", "Modified", "Pages", "Page size", "File size", "PDF version",
            ),
            propertyRows("n.pdf", full, 1, "A4", "1 kB", EnglishTexts) { "date $it" }.map { it.first },
        )
        assertEquals("date C", propertyRows("n.pdf", full, 1, "A4", "1 kB", EnglishTexts) { "date $it" }.first { it.first == "Created" }.second)
    }
}
