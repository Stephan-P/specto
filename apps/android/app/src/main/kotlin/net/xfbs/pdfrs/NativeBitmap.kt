package net.xfbs.pdfrs

import android.graphics.Bitmap

/**
 * JNI side door into libpdfrs_ffi for tile pixels (see
 * crates/pdfrs-ffi/src/android_bitmap.rs). Everything else goes through the
 * generated UniFFI bindings.
 */
object NativeBitmap {
    init {
        System.loadLibrary("pdfrs_ffi")
    }

    /**
     * Copies finished tile [tile] of the session with id [session] into
     * [bitmap], which must be a mutable ARGB_8888 bitmap of the size announced
     * by `ViewSession.takeFinished()`. Returns 0 on success.
     */
    @JvmStatic
    external fun copyTile(session: Long, tile: Long, bitmap: Bitmap): Int
}
