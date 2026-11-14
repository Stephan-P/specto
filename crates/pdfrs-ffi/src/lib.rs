//! UniFFI surface for the Android app.
//!
//! A [`ViewSession`] is one open document on screen: it owns the
//! `pdfrs_view::Viewer` (layout, zoom, scrolling, tile planning) and the
//! render scheduler. Kotlin forwards gestures, asks for a [`Frame`] and draws
//! it. See `docs/ARCHITECTURE.md` ("Android").
//!
//! Tile pixels never cross UniFFI (M0 measured that as too slow). Kotlin owns
//! one `Bitmap` per tile, keyed by an opaque id, and fills it through the JNI
//! function in `android_bitmap.rs`. Which tiles exist is decided here: the
//! cache bookkeeping lives in Rust, and [`Frame::evicted`] tells Kotlin which
//! bitmaps to drop.

#[cfg(target_os = "android")]
mod android_bitmap;
mod tiles;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, Weak};

use pdfrs_geom::{DeviceRect, Rotation, ViewPoint, ViewSize, ViewVector};
use pdfrs_render::scheduler::Scheduler;
use pdfrs_render::{Bitmap, OpenError};
use pdfrs_view::{Settings, ViewPage, Viewer};

use tiles::TileBook;

uniffi::setup_scaffolding!();

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum PdfError {
    #[error("the document is password protected")]
    PasswordRequired,
    #[error("the document uses unsupported or invalid encryption")]
    UnsupportedEncryption,
    #[error("not a valid PDF file")]
    Invalid,
}

impl From<OpenError> for PdfError {
    fn from(e: OpenError) -> Self {
        match e {
            OpenError::PasswordRequired => Self::PasswordRequired,
            OpenError::UnsupportedEncryption | OpenError::InvalidEncryption => {
                Self::UnsupportedEncryption
            }
            OpenError::Invalid => Self::Invalid,
        }
    }
}

/// Implemented in Kotlin.
#[uniffi::export(with_foreign)]
pub trait ViewListener: Send + Sync {
    /// Finished tiles are waiting in [`ViewSession::take_finished`].
    /// Called from a render thread, so hop to the main thread first.
    fn tiles_ready(&self);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum LayoutMode {
    SinglePage,
    Continuous,
    TwoPage,
    TwoPageContinuous,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum ZoomMode {
    Fixed,
    FitWidth,
    FitPage,
}

/// A rectangle in device pixels, relative to the top-left of the view.
#[derive(Clone, Copy, Debug, PartialEq, uniffi::Record)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl From<DeviceRect> for Rect {
    fn from(r: DeviceRect) -> Self {
        Self {
            x: r.origin.x as f32,
            y: r.origin.y as f32,
            width: r.size.width as f32,
            height: r.size.height as f32,
        }
    }
}

#[derive(Clone, Copy, Debug, uniffi::Record)]
pub struct TileDraw {
    /// Id of a tile previously handed out by [`ViewSession::take_finished`].
    pub id: u64,
    pub dest: Rect,
    /// The tile maps 1:1 onto device pixels: draw without filtering.
    pub exact: bool,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct PageDraw {
    /// Zero-based page index.
    pub page: u32,
    /// Fill this white, then draw `tiles` in order, clipped to it.
    pub rect: Rect,
    pub tiles: Vec<TileDraw>,
}

/// Everything needed to draw the view once.
#[derive(Clone, Debug, uniffi::Record)]
pub struct Frame {
    pub pages: Vec<PageDraw>,
    /// Tiles dropped from the cache since the last frame: release their bitmaps.
    pub evicted: Vec<u64>,
    /// Every visible tile is sharp. If not, more tiles are on their way.
    pub complete: bool,
    /// Zero-based index of the page the user is looking at.
    pub current_page: u32,
    /// Logical pixels (dp) per PDF point.
    pub zoom: f64,
    pub zoom_mode: ZoomMode,
}

/// A rendered tile waiting to be copied into a `Bitmap` of this size.
#[derive(Clone, Copy, Debug, uniffi::Record)]
pub struct FinishedTile {
    pub id: u64,
    pub width: u32,
    pub height: u32,
}

struct State {
    viewer: Viewer,
    tiles: TileBook,
}

struct Shared {
    scheduler: Scheduler,
    state: Mutex<State>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Live sessions by id, so the JNI function can find one without being handed
/// a raw pointer. A closed session simply isn't found.
static SESSIONS: LazyLock<Mutex<HashMap<u64, Weak<Shared>>>> = LazyLock::new(Mutex::default);
static NEXT_SESSION: AtomicU64 = AtomicU64::new(1);

fn find_session(id: u64) -> Option<Arc<Shared>> {
    let sessions = SESSIONS.lock().unwrap_or_else(|e| e.into_inner());
    sessions.get(&id)?.upgrade()
}

/// The Rust half of `NativeBitmap.copyTile`: hands the pixels of finished
/// tile `tile` of session `session` to `write`, which copies them into the
/// frontend's bitmap. If `write` returns `true`, the tile counts as cached
/// from now on. Returns `false` for unknown sessions and tiles, or if `write`
/// did.
pub fn claim_tile(session: u64, tile: u64, write: impl FnOnce(&Bitmap) -> bool) -> bool {
    let Some(session) = find_session(session) else {
        return false;
    };
    session.lock().tiles.claim(tile, write)
}

/// One document shown in one view.
#[derive(uniffi::Object)]
pub struct ViewSession {
    id: u64,
    page_count: u32,
    shared: Arc<Shared>,
}

#[uniffi::export]
impl ViewSession {
    /// Opens a PDF. `cache_bytes` is the budget for tile bitmaps.
    #[uniffi::constructor]
    pub fn open(
        data: Vec<u8>,
        password: Option<String>,
        cache_bytes: u64,
        listener: Arc<dyn ViewListener>,
    ) -> Result<Arc<Self>, PdfError> {
        let threads = std::thread::available_parallelism().map_or(2, |n| n.get().clamp(2, 4));
        let scheduler = Scheduler::new(threads, move || listener.tiles_ready());
        let opened = scheduler.open(data, password.as_deref())?;
        let pages: Vec<ViewPage> = opened
            .pages
            .iter()
            .enumerate()
            .map(|(i, &geometry)| ViewPage {
                doc: opened.id,
                page: i as u32,
                geometry,
                rotation: Rotation::None,
            })
            .collect();
        let settings = Settings {
            // Phone screens are narrow: spend less of them on chrome.
            gap: 8.0,
            margin: 8.0,
            ..Settings::default()
        };
        let shared = Arc::new(Shared {
            scheduler,
            state: Mutex::new(State {
                viewer: Viewer::new(pages, settings),
                tiles: TileBook::new(cache_bytes as usize),
            }),
        });
        let id = NEXT_SESSION.fetch_add(1, Ordering::Relaxed);
        SESSIONS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, Arc::downgrade(&shared));
        Ok(Arc::new(Self {
            id,
            page_count: opened.pages.len() as u32,
            shared,
        }))
    }

    /// Identifies this session to `NativeBitmap.copyTile`.
    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn page_count(&self) -> u32 {
        self.page_count
    }

    /// View size in logical pixels (dp) and device pixels per logical pixel
    /// (the display density, e.g. 2.625).
    pub fn set_viewport(&self, width: f64, height: f64, scale: f64) {
        self.shared
            .lock()
            .viewer
            .set_viewport(ViewSize::new(width, height), scale);
    }

    /// Scrolls the content by this many logical pixels.
    pub fn scroll_by(&self, dx: f64, dy: f64) {
        self.shared.lock().viewer.scroll_by(ViewVector::new(dx, dy));
    }

    /// Multiplies the zoom, keeping the content under (`x`, `y`) in place
    /// (logical pixels from the view's top-left).
    pub fn zoom_by(&self, factor: f64, x: f64, y: f64) {
        self.shared
            .lock()
            .viewer
            .zoom_by(factor, ViewPoint::new(x, y));
    }

    pub fn set_zoom_mode(&self, mode: ZoomMode) {
        self.shared.lock().viewer.set_zoom_mode(match mode {
            ZoomMode::Fixed => pdfrs_view::ZoomMode::Fixed,
            ZoomMode::FitWidth => pdfrs_view::ZoomMode::FitWidth,
            ZoomMode::FitPage => pdfrs_view::ZoomMode::FitPage,
        });
    }

    pub fn set_layout_mode(&self, mode: LayoutMode) {
        self.shared.lock().viewer.set_layout_mode(match mode {
            LayoutMode::SinglePage => pdfrs_view::LayoutMode::SinglePage,
            LayoutMode::Continuous => pdfrs_view::LayoutMode::Continuous,
            LayoutMode::TwoPage => pdfrs_view::LayoutMode::TwoPage,
            LayoutMode::TwoPageContinuous => pdfrs_view::LayoutMode::TwoPageContinuous,
        });
    }

    /// Shows `page` (zero-based) at the top of the view.
    pub fn go_to_page(&self, page: u32) {
        self.shared.lock().viewer.go_to_page(page as usize);
    }

    /// Plans what to draw now and queues the tiles that are still missing.
    /// Call after anything changed: input, a new viewport, or new tiles.
    pub fn frame(&self) -> Frame {
        let mut state = self.shared.lock();
        let State { viewer, tiles } = &mut *state;
        tiles.begin_frame();
        let planned = viewer.frame(|key| tiles.knows(key));
        let pages = planned
            .pages
            .iter()
            .map(|page| PageDraw {
                page: page.page as u32,
                rect: page.rect.into(),
                tiles: page
                    .layers
                    .iter()
                    .filter_map(|layer| {
                        Some(TileDraw {
                            id: tiles.use_tile(&layer.key)?,
                            dest: layer.dest.into(),
                            exact: layer.exact,
                        })
                    })
                    .collect(),
            })
            .collect();
        self.shared.scheduler.request(planned.requests);
        Frame {
            pages,
            evicted: tiles.take_evicted(),
            complete: planned.complete,
            current_page: viewer.current_page() as u32,
            zoom: viewer.zoom(),
            zoom_mode: match viewer.zoom_mode() {
                pdfrs_view::ZoomMode::Fixed => ZoomMode::Fixed,
                pdfrs_view::ZoomMode::FitWidth => ZoomMode::FitWidth,
                pdfrs_view::ZoomMode::FitPage => ZoomMode::FitPage,
            },
        }
    }

    /// Tiles that finished rendering. For each, create a bitmap of the given
    /// size and pass it to `NativeBitmap.copyTile`; then call [`frame`](Self::frame).
    /// Tiles not copied before the next call are discarded.
    pub fn take_finished(&self) -> Vec<FinishedTile> {
        let finished = self.shared.scheduler.take_finished();
        self.shared.lock().tiles.add_finished(finished)
    }

    /// Drops every tile the last frame didn't use (`onTrimMemory`). Their ids
    /// show up in the next frame's `evicted`.
    pub fn trim_memory(&self) {
        self.shared.lock().tiles.trim();
    }
}

impl Drop for ViewSession {
    fn drop(&mut self) {
        SESSIONS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.id);
    }
}

#[cfg(test)]
mod tests;
