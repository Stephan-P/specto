package net.xfbs.specto

import android.content.Context
import android.os.Bundle
import android.os.CancellationSignal
import android.os.ParcelFileDescriptor
import android.print.PageRange
import android.print.PrintAttributes
import android.print.PrintDocumentAdapter
import android.print.PrintDocumentInfo
import android.print.PrintManager
import java.io.File
import java.io.FileOutputStream
import kotlin.concurrent.thread
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** Where the copy handed to the print service lives, below the cache directory. */
private const val PRINTING_DIR = "printing"

/**
 * Opens the system's print dialog for the document open in [state], as it is now, unsaved edits
 * included. Must be called on an activity.
 *
 * The system prints PDFs itself: it gets the document's pages as they are (text stays text, nothing
 * is turned into images), shows the preview, and takes care of page ranges, copies and paper. It
 * can't be told a password, so a protected document is handed over without its protection, in a
 * copy that stays in the app's cache until the print job is done.
 *
 * Throws `PdfException.Save` if the copy can't be written.
 */
suspend fun Context.printDocument(state: PdfViewState, name: String) {
    val copy = withContext(Dispatchers.IO) {
        val dir = File(cacheDir, PRINTING_DIR)
        dir.deleteRecursively()
        dir.mkdirs()
        File(dir, sharedFileName(name))
    }
    state.printTo(copy.writableDescriptor())
    // Paper the way the first page lies. The reader can change it.
    val landscape = state.pageSizes.firstOrNull()?.let { it.width > it.height } == true
    val attributes = PrintAttributes.Builder()
        .setMediaSize(
            if (landscape) {
                PrintAttributes.MediaSize.UNKNOWN_LANDSCAPE
            } else {
                PrintAttributes.MediaSize.UNKNOWN_PORTRAIT
            },
        )
        .build()
    getSystemService(PrintManager::class.java)
        .print(name, PdfFilePrintAdapter(copy, name, state.pageCount), attributes)
}

/**
 * Gives the print service a finished PDF [file] of [pageCount] pages, and deletes it when the print
 * dialog is done with it.
 */
private class PdfFilePrintAdapter(
    private val file: File,
    private val name: String,
    private val pageCount: Int,
) : PrintDocumentAdapter() {
    override fun onLayout(
        oldAttributes: PrintAttributes?,
        newAttributes: PrintAttributes,
        cancellationSignal: CancellationSignal,
        callback: LayoutResultCallback,
        extras: Bundle?,
    ) {
        if (cancellationSignal.isCanceled) return callback.onLayoutCancelled()
        val info = PrintDocumentInfo.Builder(sharedFileName(name))
            .setContentType(PrintDocumentInfo.CONTENT_TYPE_DOCUMENT)
            .setPageCount(pageCount)
            .build()
        // The pages are what they are, whatever paper is chosen: the system fits them onto it.
        callback.onLayoutFinished(info, newAttributes != oldAttributes)
    }

    override fun onWrite(
        pages: Array<out PageRange>,
        destination: ParcelFileDescriptor,
        cancellationSignal: CancellationSignal,
        callback: WriteResultCallback,
    ) {
        // Called on the main thread, and the document may be large.
        thread(name = "print-copy") {
            try {
                file.inputStream().use { input ->
                    FileOutputStream(destination.fileDescriptor).use { input.copyTo(it) }
                }
                if (cancellationSignal.isCanceled) {
                    callback.onWriteCancelled()
                } else {
                    // All of them: the system picks out the pages it was asked for.
                    callback.onWriteFinished(arrayOf(PageRange.ALL_PAGES))
                }
            } catch (e: Exception) {
                callback.onWriteFailed(e.message)
            }
        }
    }

    override fun onFinish() {
        file.delete()
    }
}
