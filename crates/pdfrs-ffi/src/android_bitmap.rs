//! JNI side door for the hot path: render a page straight into an
//! `android.graphics.Bitmap`.
//!
//! UniFFI can't pass a `Bitmap`, and returning pixels through it costs extra
//! copies (`Vec` → `RustBuffer` → `ByteArray` → `Bitmap`). Here Kotlin hands
//! us the `Bitmap` plus [`PdfDocument::raw_handle`], and we copy the rendered
//! pixels once into the locked bitmap memory.
#![allow(unsafe_code)]

use std::panic::{AssertUnwindSafe, catch_unwind};

use jni_sys::{JNIEnv, jclass, jfloat, jint, jlong, jobject};
use ndk::bitmap::{Bitmap, BitmapFormat};

use crate::PdfDocument;

const OK: jint = 0;
const ERR_RENDER: jint = -1;
const ERR_BITMAP: jint = -2;
const ERR_SIZE_MISMATCH: jint = -3;
const ERR_PANIC: jint = -4;

/// `NativeBitmap.renderInto(doc: Long, page: Int, scale: Float, bitmap: Bitmap): Int`
///
/// The bitmap must be mutable `ARGB_8888` with exactly the dimensions from
/// `PdfDocument.pixelSize(page, scale)`.
///
/// # Safety
///
/// `doc` must come from `PdfDocument.rawHandle()` of an object the caller
/// keeps alive (not yet `close()`d) for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_net_xfbs_pdfrs_NativeBitmap_renderInto(
    env: *mut JNIEnv,
    _class: jclass,
    doc: jlong,
    page: jint,
    scale: jfloat,
    bitmap: jobject,
) -> jint {
    // SAFETY: see the function's safety contract.
    let doc = unsafe { &*(doc as *const PdfDocument) };
    // SAFETY: `env` and `bitmap` are valid for this JNI call.
    let bitmap = unsafe { Bitmap::from_jni(env, bitmap) };
    // Unwinding across `extern "system"` would abort the app.
    catch_unwind(AssertUnwindSafe(|| render_into(doc, page, scale, &bitmap))).unwrap_or(ERR_PANIC)
}

fn render_into(doc: &PdfDocument, page: jint, scale: f32, bitmap: &Bitmap) -> jint {
    let Ok(page) = u32::try_from(page) else {
        return ERR_RENDER;
    };
    let Ok(rendered) = doc.render(page, scale) else {
        return ERR_RENDER;
    };
    let Ok(info) = bitmap.info() else {
        return ERR_BITMAP;
    };
    if info.format() != BitmapFormat::RGBA_8888 {
        return ERR_BITMAP;
    }
    if (info.width(), info.height()) != (rendered.width, rendered.height) {
        return ERR_SIZE_MISMATCH;
    }
    let Ok(pixels) = bitmap.lock_pixels() else {
        return ERR_BITMAP;
    };

    let row = rendered.width as usize * 4;
    let stride = info.stride() as usize;
    for (y, src) in rendered.data.chunks_exact(row).enumerate() {
        // SAFETY: while locked, the bitmap owns `stride * height` bytes at
        // `pixels`, and `row <= stride`. The source is our own `Vec`.
        unsafe {
            std::ptr::copy_nonoverlapping(src.as_ptr(), pixels.cast::<u8>().add(y * stride), row);
        }
    }

    match bitmap.unlock_pixels() {
        Ok(()) => OK,
        Err(_) => ERR_BITMAP,
    }
}
