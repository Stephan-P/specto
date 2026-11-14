package net.xfbs.pdfrs

import android.graphics.Bitmap

/**
 * JNI side door into libpdfrs_ffi for the hot path (see
 * crates/pdfrs-ffi/src/android_bitmap.rs). Everything else goes through the
 * generated UniFFI bindings.
 */
object NativeBitmap {
    init {
        System.loadLibrary("pdfrs_ffi")
    }

    /**
     * Renders [page] at [scale] into [bitmap], which must be a mutable ARGB_8888
     * bitmap sized by `PdfDocument.pixelSize(page, scale)`. [doc] is
     * `PdfDocument.rawHandle()`; the document must stay open during the call.
     * Returns 0 on success.
     */
    @JvmStatic
    external fun renderInto(doc: Long, page: Int, scale: Float, bitmap: Bitmap): Int
}
