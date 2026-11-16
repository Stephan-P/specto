package net.xfbs.specto

import android.content.ClipData
import android.content.ContentResolver
import android.content.Context
import android.content.Intent
import android.net.Uri
import androidx.core.content.FileProvider
import java.io.File
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/**
 * Where copies made for sharing live, below the cache directory (see `res/xml/shared_paths.xml`).
 */
private const val SHARED_DIR = "shared"

/**
 * Offers the open document to other apps through the system's share sheet.
 *
 * The file is shared as it was opened (a protected one stays protected). If possible the receiving
 * app reads it where it is: the app passes its own permission to read [document] on. Where that
 * isn't allowed, or for a plain file path, which may not be given to other apps at all, it gets a
 * copy from the app's cache.
 *
 * Throws if the document can't be read any more.
 */
suspend fun Context.shareDocument(document: Uri, name: String) {
    if (document.scheme == ContentResolver.SCHEME_CONTENT) {
        try {
            startActivity(shareSheet(document, name))
            return
        } catch (e: SecurityException) {
            // Readable for this app, but not to be passed on: share a copy.
        }
    }
    val copy = withContext(Dispatchers.IO) { copyForSharing(document, name) }
    startActivity(shareSheet(FileProvider.getUriForFile(this, "$packageName.shared", copy), name))
}

/**
 * Offers the document open in [state] as it is now, unsaved edits included: a copy written for the
 * purpose, from the app's cache.
 *
 * Throws if it can't be written.
 */
suspend fun Context.shareEdited(state: PdfViewState, name: String) {
    val copy = withContext(Dispatchers.IO) {
        val dir = File(cacheDir, SHARED_DIR)
        dir.deleteRecursively()
        dir.mkdirs()
        File(dir, sharedFileName(name))
    }
    state.writeTo(copy.writableDescriptor(), countsAsSaved = false)
    startActivity(shareSheet(FileProvider.getUriForFile(this, "$packageName.shared", copy), name))
}

private fun shareSheet(document: Uri, name: String): Intent {
    val send = Intent(Intent.ACTION_SEND)
        .setType("application/pdf")
        .putExtra(Intent.EXTRA_STREAM, document)
        // Shown by the share sheet above the list of apps.
        .putExtra(Intent.EXTRA_TITLE, name)
        .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
    // The permission to read travels with the clip data.
    send.clipData = ClipData(name, arrayOf("application/pdf"), ClipData.Item(document))
    return Intent.createChooser(send, null)
}

/**
 * Offers [text], selected in a document, to other apps through the system's share sheet.
 *
 * Throws a `RuntimeException` if it is more than the system passes on.
 */
fun Context.shareText(text: String) {
    val send = Intent(Intent.ACTION_SEND)
        .setType("text/plain")
        .putExtra(Intent.EXTRA_TEXT, text)
    startActivity(Intent.createChooser(send, null))
}

/** Copies [document] into the cache under its own name, replacing the copy shared before. */
private fun Context.copyForSharing(document: Uri, name: String): File {
    val dir = File(cacheDir, SHARED_DIR)
    dir.deleteRecursively()
    dir.mkdirs()
    val copy = File(dir, sharedFileName(name))
    checkNotNull(contentResolver.openInputStream(document)) {
        "cannot open $document"
    }.use { input ->
        copy.outputStream().use { input.copyTo(it) }
    }
    return copy
}

/**
 * The file name a shared copy of the document called [name] gets: what the receiving app will show
 * and save it under. Never empty, never a path, and always ending in ".pdf".
 */
internal fun sharedFileName(name: String): String {
    val cleaned = name
        .map { if (it == '/' || it == '\\' || it.isISOControl()) '_' else it }
        .joinToString("")
        .trim()
        .trim('.')
        .takeLast(120)
        .ifEmpty { "document" }
    return if (cleaned.endsWith(".pdf", ignoreCase = true)) cleaned else "$cleaned.pdf"
}
