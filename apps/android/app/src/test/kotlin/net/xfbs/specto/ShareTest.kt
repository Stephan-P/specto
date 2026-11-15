package net.xfbs.specto

import org.junit.Assert.assertEquals
import org.junit.Test

class ShareTest {
    @Test
    fun aFileNameIsKept() {
        assertEquals("Annual report 2025.pdf", sharedFileName("Annual report 2025.pdf"))
        assertEquals("Bericht für Jürgen.PDF", sharedFileName("Bericht für Jürgen.PDF"))
    }

    @Test
    fun aNameWithoutTheEndingGetsIt() {
        assertEquals("scan.pdf", sharedFileName("scan"))
        assertEquals("1234.pdf", sharedFileName("1234"))
    }

    @Test
    fun aNameCannotLeaveItsDirectory() {
        assertEquals("_etc_passwd.pdf", sharedFileName("/etc/passwd"))
        assertEquals("_.._secret.pdf", sharedFileName("../../secret.pdf"))
        assertEquals("a_b.pdf", sharedFileName("a\\b"))
        assertEquals("a_b.pdf", sharedFileName("a\nb"))
    }

    @Test
    fun anEmptyNameGetsOne() {
        assertEquals("document.pdf", sharedFileName(""))
        assertEquals("document.pdf", sharedFileName(" .. "))
    }

    @Test
    fun aVeryLongNameIsShortenedFromTheFront() {
        val name = sharedFileName("x".repeat(500) + "-final.pdf")
        assertEquals(120, name.length)
        assertEquals(true, name.endsWith("-final.pdf"))
    }
}
