//! JNI functions behind `net.xfbs.specto.Native`: the per-frame path and tile
//! pixels, which are too hot (or too big) for UniFFI. Each is a thin wrapper
//! around [`crate::hot`] or [`crate::claim_tile`]; sessions are looked up by
//! id, so a closed session is simply not found.
#![allow(unsafe_code)]

use std::panic::{AssertUnwindSafe, catch_unwind};

use jni_sys::{JNIEnv, jclass, jdouble, jint, jlong, jobject};
use ndk::bitmap::{Bitmap, BitmapFormat};

use crate::hot;

const OK: jint = 0;
const ERR_UNKNOWN_TILE: jint = -1;
const ERR_BITMAP: jint = -2;
const ERR_PANIC: jint = -4;

/// Runs `f`, turning a panic into `on_panic`: unwinding across
/// `extern "system"` would abort the app.
fn guarded<T>(on_panic: T, f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(on_panic)
}

/// `Native.setViewport(session: Long, width: Double, height: Double, scale: Double)`
#[unsafe(no_mangle)]
pub extern "system" fn Java_net_xfbs_specto_Native_setViewport(
    _env: *mut JNIEnv,
    _class: jclass,
    session: jlong,
    width: jdouble,
    height: jdouble,
    scale: jdouble,
) {
    guarded((), || {
        hot::set_viewport(session as u64, width, height, scale);
    });
}

/// `Native.scrollBy(session: Long, dx: Double, dy: Double)`
#[unsafe(no_mangle)]
pub extern "system" fn Java_net_xfbs_specto_Native_scrollBy(
    _env: *mut JNIEnv,
    _class: jclass,
    session: jlong,
    dx: jdouble,
    dy: jdouble,
) {
    guarded((), || {
        hot::scroll_by(session as u64, dx, dy);
    });
}

/// `Native.selectTo(session: Long, x: Double, y: Double)`
#[unsafe(no_mangle)]
pub extern "system" fn Java_net_xfbs_specto_Native_selectTo(
    _env: *mut JNIEnv,
    _class: jclass,
    session: jlong,
    x: jdouble,
    y: jdouble,
) {
    guarded((), || {
        hot::select_to(session as u64, x, y);
    });
}

/// `Native.scrollInPageBy(session: Long, dx: Double, dy: Double)`
#[unsafe(no_mangle)]
pub extern "system" fn Java_net_xfbs_specto_Native_scrollInPageBy(
    _env: *mut JNIEnv,
    _class: jclass,
    session: jlong,
    dx: jdouble,
    dy: jdouble,
) {
    guarded((), || {
        hot::scroll_in_page_by(session as u64, dx, dy);
    });
}

/// `Native.zoomBy(session: Long, factor: Double, x: Double, y: Double)`
#[unsafe(no_mangle)]
pub extern "system" fn Java_net_xfbs_specto_Native_zoomBy(
    _env: *mut JNIEnv,
    _class: jclass,
    session: jlong,
    factor: jdouble,
    x: jdouble,
    y: jdouble,
) {
    guarded((), || {
        hot::zoom_by(session as u64, factor, x, y);
    });
}

/// `Native.setThumbnailRange(session: Long, first: Int, count: Int)`
#[unsafe(no_mangle)]
pub extern "system" fn Java_net_xfbs_specto_Native_setThumbnailRange(
    _env: *mut JNIEnv,
    _class: jclass,
    session: jlong,
    first: jint,
    count: jint,
) {
    guarded((), || {
        hot::set_thumbnail_range(session as u64, first.max(0) as u32, count.max(0) as u32);
    });
}

/// The memory of a direct `java.nio.ByteBuffer`, or `None` if it isn't one.
///
/// # Safety
///
/// `env` and `buffer` must be valid for the current JNI call, and the
/// returned slice must not outlive it or be aliased by the caller's thread
/// (Kotlin only reads the buffer after the native method returns).
unsafe fn direct_buffer<'a>(env: *mut JNIEnv, buffer: jobject) -> Option<&'a mut [u8]> {
    // SAFETY: `env` is the valid JNI environment of this call, and its
    // function table is complete on Android (JNI 1.6).
    let (address, capacity) = unsafe {
        let functions = &**env;
        (
            (functions.GetDirectBufferAddress?)(env, buffer),
            (functions.GetDirectBufferCapacity?)(env, buffer),
        )
    };
    if address.is_null() || capacity <= 0 {
        return None;
    }
    // SAFETY: the JVM guarantees `capacity` bytes at `address` for a direct
    // buffer, which the caller keeps referenced during the call.
    Some(unsafe { std::slice::from_raw_parts_mut(address.cast::<u8>(), capacity as usize) })
}

/// `Native.frame(session: Long, out: ByteBuffer): Int`: see [`hot::frame`].
///
/// # Safety
///
/// Must only be called by the JVM as the native method above.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_net_xfbs_specto_Native_frame(
    env: *mut JNIEnv,
    _class: jclass,
    session: jlong,
    out: jobject,
) -> jint {
    // SAFETY: called by the JVM with a valid `env` and `out`.
    let Some(out) = (unsafe { direct_buffer(env, out) }) else {
        return 0;
    };
    guarded(0, || hot::frame(session as u64, out) as jint)
}

/// `Native.lastFrame(session: Long, out: ByteBuffer): Int`: see [`hot::last_frame`].
///
/// # Safety
///
/// Must only be called by the JVM as the native method above.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_net_xfbs_specto_Native_lastFrame(
    env: *mut JNIEnv,
    _class: jclass,
    session: jlong,
    out: jobject,
) -> jint {
    // SAFETY: called by the JVM with a valid `env` and `out`.
    let Some(out) = (unsafe { direct_buffer(env, out) }) else {
        return 0;
    };
    guarded(0, || hot::last_frame(session as u64, out) as jint)
}

/// `Native.takeFinished(session: Long, out: ByteBuffer): Int`: see [`hot::take_finished`].
///
/// # Safety
///
/// Must only be called by the JVM as the native method above.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_net_xfbs_specto_Native_takeFinished(
    env: *mut JNIEnv,
    _class: jclass,
    session: jlong,
    out: jobject,
) -> jint {
    // SAFETY: called by the JVM with a valid `env` and `out`.
    let Some(out) = (unsafe { direct_buffer(env, out) }) else {
        return 0;
    };
    guarded(0, || hot::take_finished(session as u64, out) as jint)
}

/// `Native.copyTile(session: Long, tile: Long, bitmap: Bitmap): Int`
///
/// Copies a finished tile into `bitmap`, which must be mutable `ARGB_8888`
/// with the tile's dimensions. UniFFI can't pass a `Bitmap`, and returning
/// pixels through it costs extra copies (measured in M0).
///
/// # Safety
///
/// Must only be called by the JVM as the native method above, which
/// guarantees that `env` and `bitmap` are valid for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_net_xfbs_specto_Native_copyTile(
    env: *mut JNIEnv,
    _class: jclass,
    session: jlong,
    tile: jlong,
    bitmap: jobject,
) -> jint {
    // SAFETY: `env` and `bitmap` are valid for this JNI call.
    let bitmap = unsafe { Bitmap::from_jni(env, bitmap) };
    guarded(ERR_PANIC, || {
        let mut status = ERR_UNKNOWN_TILE;
        crate::claim_tile(session as u64, tile as u64, |pixels| {
            status = write(pixels, &bitmap);
            status == OK
        });
        status
    })
}

fn write(src: &specto_render::Bitmap, bitmap: &Bitmap) -> jint {
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
