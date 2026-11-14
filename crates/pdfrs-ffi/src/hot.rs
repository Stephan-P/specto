//! The per-frame path, without UniFFI: the same operations as on
//! [`ViewSession`](crate::ViewSession), addressed by session id and with
//! results written into a caller-provided byte buffer. `jni.rs` exposes these
//! to Kotlin, which reads the buffer with a `ByteBuffer` in native byte order.
//!
//! Frame layout (see [`encode_frame`]):
//!
//! ```text
//! u32 flags            bit 0: complete, bit 1: settled, bit 2: paged
//! u32 zoom_mode        0 fixed, 1 fit width, 2 fit page
//! u32 current_page
//! i32 settles_in_ms    -1: none
//! f64 zoom
//! f64 device_scale
//! f64 zoom_factor
//! u32 evicted_count
//! u32 page_count
//! evicted_count × { u64 id }
//! page_count × {
//!     u32 page, u32 tile_count, f32 x, y, width, height
//!     tile_count × { u64 id, f32 x, y, width, height, u32 exact, u32 unused }
//! }
//! u32 thumbnail_first
//! u32 thumbnail_count
//! thumbnail_count × { u64 id }    0: not rendered yet
//! ```
//!
//! Finished tiles ([`take_finished`]): `u32 count`, `u32 unused`, then
//! `count × { u64 id, u32 width, u32 height }`.

use crate::{FinishedTile, Frame, Rect, ZoomMode, find_session};

/// Scrolls by logical pixels. `false` if the session is closed.
pub fn scroll_by(session: u64, dx: f64, dy: f64) -> bool {
    let Some(session) = find_session(session) else {
        return false;
    };
    session
        .lock()
        .viewer
        .scroll_by(pdfrs_geom::ViewVector::new(dx, dy));
    true
}

/// See `ViewSession::set_thumbnail_range`. `false` if the session is closed.
pub fn set_thumbnail_range(session: u64, first: u32, count: u32) -> bool {
    let Some(session) = find_session(session) else {
        return false;
    };
    let mut state = session.lock();
    let pages = state.viewer.page_count() as u32;
    let first = first.min(pages);
    state.thumbnails = (first, count.min(pages - first));
    true
}

/// Scrolls like a fling: in paged mode it stops at the edges of the current
/// page instead of sliding on to the next. `false` if the session is closed.
pub fn scroll_in_page_by(session: u64, dx: f64, dy: f64) -> bool {
    let Some(session) = find_session(session) else {
        return false;
    };
    session
        .lock()
        .viewer
        .scroll_in_page_by(pdfrs_geom::ViewVector::new(dx, dy));
    true
}

/// Multiplies the zoom around (`x`, `y`). `false` if the session is closed.
pub fn zoom_by(session: u64, factor: f64, x: f64, y: f64) -> bool {
    let Some(session) = find_session(session) else {
        return false;
    };
    session
        .lock()
        .viewer
        .zoom_by(factor, pdfrs_geom::ViewPoint::new(x, y));
    true
}

/// Plans a frame and writes it to `out`. Returns the number of bytes written.
/// If `out` is too small, returns minus the size needed; fetch the same frame
/// with [`last_frame`] and a larger buffer. Returns 0 for a closed session.
pub fn frame(session: u64, out: &mut [u8]) -> i64 {
    let Some(session) = find_session(session) else {
        return 0;
    };
    let mut state = session.lock();
    let frame = session.frame(&mut state);
    state.encoded.clear();
    encode_frame(&frame, &mut state.encoded);
    copy_out(&state.encoded, out)
}

/// The frame most recently planned by [`frame`], again.
pub fn last_frame(session: u64, out: &mut [u8]) -> i64 {
    let Some(session) = find_session(session) else {
        return 0;
    };
    copy_out(&session.lock().encoded, out)
}

/// Like `ViewSession::take_finished`. Sized so it always fits: at most
/// `(out.len() - 8) / 16` tiles are announced, and the rest stay queued for
/// the next call. Returns the number of bytes written (0: closed or no room).
pub fn take_finished(session: u64, out: &mut [u8]) -> i64 {
    let Some(session) = find_session(session) else {
        return 0;
    };
    if out.len() < 8 {
        return 0;
    }
    let tiles = session.take_finished();
    let mut buf = Vec::with_capacity(8 + tiles.len() * 16);
    // Anything that doesn't fit is dropped by the next `add_finished` and
    // simply rendered again; buffers are sized so this doesn't happen.
    let count = tiles.len().min((out.len() - 8) / 16);
    put_u32(&mut buf, count as u32);
    put_u32(&mut buf, 0);
    for FinishedTile { id, width, height } in tiles.into_iter().take(count) {
        buf.extend_from_slice(&id.to_ne_bytes());
        put_u32(&mut buf, width);
        put_u32(&mut buf, height);
    }
    copy_out(&buf, out)
}

fn copy_out(encoded: &[u8], out: &mut [u8]) -> i64 {
    if encoded.len() > out.len() {
        return -(encoded.len() as i64);
    }
    out[..encoded.len()].copy_from_slice(encoded);
    encoded.len() as i64
}

fn put_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_ne_bytes());
}

fn put_rect(buf: &mut Vec<u8>, r: &Rect) {
    for v in [r.x, r.y, r.width, r.height] {
        buf.extend_from_slice(&v.to_ne_bytes());
    }
}

pub(crate) fn encode_frame(frame: &Frame, buf: &mut Vec<u8>) {
    put_u32(
        buf,
        u32::from(frame.complete) | u32::from(frame.settled) << 1 | u32::from(frame.paged) << 2,
    );
    put_u32(
        buf,
        match frame.zoom_mode {
            ZoomMode::Fixed => 0,
            ZoomMode::FitWidth => 1,
            ZoomMode::FitPage => 2,
        },
    );
    put_u32(buf, frame.current_page);
    let settles = frame
        .settles_in_ms
        .map_or(-1, |ms| ms.min(i32::MAX as u32) as i32);
    buf.extend_from_slice(&settles.to_ne_bytes());
    buf.extend_from_slice(&frame.zoom.to_ne_bytes());
    buf.extend_from_slice(&frame.device_scale.to_ne_bytes());
    buf.extend_from_slice(&frame.zoom_factor.to_ne_bytes());
    put_u32(buf, frame.evicted.len() as u32);
    put_u32(buf, frame.pages.len() as u32);
    for id in &frame.evicted {
        buf.extend_from_slice(&id.to_ne_bytes());
    }
    for page in &frame.pages {
        put_u32(buf, page.page);
        put_u32(buf, page.tiles.len() as u32);
        put_rect(buf, &page.rect);
        for tile in &page.tiles {
            buf.extend_from_slice(&tile.id.to_ne_bytes());
            put_rect(buf, &tile.dest);
            put_u32(buf, u32::from(tile.exact));
            put_u32(buf, 0);
        }
    }
    put_u32(buf, frame.thumbnail_first);
    put_u32(buf, frame.thumbnails.len() as u32);
    for id in &frame.thumbnails {
        buf.extend_from_slice(&id.to_ne_bytes());
    }
}
