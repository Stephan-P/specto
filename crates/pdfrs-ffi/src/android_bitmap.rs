//! JNI side door for tile pixels: copies a finished tile straight into an
//! `android.graphics.Bitmap`.
//!
//! UniFFI can't pass a `Bitmap`, and returning pixels through it costs extra
//! copies (measured in M0). Kotlin hands us the session id, the tile id from
//! `ViewSession.takeFinished()` and a bitmap of the announced size.
#![allow(unsafe_code)]

use std::panic::{AssertUnwindSafe, catch_unwind};

use jni_sys::{JNIEnv, jclass, jint, jlong, jobject};
use ndk::bitmap::{Bitmap, BitmapFormat};

const OK: jint = 0;
const ERR_UNKNOWN_TILE: jint = -1;
const ERR_BITMAP: jint = -2;
const ERR_PANIC: jint = -4;

/// `NativeBitmap.copyTile(session: Long, tile: Long, bitmap: Bitmap): Int`
///
/// The bitmap must be mutable `ARGB_8888` with the tile's dimensions.
///
/// # Safety
///
/// Must only be called by the JVM as the native method above, which
/// guarantees that `env` and `bitmap` are valid for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_net_xfbs_pdfrs_NativeBitmap_copyTile(
    env: *mut JNIEnv,
    _class: jclass,
    session: jlong,
    tile: jlong,
    bitmap: jobject,
) -> jint {
    // SAFETY: `env` and `bitmap` are valid for this JNI call.
    let bitmap = unsafe { Bitmap::from_jni(env, bitmap) };
    // Unwinding across `extern "system"` would abort the app.
    catch_unwind(AssertUnwindSafe(|| {
        copy_tile(session as u64, tile as u64, &bitmap)
    }))
    .unwrap_or(ERR_PANIC)
}

fn copy_tile(session: u64, tile: u64, bitmap: &Bitmap) -> jint {
    let mut status = ERR_UNKNOWN_TILE;
    crate::claim_tile(session, tile, |pixels| {
        status = write(pixels, bitmap);
        status == OK
    });
    status
}

fn write(src: &pdfrs_render::Bitmap, bitmap: &Bitmap) -> jint {
    let Ok(info) = bitmap.info() else {
        return ERR_BITMAP;
    };
    if info.format() != BitmapFormat::RGBA_8888
        || (info.width(), info.height()) != (src.width, src.height)
    {
        return ERR_BITMAP;
    }
    let Ok(dst) = bitmap.lock_pixels() else {
        return ERR_BITMAP;
    };
    let row = src.width as usize * 4;
    let stride = info.stride() as usize;
    for (y, line) in src.data.chunks_exact(row).enumerate() {
        // SAFETY: while locked, the bitmap owns `stride * height` bytes at
        // `dst`, and `row <= stride`. The source is our own `Vec`.
        unsafe {
            std::ptr::copy_nonoverlapping(line.as_ptr(), dst.cast::<u8>().add(y * stride), row);
        }
    }
    match bitmap.unlock_pixels() {
        Ok(()) => OK,
        Err(_) => ERR_BITMAP,
    }
}
