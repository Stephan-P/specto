package net.xfbs.pdfrs

import android.graphics.Bitmap
import java.nio.ByteBuffer

/**
 * JNI functions of libpdfrs_ffi (crates/pdfrs-ffi/src/jni.rs) for what runs
 * every frame and for tile pixels. A call through the generated UniFFI
 * bindings costs a few hundred microseconds, which is fine for opening a file
 * or changing a mode but not for something done several times per frame.
 *
 * [session] is `ViewSession.id()`. Calls for a closed session do nothing.
 */
object Native {
    init {
        System.loadLibrary("pdfrs_ffi")
    }

    /** Scrolls by dp. */
    @JvmStatic
    external fun scrollBy(session: Long, dx: Double, dy: Double)

    /**
     * Scrolls by dp like a fling: in paged mode it stops at the edges of the
     * current page instead of sliding on to the next.
     */
    @JvmStatic
    external fun scrollInPageBy(session: Long, dx: Double, dy: Double)

    /** Multiplies the zoom, keeping the content at ([x], [y]) dp in place. */
    @JvmStatic
    external fun zoomBy(session: Long, factor: Double, x: Double, y: Double)

    /** The pages whose thumbnails are on screen; see [ViewFrame.thumbnailId]. */
    @JvmStatic
    external fun setThumbnailRange(session: Long, first: Int, count: Int)

    /**
     * Plans a frame and writes it into [out] (a direct buffer; see
     * [ViewFrame.decode] for the layout). Returns the bytes written, or minus
     * the size needed if [out] is too small; then call [lastFrame].
     */
    @JvmStatic
    external fun frame(session: Long, out: ByteBuffer): Int

    /** Writes the frame last planned by [frame] again. */
    @JvmStatic
    external fun lastFrame(session: Long, out: ByteBuffer): Int

    /**
     * Writes the tiles that finished rendering into [out]: a count, four
     * unused bytes, then (id: Long, width: Int, height: Int) each.
     */
    @JvmStatic
    external fun takeFinished(session: Long, out: ByteBuffer): Int

    /**
     * Copies finished tile [tile] into [bitmap], which must be a mutable
     * ARGB_8888 bitmap of the announced size. Returns 0 on success.
     */
    @JvmStatic
    external fun copyTile(session: Long, tile: Long, bitmap: Bitmap): Int
}
