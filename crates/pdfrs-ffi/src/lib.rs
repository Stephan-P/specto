//! UniFFI surface for the Android app.
//!
//! A [`ViewSession`] is one open document on screen: it owns the
//! `pdfrs_view::Viewer` (layout, zoom, scrolling, tile planning) and the
//! render scheduler. Kotlin forwards gestures, asks for a [`Frame`] and draws
//! it. See `docs/ARCHITECTURE.md` ("Android").
//!
//! UniFFI is used for everything that happens occasionally (opening, modes,
//! viewport changes). Its Kotlin bindings go through JNA, and a single call
//! costs a few hundred microseconds on a phone, so what runs every frame goes
//! through plain JNI instead (`jni.rs`, backed by [`hot`]): scrolling,
//! zooming, planning a frame and collecting finished tiles.
//!
//! Tile pixels never cross UniFFI either (M0 measured that as too slow).
//! Kotlin owns one `Bitmap` per tile, keyed by an opaque id, and fills it
//! through JNI. Which tiles exist is decided here: the cache bookkeeping
//! lives in Rust, and [`Frame::evicted`] tells Kotlin which bitmaps to drop.

pub mod hot;
#[cfg(target_os = "android")]
mod jni;
mod tiles;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, Weak};
use std::time::Instant;

use pdfrs_geom::{DeviceRect, Rotation, ViewPoint, ViewVector};
use pdfrs_render::scheduler::Scheduler;
use pdfrs_render::{Bitmap, OpenError};
use pdfrs_source::{Link, NamedAction, OutlineItem, PageLabels, SourceDocument, Target};
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

/// How the document is presented, as offered in the app's menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum ViewMode {
    /// One page at a time, each fitted to the screen; swipe left and right.
    Paged,
    /// All pages in one column fitted to the width; scroll up and down.
    Continuous,
}

/// One line of the outline (table of contents), which is a tree flattened
/// in reading order.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct OutlineEntry {
    pub title: String,
    /// Nesting level, 0 for top-level items.
    pub depth: u32,
    /// Zero-based page the item leads to, if it leads into this document.
    pub page: Option<u32>,
    /// Whether deeper entries follow that belong to this one.
    pub has_children: bool,
}

/// What activating a link or outline entry did.
#[derive(Clone, Debug, PartialEq, uniffi::Enum)]
pub enum Activated {
    /// There was nothing to activate (or nothing we support).
    Nothing,
    /// The view went somewhere else in the document: plan a new frame.
    Moved,
    /// An external link. Opening it, after asking the user, is up to the app.
    Uri { uri: String },
}

/// What the app remembers about a document between sessions, to continue
/// where the reader left off.
#[derive(Clone, Copy, Debug, PartialEq, uniffi::Record)]
pub struct SavedView {
    /// Zero-based page at the top of the view.
    pub page: u32,
    /// How far down that page the view starts, as a fraction of its height.
    pub page_offset: f64,
    /// [`ViewMode::Paged`] rather than continuous.
    pub paged: bool,
    /// Logical pixels per point if the reader had zoomed in; `None` if the
    /// view was fitted.
    pub zoom: Option<f64>,
    /// Two pages side by side.
    pub two_pages: bool,
    /// With two pages: the first page on its own, like a book's cover.
    pub cover_alone: bool,
}

/// A distance in logical pixels (dp).
#[derive(Clone, Copy, Debug, PartialEq, uniffi::Record)]
pub struct Offset {
    pub x: f64,
    pub y: f64,
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
    /// The zoom has been still long enough that pages are drawn from tiles
    /// at the exact device scale, 1:1. While zooming they come from a few
    /// fixed zoom levels and are scaled.
    pub settled: bool,
    /// Call [`ViewSession::frame`] again after this many milliseconds even if
    /// nothing else happens: that is when the view settles and sharpens.
    pub settles_in_ms: Option<u32>,
    /// Device pixels per PDF point that pages are drawn at.
    pub device_scale: f64,
    /// Zero-based index of the page the user is looking at.
    pub current_page: u32,
    /// Logical pixels (dp) per PDF point.
    pub zoom: f64,
    pub zoom_mode: ZoomMode,
    /// The zoom relative to the view mode's fitted state (the whole page
    /// when paged, the width when continuous): 1.0 when fitted.
    pub zoom_factor: f64,
    /// Whether the view is [`ViewMode::Paged`].
    pub paged: bool,
    /// Whether pages are shown two side by side.
    pub two_pages: bool,
    /// With two pages: whether the first page is on its own.
    pub cover_alone: bool,
    /// First page of the range set with [`ViewSession::set_thumbnail_range`].
    pub thumbnail_first: u32,
    /// For each page of that range, the id of its thumbnail tile, or 0 while
    /// it is still being rendered.
    pub thumbnails: Vec<u64>,
}

/// Size of a page as displayed, in PDF points.
#[derive(Clone, Copy, Debug, PartialEq, uniffi::Record)]
pub struct PageSize {
    pub width: f32,
    pub height: f32,
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
    /// The last frame encoded for [`hot::frame`], kept so a caller whose
    /// buffer was too small can fetch it again without planning a new one.
    encoded: Vec<u8>,
    /// Pages whose thumbnails the frontend is showing (first, count).
    thumbnails: (u32, u32),
    /// Structure (links), separate from rendering.
    source: SourceDocument,
    /// Links per page, parsed on first use.
    links: HashMap<usize, Vec<Link>>,
}

impl State {
    /// Does what a link or outline entry asks for.
    fn follow(&mut self, target: &Target) -> Activated {
        let viewer = &mut self.viewer;
        match target {
            Target::Destination(d) => viewer.go_to_pdf_position(d.page, d.left, d.top),
            Target::Named(NamedAction::NextPage) => viewer.next_page(),
            Target::Named(NamedAction::PrevPage) => viewer.previous_page(),
            Target::Named(NamedAction::FirstPage) => viewer.go_to_page(0),
            Target::Named(NamedAction::LastPage) => {
                viewer.go_to_page(viewer.page_count().saturating_sub(1));
            }
            Target::Uri(uri) => return Activated::Uri { uri: uri.clone() },
        }
        Activated::Moved
    }
}

/// Flattens the outline tree in reading order.
fn flatten_outline(
    items: Vec<OutlineItem>,
    depth: u32,
    out: &mut Vec<(OutlineEntry, Option<Target>)>,
) {
    for item in items {
        let page = match &item.target {
            Some(Target::Destination(d)) => Some(d.page as u32),
            _ => None,
        };
        out.push((
            OutlineEntry {
                title: item.title,
                depth,
                page,
                has_children: !item.children.is_empty(),
            },
            item.target,
        ));
        flatten_outline(item.children, depth + 1, out);
    }
}

struct Shared {
    scheduler: Scheduler,
    state: Mutex<State>,
    /// The outline with what each entry does, read once at open.
    outline: Vec<(OutlineEntry, Option<Target>)>,
    /// The document's own page numbering ("iv", "A-3", …), unless that is
    /// just 1, 2, 3.
    labels: Option<PageLabels>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn frame(&self, state: &mut State) -> Frame {
        let State {
            viewer,
            tiles,
            thumbnails,
            ..
        } = state;
        tiles.begin_frame();
        let now = Instant::now();
        let mut planned = viewer.frame(now, |key| tiles.knows(key));
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
        // Thumbnails are the pages' placeholder tiles, so the page view and
        // the thumbnail grid share one render. They are requested after
        // everything the page view wants: the scheduler has one queue, and
        // each frame replaces it.
        let (thumbnail_first, count) = *thumbnails;
        let thumbnail_ids = (thumbnail_first..thumbnail_first.saturating_add(count))
            .map(|page| {
                let Some(key) = viewer.placeholder_key(page as usize) else {
                    return 0;
                };
                tiles.use_tile(&key).unwrap_or_else(|| {
                    if !tiles.knows(&key) && !planned.requests.contains(&key) {
                        planned.requests.push(key);
                    }
                    0
                })
            })
            .collect();
        self.scheduler.request(planned.requests);
        Frame {
            thumbnail_first,
            thumbnails: thumbnail_ids,
            pages,
            evicted: tiles.take_evicted(),
            complete: planned.complete,
            settled: viewer.is_settled(now),
            settles_in_ms: planned
                .settles_at
                .map(|at| at.saturating_duration_since(now).as_millis() as u32),
            device_scale: viewer.device_scale().get(),
            current_page: viewer.current_page() as u32,
            zoom_factor: viewer.zoom_relative_to(home_fit(viewer)),
            paged: viewer.settings().swipe,
            two_pages: viewer.layout_mode().is_two_page(),
            cover_alone: viewer.settings().cover_alone,
            zoom: viewer.zoom(),
            zoom_mode: match viewer.zoom_mode() {
                pdfrs_view::ZoomMode::Fixed => ZoomMode::Fixed,
                pdfrs_view::ZoomMode::FitWidth => ZoomMode::FitWidth,
                pdfrs_view::ZoomMode::FitPage => ZoomMode::FitPage,
            },
        }
    }

    fn take_finished(&self) -> Vec<FinishedTile> {
        let finished = self.scheduler.take_finished();
        let mut state = self.lock();
        let State { viewer, tiles, .. } = &mut *state;
        tiles.add_finished(finished, |key| {
            viewer.placeholder_key(key.page as usize) == Some(*key)
        })
    }
}

/// The zoom mode a view returns to: the whole page when swiping through
/// pages, the width when scrolling.
fn home_fit(viewer: &Viewer) -> pdfrs_view::ZoomMode {
    if viewer.settings().swipe {
        pdfrs_view::ZoomMode::FitPage
    } else {
        pdfrs_view::ZoomMode::FitWidth
    }
}

/// The layout for swiping or scrolling through single pages or pairs.
fn layout_for(paged: bool, two_pages: bool) -> pdfrs_view::LayoutMode {
    use pdfrs_view::LayoutMode::{Continuous, SinglePage, TwoPage, TwoPageContinuous};
    match (paged, two_pages) {
        (true, false) => SinglePage,
        (true, true) => TwoPage,
        (false, false) => Continuous,
        (false, true) => TwoPageContinuous,
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

/// The Rust half of `Native.copyTile`: hands the pixels of finished
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
        let data: Arc<[u8]> = data.into();
        let opened = scheduler.open(data.clone(), password.as_deref())?;
        let source = SourceDocument::open(data, password.as_deref())?;
        let mut outline = Vec::new();
        flatten_outline(source.outline(), 0, &mut outline);
        let labels = source.page_labels().filter(|l| !l.is_plain());
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
                encoded: Vec::new(),
                thumbnails: (0, 0),
                source,
                links: HashMap::new(),
            }),
            outline,
            labels,
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

    /// Identifies this session to the JNI functions in `Native`.
    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn page_count(&self) -> u32 {
        self.page_count
    }

    /// The pages' printed numbers ("i", "ii", "1", "A-3", …), one per page,
    /// for documents that number their pages other than 1, 2, 3. `None`
    /// otherwise: then a page's number is its position.
    pub fn page_labels(&self) -> Option<Vec<String>> {
        let labels = self.shared.labels.as_ref()?;
        Some(
            (0..self.page_count as usize)
                .map(|i| labels.label(i))
                .collect(),
        )
    }

    /// The zero-based page the user means by `text`: a printed page number
    /// (exactly, then ignoring case), else a position counted from 1.
    pub fn find_page(&self, text: String) -> Option<u32> {
        let text = text.trim();
        let labelled = self.shared.labels.as_ref().and_then(|l| l.find(text));
        labelled
            .or_else(|| {
                let n = text.parse::<usize>().ok()?;
                (1..=self.page_count as usize).contains(&n).then(|| n - 1)
            })
            .map(|i| i as u32)
    }

    /// The outline (table of contents), flattened in reading order. Empty if
    /// the document has none.
    pub fn outline(&self) -> Vec<OutlineEntry> {
        self.shared.outline.iter().map(|(e, _)| e.clone()).collect()
    }

    /// Goes where outline entry `index` (into [`outline`](Self::outline)) leads.
    pub fn activate_outline(&self, index: u32) -> Activated {
        match self.shared.outline.get(index as usize) {
            Some((_, Some(target))) => self.shared.lock().follow(target),
            _ => Activated::Nothing,
        }
    }

    /// A tap at (`x`, `y`) in logical pixels from the view's top-left: follows
    /// the link there, if there is one.
    pub fn tap(&self, x: f64, y: f64) -> Activated {
        let mut state = self.shared.lock();
        let Some((page, at)) = state.viewer.hit_test(ViewPoint::new(x, y)) else {
            return Activated::Nothing;
        };
        let Some(view_page) = state.viewer.pages().get(page) else {
            return Activated::Nothing;
        };
        let pdf = view_page.displayed().page_to_pdf().transform_point(at);
        let source_page = view_page.page as usize;
        let State { source, links, .. } = &mut *state;
        let target = links
            .entry(source_page)
            .or_insert_with(|| source.links(source_page))
            .iter()
            // The topmost link is the last one drawn.
            .rev()
            .find(|l| l.contains(pdf))
            .map(|l| l.target.clone());
        match target {
            Some(target) => state.follow(&target),
            None => Activated::Nothing,
        }
    }

    /// Sizes of all pages as displayed, e.g. for the thumbnail grid's cells.
    pub fn page_sizes(&self) -> Vec<PageSize> {
        let state = self.shared.lock();
        state
            .viewer
            .pages()
            .iter()
            .map(|page| {
                let size = page.displayed().size();
                PageSize {
                    width: size.width as f32,
                    height: size.height as f32,
                }
            })
            .collect()
    }

    /// The pages whose thumbnails are on screen (`count` may be 0). Frames
    /// report their tiles in [`Frame::thumbnails`], keep them cached, and
    /// request the ones still missing.
    pub fn set_thumbnail_range(&self, first: u32, count: u32) {
        hot::set_thumbnail_range(self.id, first, count);
    }

    /// View size in logical pixels (dp) and device pixels per logical pixel
    /// (the display density, e.g. 2.625).
    pub fn set_viewport(&self, width: f64, height: f64, scale: f64) {
        hot::set_viewport(self.id, width, height, scale);
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

    /// Switches between swiping through pages and scrolling, keeping the
    /// page. Sessions start out [`ViewMode::Continuous`].
    pub fn set_view_mode(&self, mode: ViewMode) {
        let mut state = self.shared.lock();
        let viewer = &mut state.viewer;
        let paged = mode == ViewMode::Paged;
        viewer.set_layout_mode(layout_for(paged, viewer.layout_mode().is_two_page()));
        viewer.set_swipe(paged);
        viewer.set_zoom_mode(home_fit(viewer));
    }

    /// Shows pages two side by side (or one at a time again), in either view
    /// mode, keeping the page. With `cover_alone`, the first page stands on
    /// its own, so that a book's left and right pages end up left and right.
    pub fn set_spread(&self, two_pages: bool, cover_alone: bool) {
        let mut state = self.shared.lock();
        let viewer = &mut state.viewer;
        let layout = layout_for(viewer.settings().swipe, two_pages);
        if layout == viewer.layout_mode() && cover_alone == viewer.settings().cover_alone {
            return;
        }
        viewer.set_layout_mode(layout);
        viewer.set_cover_alone(cover_alone);
        viewer.set_zoom_mode(home_fit(viewer));
    }

    /// Where the reader is, to be stored and passed to
    /// [`restore_view`](Self::restore_view) the next time the document is opened.
    pub fn saved_view(&self) -> SavedView {
        let state = self.shared.lock();
        let view = state.viewer.view_state();
        SavedView {
            page: view.page as u32,
            page_offset: view.page_offset,
            paged: state.viewer.settings().swipe,
            zoom: (view.zoom_mode == pdfrs_view::ZoomMode::Fixed).then_some(view.zoom),
            two_pages: view.layout.is_two_page(),
            cover_alone: view.cover_alone,
        }
    }

    /// Continues where [`saved_view`](Self::saved_view) left off. Call once
    /// the viewport is set. Values that no longer fit (the file changed) are
    /// clamped.
    pub fn restore_view(&self, saved: SavedView) {
        self.set_view_mode(if saved.paged {
            ViewMode::Paged
        } else {
            ViewMode::Continuous
        });
        self.set_spread(saved.two_pages, saved.cover_alone);
        let mut state = self.shared.lock();
        let viewer = &mut state.viewer;
        let mut view = viewer.view_state();
        view.page = saved.page as usize;
        view.page_offset = saved.page_offset;
        match saved.zoom {
            Some(zoom) => {
                view.zoom_mode = pdfrs_view::ZoomMode::Fixed;
                view.zoom = zoom;
            }
            None => view.zoom_mode = home_fit(viewer),
        }
        viewer.restore_view_state(&view);
    }

    /// Back to the view mode's fitted zoom (after zooming in).
    pub fn fit(&self) {
        let mut state = self.shared.lock();
        let fit = home_fit(&state.viewer);
        state.viewer.set_zoom_mode(fit);
    }

    /// Call when a drag or pinch ends in paged mode: how far the view still
    /// has to scroll to come to rest on a page (animate `scrollBy` over it),
    /// or `None` if it rests where it is. (`vx`, `vy`) is the scrolling
    /// velocity at release in logical px/s; a flick turns the page.
    pub fn snap_delta(&self, vx: f64, vy: f64) -> Option<Offset> {
        let state = self.shared.lock();
        let delta = state.viewer.snap_delta(ViewVector::new(vx, vy))?;
        Some(Offset {
            x: delta.x,
            y: delta.y,
        })
    }

    /// A pinch (or zoom animation) starts. Until [`end_zoom_gesture`](Self::end_zoom_gesture),
    /// no exact-scale tiles are rendered for the zooms passed through.
    pub fn begin_zoom_gesture(&self) {
        self.shared.lock().viewer.begin_zoom_gesture();
    }

    pub fn end_zoom_gesture(&self) {
        self.shared.lock().viewer.end_zoom_gesture();
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
        self.shared.frame(&mut self.shared.lock())
    }

    /// Tiles that finished rendering. For each, create a bitmap of the given
    /// size and pass it to `Native.copyTile`; then call [`frame`](Self::frame).
    /// Tiles not copied before the next call are discarded.
    pub fn take_finished(&self) -> Vec<FinishedTile> {
        self.shared.take_finished()
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
