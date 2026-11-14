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
use pdfrs_source::{Link, NamedAction, OutlineItem, PageLabels, PdfData, SourceDocument, Target};
use pdfrs_text::{Extractor, TextPage};
use pdfrs_view::{
    Granularity, Selection, Settings, TextPages, ViewPage, Viewer, over_text, text_char,
    text_position,
};

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
    // Not `message`: in the generated Kotlin exception that would clash
    // with `Throwable.message`, and the bindings wouldn't compile.
    #[error("couldn't read the file: {reason}")]
    Io { reason: String },
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
    /// Fitted means the page's width rather than the whole page; see
    /// [`ViewSession::set_fit_width`].
    pub fit_width: bool,
}

/// What a document says about itself (its `/Info` dictionary and header),
/// for a properties dialog. Any of it may be missing or wrong: it is whatever
/// the program that wrote the file put there.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct DocumentInfo {
    pub title: Option<String>,
    pub author: Option<String>,
    pub subject: Option<String>,
    pub keywords: Option<String>,
    /// The program the document was written in.
    pub creator: Option<String>,
    /// The program that turned it into a PDF.
    pub producer: Option<String>,
    /// ISO 8601 with the offset from UTC, e.g. `2024-03-05T14:22:10+01:00`.
    pub created: Option<String>,
    pub modified: Option<String>,
    /// E.g. "1.7".
    pub pdf_version: String,
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

/// A point in device pixels, relative to the top-left of the view.
#[derive(Clone, Copy, Debug, PartialEq, uniffi::Record)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

/// Text to highlight: the part of one line that is selected, as the four
/// corners of a quadrilateral (lines can be rotated). "Start" and "end" are
/// along the line, "bottom" is its baseline side.
#[derive(Clone, Copy, Debug, PartialEq, uniffi::Record)]
pub struct Highlight {
    pub start_bottom: Point,
    pub end_bottom: Point,
    pub end_top: Point,
    pub start_top: Point,
}

/// One end of the selection on screen: the edge of its first or last line,
/// where a handle for moving that end belongs.
#[derive(Clone, Copy, Debug, PartialEq, uniffi::Record)]
pub struct SelectionEdge {
    pub bottom: Point,
    pub top: Point,
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
    /// Zero-based index of the page the user is looking at; with two pages,
    /// the first of the pair.
    pub current_page: u32,
    /// The last page shown together with it: the same page, or with two
    /// pages the second of the pair.
    pub last_page: u32,
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
    /// Whether any text is selected, on screen or not.
    pub selection: bool,
    /// The selected text on the visible pages: draw these over the pages.
    pub highlights: Vec<Highlight>,
    /// Where the selection starts and ends, for its handles; `None` for an
    /// end that isn't on a visible page.
    pub selection_start: Option<SelectionEdge>,
    pub selection_end: Option<SelectionEdge>,
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
    /// What the view is fitted to unless the reader zoomed, and returns to:
    /// `FitPage` or `FitWidth`.
    fit: pdfrs_view::ZoomMode,
    /// Text pages extracted so far, for selecting.
    texts: TextCache,
    selection: Option<Selection>,
}

/// How many pages' text is kept (the least recently used go first). A page
/// takes about 100 bytes per character.
const TEXT_PAGES: usize = 24;

/// Text pages by page number.
#[derive(Default)]
struct TextCache {
    pages: HashMap<usize, (Arc<TextPage>, u64)>,
    clock: u64,
}

/// The document's text pages, extracted when first needed: a fraction of a
/// millisecond to a few milliseconds per page.
struct TextLayer<'a> {
    source: &'a SourceDocument,
    cache: &'a mut TextCache,
}

impl TextPages for TextLayer<'_> {
    fn text_page(&mut self, page: usize) -> Option<Arc<TextPage>> {
        let cache = &mut *self.cache;
        cache.clock += 1;
        if let Some((text, used)) = cache.pages.get_mut(&page) {
            *used = cache.clock;
            return Some(text.clone());
        }
        let text = Arc::new(Extractor::new(self.source.pdf()).page(page)?);
        cache.pages.insert(page, (text.clone(), cache.clock));
        if cache.pages.len() > TEXT_PAGES
            && let Some(&oldest) = cache
                .pages
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(p, _)| p)
        {
            cache.pages.remove(&oldest);
        }
        Some(text)
    }
}

impl State {
    /// The viewer, the text layer and the selection, to work on together.
    fn text(&mut self) -> (&Viewer, TextLayer<'_>, &mut Option<Selection>) {
        let texts = TextLayer {
            source: &self.source,
            cache: &mut self.texts,
        };
        (&self.viewer, texts, &mut self.selection)
    }

    /// Moves the end of the selection that isn't its anchor to the text
    /// nearest to `p`. A selection never shrinks to nothing this way.
    fn select_to(&mut self, p: ViewPoint) {
        let (viewer, mut texts, selection) = self.text();
        let (Some(s), Some(to)) = (selection.as_mut(), text_position(viewer, &mut texts, p)) else {
            return;
        };
        let before = s.clone();
        s.extend(&mut texts, to);
        if s.is_empty() {
            *s = before;
        }
    }

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
            fit,
            source,
            texts,
            selection,
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

        // The selection on the visible pages. Positions come in logical
        // pixels; everything in a frame is in device pixels.
        let selection = selection.as_ref().filter(|s| !s.is_empty());
        let mut highlights = Vec::new();
        let (mut selection_start, mut selection_end) = (None, None);
        if let Some(selection) = selection {
            let px = viewer.device_scale().get() / viewer.zoom();
            let point = |p: ViewPoint| Point {
                x: (p.x * px) as f32,
                y: (p.y * px) as f32,
            };
            let mut texts = TextLayer {
                source,
                cache: texts,
            };
            let (start, end) = selection.range();
            for page in &planned.pages {
                let quads = selection.view_quads(viewer, &mut texts, page.page);
                if let (true, Some(q)) = (page.page == start.page, quads.first()) {
                    selection_start = Some(SelectionEdge {
                        bottom: point(q[0]),
                        top: point(q[3]),
                    });
                }
                if let (true, Some(q)) = (page.page == end.page, quads.last()) {
                    selection_end = Some(SelectionEdge {
                        bottom: point(q[1]),
                        top: point(q[2]),
                    });
                }
                highlights.extend(quads.iter().map(|q| Highlight {
                    start_bottom: point(q[0]),
                    end_bottom: point(q[1]),
                    end_top: point(q[2]),
                    start_top: point(q[3]),
                }));
            }
        }
        Frame {
            selection: selection.is_some(),
            highlights,
            selection_start,
            selection_end,
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
            current_page: viewer.current_spread().first().map_or(0, |&p| p as u32),
            last_page: viewer.current_spread().last().map_or(0, |&p| p as u32),
            zoom_factor: viewer.zoom_relative_to(*fit),
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

/// The fit a view mode starts with: the whole page when swiping through
/// pages, the width when scrolling.
fn default_fit(paged: bool) -> pdfrs_view::ZoomMode {
    fit_mode(!paged)
}

fn fit_mode(fit_width: bool) -> pdfrs_view::ZoomMode {
    if fit_width {
        pdfrs_view::ZoomMode::FitWidth
    } else {
        pdfrs_view::ZoomMode::FitPage
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

/// The bytes of a file to open, read once and reusable, e.g. to try again
/// with a password.
#[derive(uniffi::Object)]
pub struct PdfFile(PdfData);

#[uniffi::export]
impl PdfFile {
    /// Takes over the open file descriptor `fd` (from
    /// `ParcelFileDescriptor.detachFd()`) and reads the file, or maps it if
    /// it's large; see [`PdfData`]. The descriptor is closed either way.
    /// Pipes, as some document providers hand out, are read.
    #[uniffi::constructor]
    pub fn from_fd(fd: i32) -> Result<Arc<Self>, PdfError> {
        if fd < 0 {
            return Err(PdfError::Io {
                reason: "invalid file descriptor".into(),
            });
        }
        #[cfg(unix)]
        {
            use std::os::fd::FromRawFd;
            // SAFETY: the caller hands over ownership of an open descriptor
            // (detachFd), so nothing else closes or uses it.
            #[allow(unsafe_code)]
            let file = unsafe { std::fs::File::from_raw_fd(fd) };
            PdfData::from_file(file)
                .map(|d| Arc::new(Self(d)))
                .map_err(|e| PdfError::Io {
                    reason: e.to_string(),
                })
        }
        #[cfg(not(unix))]
        Err(PdfError::Io {
            reason: "file descriptors are not supported here".into(),
        })
    }

    /// Bytes already in memory.
    #[uniffi::constructor]
    pub fn from_bytes(data: Vec<u8>) -> Arc<Self> {
        Arc::new(Self(data.into()))
    }

    pub fn size(&self) -> u64 {
        self.0.len() as u64
    }

    /// Whether the file is mapped rather than read into memory.
    pub fn is_mapped(&self) -> bool {
        self.0.is_mapped()
    }
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
        Self::open_data(data.into(), password, cache_bytes, listener)
    }

    /// Opens `file`, as [`open`](Self::open) does.
    #[uniffi::constructor]
    pub fn open_file(
        file: Arc<PdfFile>,
        password: Option<String>,
        cache_bytes: u64,
        listener: Arc<dyn ViewListener>,
    ) -> Result<Arc<Self>, PdfError> {
        Self::open_data(file.0.clone(), password, cache_bytes, listener)
    }
}

impl ViewSession {
    fn open_data(
        data: PdfData,
        password: Option<String>,
        cache_bytes: u64,
        listener: Arc<dyn ViewListener>,
    ) -> Result<Arc<Self>, PdfError> {
        let threads = std::thread::available_parallelism().map_or(2, |n| n.get().clamp(2, 4));
        let scheduler = Scheduler::new(threads, move || listener.tiles_ready());
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
                // Sessions start out scrolling.
                fit: default_fit(false),
                texts: TextCache::default(),
                selection: None,
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
}

#[uniffi::export]
impl ViewSession {
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

    /// What the document says about itself.
    pub fn info(&self) -> DocumentInfo {
        let state = self.shared.lock();
        let m = state.source.metadata();
        DocumentInfo {
            title: m.title,
            author: m.author,
            subject: m.subject,
            keywords: m.keywords,
            creator: m.creator,
            producer: m.producer,
            created: m.created.map(|d| d.to_string()),
            modified: m.modified.map(|d| d.to_string()),
            pdf_version: state.source.version().to_owned(),
        }
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

    // --- text selection ------------------------------------------------------
    //
    // Positions are in logical pixels from the view's top-left, like taps.
    // The selection shows up in frames: [`Frame::highlights`] and the edges
    // for its handles.

    /// A long press at (`x`, `y`): selects the word there and returns true,
    /// or does nothing and returns false if there is no text there. Moving
    /// on with [`select_to`](Self::select_to) then selects word by word.
    pub fn select_word_at(&self, x: f64, y: f64) -> bool {
        let mut state = self.shared.lock();
        let (viewer, mut texts, selection) = state.text();
        let p = ViewPoint::new(x, y);
        if !over_text(viewer, &mut texts, p) {
            return false;
        }
        let Some(at) = text_char(viewer, &mut texts, p) else {
            return false;
        };
        let word = Selection::new(&mut texts, at, Granularity::Word);
        if word.is_empty() {
            return false;
        }
        *selection = Some(word);
        true
    }

    /// Takes hold of one end of the selection (its end, or else its start),
    /// as when a handle is touched: [`select_to`](Self::select_to) then
    /// moves that end, character by character, and the other one stays.
    pub fn grab_selection(&self, end: bool) {
        let mut state = self.shared.lock();
        let (_, mut texts, selection) = state.text();
        let Some(current) = selection.as_ref() else {
            return;
        };
        let (start, stop) = current.range();
        let (fixed, moving) = if end { (start, stop) } else { (stop, start) };
        let mut held = Selection::new(&mut texts, fixed, Granularity::Char);
        held.extend(&mut texts, moving);
        *selection = Some(held);
    }

    /// Extends the selection to the text nearest to (`x`, `y`): from the
    /// word first selected, or moving the end last grabbed. `Native.selectTo`
    /// does the same without UniFFI's overhead, for drags.
    pub fn select_to(&self, x: f64, y: f64) {
        hot::select_to(self.id, x, y);
    }

    /// Selects the whole document.
    pub fn select_all(&self) {
        let mut state = self.shared.lock();
        state.selection = Some(Selection::all(self.page_count as usize));
    }

    pub fn clear_selection(&self) {
        self.shared.lock().selection = None;
    }

    /// The selected text, for copying: lines and pages separated by
    /// newlines. `None` if nothing is selected or the selection holds no
    /// text. Reads every selected page, which takes a while for a long
    /// document selected as a whole.
    pub fn selected_text(&self) -> Option<String> {
        let mut state = self.shared.lock();
        let (_, mut texts, selection) = state.text();
        let text = selection.as_ref()?.text(&mut texts);
        (!text.is_empty()).then_some(text)
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
    /// page. Sessions start out [`ViewMode::Continuous`]. A change of mode
    /// also returns to the mode's own fit: the whole page when swiping, the
    /// width when scrolling (see [`set_fit_width`](Self::set_fit_width)).
    pub fn set_view_mode(&self, mode: ViewMode) {
        let mut state = self.shared.lock();
        let paged = mode == ViewMode::Paged;
        if paged != state.viewer.settings().swipe {
            state.fit = default_fit(paged);
        }
        let fit = state.fit;
        let viewer = &mut state.viewer;
        viewer.set_layout_mode(layout_for(paged, viewer.layout_mode().is_two_page()));
        viewer.set_swipe(paged);
        viewer.set_zoom_mode(fit);
    }

    /// Chooses what the view is fitted to: the page's width, with the rest
    /// of a taller page reached by scrolling, or the whole page. Also undoes
    /// any zooming. The choice holds until the view mode changes.
    pub fn set_fit_width(&self, fit_width: bool) {
        let mut state = self.shared.lock();
        state.fit = fit_mode(fit_width);
        let fit = state.fit;
        state.viewer.set_zoom_mode(fit);
    }

    /// Shows pages two side by side (or one at a time again), in either view
    /// mode, keeping the page. With `cover_alone`, the first page stands on
    /// its own, so that a book's left and right pages end up left and right.
    pub fn set_spread(&self, two_pages: bool, cover_alone: bool) {
        let mut state = self.shared.lock();
        let fit = state.fit;
        let viewer = &mut state.viewer;
        let layout = layout_for(viewer.settings().swipe, two_pages);
        if layout == viewer.layout_mode() && cover_alone == viewer.settings().cover_alone {
            return;
        }
        viewer.set_layout_mode(layout);
        viewer.set_cover_alone(cover_alone);
        viewer.set_zoom_mode(fit);
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
            fit_width: state.fit == pdfrs_view::ZoomMode::FitWidth,
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
        self.set_fit_width(saved.fit_width);
        let mut state = self.shared.lock();
        let fit = state.fit;
        let viewer = &mut state.viewer;
        let mut view = viewer.view_state();
        view.page = saved.page as usize;
        view.page_offset = saved.page_offset;
        match saved.zoom {
            Some(zoom) => {
                view.zoom_mode = pdfrs_view::ZoomMode::Fixed;
                view.zoom = zoom;
            }
            None => view.zoom_mode = fit,
        }
        viewer.restore_view_state(&view);
    }

    /// Back to the fitted view (after zooming in).
    pub fn fit(&self) {
        let mut state = self.shared.lock();
        let fit = state.fit;
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

    /// Goes to the next or previous page: the next pair with two pages, and
    /// the top of the neighbouring page when scrolling continuously. Nothing
    /// happens at either end of the document.
    pub fn turn_page(&self, forward: bool) {
        let mut state = self.shared.lock();
        if forward {
            state.viewer.next_page();
        } else {
            state.viewer.previous_page();
        }
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
