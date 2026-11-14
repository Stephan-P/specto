package net.xfbs.pdfrs

import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import uniffi.pdfrs_ffi.SavedView

class DocumentStoreTest {
    @get:Rule
    val folder = TemporaryFolder()

    private val file: File by lazy { File(folder.root, "documents.json") }
    private val view = SavedView(
        page = 3u, pageOffset = 0.25, paged = false, zoom = 1.5, twoPages = true, coverAlone = true,
    )

    @Test
    fun remembersAcrossRestarts() {
        DocumentStore(file).apply {
            opened("k1", "A book.pdf", "content://docs/1", pageCount = 240)
            saveView("k1", view)
        }
        val entry = checkNotNull(DocumentStore(file).find("k1"))
        assertEquals("A book.pdf", entry.name)
        assertEquals("content://docs/1", entry.uri)
        assertEquals(240, entry.pageCount)
        assertEquals(view, entry.view)
    }

    @Test
    fun fittedViewHasNoZoom() {
        val fitted = view.copy(zoom = null, paged = true, twoPages = false, coverAlone = false)
        DocumentStore(file).apply {
            opened("k1", "a.pdf", null, 3)
            saveView("k1", fitted)
        }
        assertEquals(fitted, DocumentStore(file).find("k1")?.view)
    }

    @Test
    fun recentsAreTheReopenableOnesNewestFirst() {
        val store = DocumentStore(file)
        store.opened("k1", "one.pdf", "content://docs/1", 1)
        store.opened("k2", "from another app.pdf", null, 1)
        store.opened("k3", "three.pdf", "content://docs/3", 1)
        assertEquals(listOf("k3", "k1"), store.recents.map { it.key })
        assertEquals(setOf("content://docs/1", "content://docs/3"), store.reopenable())

        // Opening one again moves it to the front.
        store.opened("k1", "one.pdf", "content://docs/1", 1)
        assertEquals(listOf("k1", "k3"), store.recents.map { it.key })
    }

    @Test
    fun reopeningKeepsThePositionAndAKnownAddress() {
        val store = DocumentStore(file)
        store.opened("k1", "one.pdf", "content://docs/1", 10)
        store.saveView("k1", view)
        // The same file, this time handed over by an app whose address can't be kept.
        store.opened("k1", "one (copy).pdf", null, 10)
        val entry = checkNotNull(store.find("k1"))
        assertEquals("one (copy).pdf", entry.name)
        assertEquals("content://docs/1", entry.uri)
        assertEquals(view, entry.view)
    }

    @Test
    fun anUnreachableFileLeavesTheListButKeepsItsPosition() {
        val store = DocumentStore(file)
        store.opened("k1", "one.pdf", "content://docs/1", 10)
        store.saveView("k1", view)
        store.unreachable("content://docs/1")
        assertTrue(store.recents.isEmpty())
        assertEquals(view, DocumentStore(file).find("k1")?.view)
    }

    @Test
    fun savingAViewForAnUnknownDocumentDoesNothing() {
        val store = DocumentStore(file)
        store.saveView("nope", view)
        assertNull(store.find("nope"))
    }

    @Test
    fun oldDocumentsAreForgotten() {
        val store = DocumentStore(file)
        for (i in 1..205) store.opened("k$i", "$i.pdf", "content://docs/$i", 1)
        assertNull(store.find("k1"))
        assertEquals("205.pdf", store.find("k205")?.name)
        // The start screen shows fewer than are remembered.
        assertEquals(20, store.recents.size)
        assertEquals("k205", store.recents.first().key)
    }

    @Test
    fun anUnreadableFileStartsAfresh() {
        file.writeText("{ not json")
        val store = DocumentStore(file)
        assertTrue(store.recents.isEmpty())
        store.opened("k1", "one.pdf", "content://docs/1", 1)
        assertEquals("one.pdf", DocumentStore(file).find("k1")?.name)
    }

    @Test
    fun fingerprintFollowsContentNotOrigin() {
        val a = ByteArray(300_000) { (it % 251).toByte() }
        assertEquals(DocumentStore.fingerprint(a), DocumentStore.fingerprint(a.copyOf()))
        // Different at the start, at the end, or in length.
        assertNotEquals(DocumentStore.fingerprint(a), DocumentStore.fingerprint(a.copyOf().also { it[0] = 9 }))
        assertNotEquals(DocumentStore.fingerprint(a), DocumentStore.fingerprint(a.copyOf().also { it[it.lastIndex] = 9 }))
        assertNotEquals(DocumentStore.fingerprint(a), DocumentStore.fingerprint(a.copyOf(a.size + 1)))
        // Small files work too.
        assertNotEquals(DocumentStore.fingerprint(byteArrayOf(1, 2)), DocumentStore.fingerprint(byteArrayOf(1, 3)))
        assertEquals(32, DocumentStore.fingerprint(ByteArray(0)).length)
    }
}
