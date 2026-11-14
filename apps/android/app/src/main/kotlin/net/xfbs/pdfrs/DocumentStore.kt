package net.xfbs.pdfrs

import android.util.Log
import androidx.compose.runtime.mutableStateListOf
import java.io.File
import java.security.MessageDigest
import org.json.JSONArray
import org.json.JSONObject
import uniffi.pdfrs_ffi.SavedView

/** Documents remembered beyond this many are forgotten, oldest first. */
private const val MAX_DOCUMENTS = 200

/** How many documents the start screen lists. */
private const val MAX_RECENTS = 20

/** Bytes hashed at each end of a file to recognise it. */
private const val FINGERPRINT_SPAN = 64 * 1024

/**
 * What the app remembers about documents between sessions: where the reader
 * was in each, and which ones can be reopened from the start screen.
 *
 * A document is recognised by its content ([fingerprint]), not by where it
 * came from: the same file arrives under different addresses depending on
 * the app that hands it over. Kept in a small JSON [file] (in the app's
 * private storage). Main thread only.
 */
class DocumentStore(private val file: File) {
    class Entry(
        val key: String,
        val name: String,
        /** Where to reopen it from, if the app may still read that address. */
        val uri: String?,
        /** When it was last opened, in milliseconds since the epoch. */
        val opened: Long,
        val pageCount: Int,
        val view: SavedView?,
    )

    /** Most recently opened first. Observable, so the start screen follows. */
    private val entries = mutableStateListOf<Entry>()

    init {
        try {
            if (file.exists()) {
                val array = JSONObject(file.readText()).getJSONArray("documents")
                for (i in 0 until array.length()) entries += array.getJSONObject(i).toEntry()
            }
        } catch (e: Exception) {
            // Unreadable (e.g. written by a newer version): start afresh.
            Log.w("pdfrs", "ignoring $file", e)
            entries.clear()
        }
    }

    /** Documents that can be reopened, most recent first. */
    val recents: List<Entry> get() = entries.filter { it.uri != null }.take(MAX_RECENTS)

    fun find(key: String): Entry? = entries.firstOrNull { it.key == key }

    /**
     * Notes that [key] was just opened. [uri] is where to reopen it from, or
     * null if that won't be possible; an address known from before is kept.
     */
    fun opened(key: String, name: String, uri: String?, pageCount: Int) {
        val known = find(key)
        entries.removeAll { it.key == key }
        entries.add(0, Entry(key, name, uri ?: known?.uri, System.currentTimeMillis(), pageCount, known?.view))
        while (entries.size > MAX_DOCUMENTS) entries.removeAt(entries.lastIndex)
        save()
    }

    /** Remembers where the reader is in [key]. */
    fun saveView(key: String, view: SavedView) {
        val i = entries.indexOfFirst { it.key == key }
        if (i < 0 || entries[i].view == view) return
        val e = entries[i]
        entries[i] = Entry(e.key, e.name, e.uri, e.opened, e.pageCount, view)
        save()
    }

    /** [uri] can't be opened any more: stop offering it. The reading position stays. */
    fun unreachable(uri: String) {
        var changed = false
        for (i in entries.indices) {
            val e = entries[i]
            if (e.uri == uri) {
                entries[i] = Entry(e.key, e.name, null, e.opened, e.pageCount, e.view)
                changed = true
            }
        }
        if (changed) save()
    }

    /** Addresses the start screen may still offer; the rest need no read permission any more. */
    fun reopenable(): Set<String> = recents.mapNotNull { it.uri }.toSet()

    private fun save() {
        val array = JSONArray()
        for (e in entries) array.put(e.toJson())
        try {
            // Via a temporary file, so a crash can't leave half a file behind.
            val temporary = File(file.path + ".new")
            temporary.writeText(JSONObject().put("documents", array).toString())
            if (!temporary.renameTo(file)) Log.w("pdfrs", "could not replace $file")
        } catch (e: Exception) {
            Log.w("pdfrs", "could not write $file", e)
        }
    }

    companion object {
        /**
         * Identifies a file by its length and both ends: quick even for huge
         * files, and stable however the file reaches the app.
         */
        fun fingerprint(pdf: ByteArray): String {
            val digest = MessageDigest.getInstance("SHA-256")
            digest.update(pdf.size.toString().toByteArray())
            digest.update(pdf, 0, minOf(FINGERPRINT_SPAN, pdf.size))
            if (pdf.size > FINGERPRINT_SPAN) digest.update(pdf, pdf.size - FINGERPRINT_SPAN, FINGERPRINT_SPAN)
            return digest.digest().take(16).joinToString("") { "%02x".format(it) }
        }
    }
}

private fun DocumentStore.Entry.toJson(): JSONObject {
    val json = JSONObject()
        .put("key", key)
        .put("name", name)
        .put("opened", opened)
        .put("pages", pageCount)
    if (uri != null) json.put("uri", uri)
    view?.let {
        json.put("page", it.page.toInt()).put("offset", it.pageOffset).put("paged", it.paged)
        it.zoom?.let { zoom -> json.put("zoom", zoom) }
        if (it.twoPages) json.put("two", true)
        if (it.coverAlone) json.put("cover", true)
    }
    return json
}

private fun JSONObject.toEntry() = DocumentStore.Entry(
    key = getString("key"),
    name = getString("name"),
    uri = if (has("uri")) getString("uri") else null,
    opened = getLong("opened"),
    pageCount = optInt("pages", 0),
    view = if (has("page")) {
        SavedView(
            page = getInt("page").coerceAtLeast(0).toUInt(),
            pageOffset = optDouble("offset", 0.0),
            paged = optBoolean("paged", true),
            zoom = if (has("zoom")) getDouble("zoom") else null,
            twoPages = optBoolean("two", false),
            coverAlone = optBoolean("cover", false),
        )
    } else {
        null
    },
)
