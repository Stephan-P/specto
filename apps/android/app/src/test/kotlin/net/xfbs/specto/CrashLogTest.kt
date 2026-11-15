package net.xfbs.specto

import java.time.Instant
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class CrashLogTest {
    private val noon = Instant.parse("2026-11-15T12:00:00.250Z")

    @Test
    fun anEntrySaysWhenWhereAndWhat() {
        val entry = crashEntry(noon, "main", IllegalStateException("boom", RuntimeException("the cause")))
        val lines = entry.lines()
        assertEquals("--- 2026-11-15T12:00:00Z Crash", lines[0])
        assertEquals("thread 'main': java.lang.IllegalStateException: boom", lines[1])
        assertTrue(entry, lines[2].trim().startsWith("at net.xfbs.specto.CrashLogTest"))
        assertTrue(entry, "Caused by: java.lang.RuntimeException: the cause" in entry)
        assertTrue("ends with an empty line", entry.endsWith("\n\n"))
    }

    @Test
    fun theNewestEntryDatesTheLog() {
        assertEquals(null, lastCrashTime(""))
        assertEquals(null, lastCrashTime("something else\n--- not a date\n"))
        val log = crashEntry(noon, "main", Exception("first")) +
            // As the Rust library writes one.
            "--- 2026-11-16T08:30:00Z Rust panic\nthread 'render': oops\nat src/lib.rs:1:2\n\n"
        assertEquals(Instant.parse("2026-11-16T08:30:00Z"), lastCrashTime(log))
    }

    @Test
    fun aLongLogKeepsItsNewestEntriesWhole() {
        val entries = (1..5).map { "--- 2026-11-1${it}T00:00:00Z Crash\nthread 'main': number $it\n\n" }
        val log = entries.joinToString("")
        val one = entries[0].length
        assertEquals(log, trimmedLog(log, log.length))
        assertEquals(entries.takeLast(2).joinToString(""), trimmedLog(log, 2 * one))
        // Not a part of the third one.
        assertEquals(entries.takeLast(2).joinToString(""), trimmedLog(log, 2 * one + 10))
        // One entry longer than everything allowed: its end.
        assertEquals(log.takeLast(10), trimmedLog(log, 10))
    }
}
