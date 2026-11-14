use pdfrs_geom::{
    DeviceIntRect, DevicePoint, DeviceRect, LayoutPoint, LayoutRect, LayoutVector, PageGeometry,
    PagePoint, PageSize, Rotation, ViewPoint, ViewRect, ViewSize, ViewVector,
};
use pdfrs_render::Content;
use std::time::{Duration, Instant};

use pdfrs_render::tile::{
    DocId, TILE_SIZE, TileKey, TileScale, level_for_scale, level_scale, tiles_covering,
};

use crate::layout::{Layout, LayoutMode, LayoutParams, group_spreads, layout, spread_points};

/// A page as the viewer sees it: which page of which document, and how it is
/// shown. In M3 these come from the edit model, so reordering or merging is
/// just a new list.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewPage {
    pub doc: DocId,
    pub page: u32,
    /// Geometry as stored in the file (its own `/Rotate` included).
    pub geometry: PageGeometry,
    /// Extra clockwise rotation (the user's "rotate page").
    pub rotation: Rotation,
}

impl ViewPage {
    /// Geometry as displayed.
    pub fn displayed(&self) -> PageGeometry {
        self.geometry.rotated(self.rotation)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ZoomMode {
    /// Zoom stays where the user put it.
    Fixed,
    /// The widest spread fills the viewport width.
    #[default]
    FitWidth,
    /// The largest spread fits entirely in the viewport.
    FitPage,
}

#[derive(Clone, Debug)]
pub struct Settings {
    /// Gap between spreads and between the pages of a spread, logical px.
    pub gap: f64,
    /// Space around the content, logical px.
    pub margin: f64,
    pub min_zoom: f64,
    pub max_zoom: f64,
    /// In two-page modes, show the first page on its own.
    pub cover_alone: bool,
    /// What tiles contain. Becomes `PageOnly` once annotations are drawn
    /// from the edit model (M4).
    pub content: Content,
    /// Prefetch tiles this many viewport heights above and below.
    pub prefetch: f64,
    /// How long the zoom must stay unchanged (and no zoom gesture be active)
    /// before the view counts as settled and switches from √2-level tiles to
    /// tiles at the exact device scale.
    pub settle_delay: Duration,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            gap: 12.0,
            margin: 12.0,
            min_zoom: 0.05,
            max_zoom: 32.0,
            cover_alone: false,
            content: Content::PageAndAnnotations,
            prefetch: 0.5,
            settle_delay: Duration::from_millis(250),
        }
    }
}

/// What to draw for one frame, in device pixels relative to the viewport.
#[derive(Clone, Debug, Default)]
pub struct Frame {
    pub pages: Vec<PageDraw>,
    /// Tiles to render, highest priority first. Pass to `Scheduler::request`.
    pub requests: Vec<TileKey>,
    /// Every visible tile is available at full sharpness.
    pub complete: bool,
    /// The view is still zooming (or just stopped) and is drawn from √2-level
    /// tiles. Once settled it switches to exact-scale tiles: redraw at this
    /// time even if nothing else happens.
    pub settles_at: Option<Instant>,
}

/// One visible page.
#[derive(Clone, Debug)]
pub struct PageDraw {
    pub page: usize,
    /// The page, snapped to whole device pixels. Fill it white, then draw
    /// `layers` in order, clipped to this rectangle.
    pub rect: DeviceRect,
    pub layers: Vec<TileDraw>,
}

#[derive(Clone, Copy, Debug)]
pub struct TileDraw {
    pub key: TileKey,
    pub dest: DeviceRect,
    /// The tile maps 1:1 onto device pixels: draw without filtering.
    pub exact: bool,
}

/// A content position that should stay put across relayouts (zoom, resize,
/// mode changes): a point within a spread, in points, and where it was on screen.
/// Points in a margin or gap are clamped to the spread's edge; the rest is
/// kept in logical px, since margins and gaps don't scale with zoom.
struct Anchor {
    spread: usize,
    rel: (f64, f64),
    outside: (f64, f64),
    view: ViewPoint,
}

/// UI-agnostic viewer state: layout, zoom and scrolling, plus planning which
/// tiles to draw and request. Frontends feed it input and draw its [`Frame`]s.
pub struct Viewer {
    pages: Vec<ViewPage>,
    sizes: Vec<PageSize>,
    settings: Settings,
    mode: LayoutMode,
    zoom_mode: ZoomMode,
    /// Logical px per point.
    zoom: f64,
    viewport: ViewSize,
    /// Device px per logical px.
    scale: f64,
    /// Layout position of the viewport's top-left corner.
    scroll: LayoutVector,
    spreads: Vec<Vec<usize>>,
    page_spread: Vec<usize>,
    /// Current spread in paged modes.
    current: usize,
    layout: Layout,
    /// Device scale of the last frame; a change restarts the settle delay.
    last_device_scale: Option<TileScale>,
    /// A zoom gesture ended since the last frame (restarts the settle delay).
    zoom_dirty: bool,
    /// When the device scale last changed, as seen by `frame`.
    zoom_changed_at: Option<Instant>,
    /// A zoom gesture (pinch) is in progress.
    gesture: bool,
    /// Whether `frame` has run yet. The first frame is settled right away:
    /// nothing is zooming, so there's no reason to render levels first.
    framed: bool,
}

impl Viewer {
    pub fn new(pages: Vec<ViewPage>, settings: Settings) -> Self {
        let mut v = Self {
            pages: Vec::new(),
            sizes: Vec::new(),
            settings,
            mode: LayoutMode::default(),
            zoom_mode: ZoomMode::default(),
            zoom: 1.0,
            viewport: ViewSize::new(800.0, 600.0),
            scale: 1.0,
            scroll: LayoutVector::zero(),
            spreads: Vec::new(),
            page_spread: Vec::new(),
            current: 0,
            layout: Layout::default(),
            last_device_scale: None,
            zoom_dirty: false,
            zoom_changed_at: None,
            gesture: false,
            framed: false,
        };
        v.set_pages(pages);
        v
    }

    // --- queries ------------------------------------------------------------

    pub fn pages(&self) -> &[ViewPage] {
        &self.pages
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn layout_mode(&self) -> LayoutMode {
        self.mode
    }

    pub fn zoom_mode(&self) -> ZoomMode {
        self.zoom_mode
    }

    /// Logical pixels per PDF point.
    pub fn zoom(&self) -> f64 {
        self.zoom
    }

    pub fn viewport(&self) -> ViewSize {
        self.viewport
    }

    pub fn scale_factor(&self) -> f64 {
        self.scale
    }

    /// Size of the laid-out content (for scrollbars).
    pub fn content_size(&self) -> pdfrs_geom::LayoutSize {
        self.layout.size
    }

    /// Layout position of the viewport's top-left corner (for scrollbars).
    pub fn scroll_offset(&self) -> LayoutVector {
        self.scroll
    }

    /// Where page `i` is on screen, or `None` if it isn't laid out (paged modes).
    pub fn page_rect(&self, i: usize) -> Option<ViewRect> {
        let r = (*self.layout.page_rects.get(i)?)?;
        Some(ViewRect::new(
            ViewPoint::new(r.origin.x - self.scroll.x, r.origin.y - self.scroll.y),
            ViewSize::new(r.size.width, r.size.height),
        ))
    }

    /// The page the user is looking at: in continuous modes the one with the
    /// largest visible area, in paged modes the first of the current spread.
    pub fn current_page(&self) -> usize {
        if !self.mode.is_continuous() {
            return self.spreads.get(self.current).map_or(0, |s| s[0]);
        }
        let view = self.view_layout_rect();
        self.visible_spreads(view.min_y(), view.max_y())
            .flat_map(|s| s.pages.iter().copied())
            .filter_map(|i| {
                let r = self.layout.page_rects[i]?;
                let area = r.intersection(&view).map_or(0.0, |r| r.area());
                Some((i, area))
            })
            .fold(None, |best: Option<(usize, f64)>, (i, a)| match best {
                Some((_, ba)) if ba >= a => best,
                _ => Some((i, a)),
            })
            .map_or(0, |(i, _)| i)
    }

    /// The page under `p` and the position on it, in page space (points).
    pub fn hit_test(&self, p: ViewPoint) -> Option<(usize, PagePoint)> {
        let lp = self.to_layout(p);
        let view = self.view_layout_rect();
        self.visible_spreads(view.min_y(), view.max_y())
            .flat_map(|s| s.pages.iter().copied())
            .find_map(|i| {
                let r = self.layout.page_rects[i]?;
                r.contains(lp).then(|| {
                    (
                        i,
                        PagePoint::new(
                            (lp.x - r.origin.x) / self.zoom,
                            (lp.y - r.origin.y) / self.zoom,
                        ),
                    )
                })
            })
    }

    /// Where a point on page `i` is on screen.
    pub fn page_to_view(&self, i: usize, p: PagePoint) -> Option<ViewPoint> {
        let r = self.page_rect(i)?;
        Some(ViewPoint::new(
            r.origin.x + p.x * self.zoom,
            r.origin.y + p.y * self.zoom,
        ))
    }

    // --- changes ------------------------------------------------------------

    /// Replaces the page list (after reorder, delete, merge, rotate), trying
    /// to keep the current page in view.
    pub fn set_pages(&mut self, pages: Vec<ViewPage>) {
        let keep = self.current_page();
        self.sizes = pages.iter().map(|p| p.displayed().size()).collect();
        self.pages = pages;
        self.regroup();
        if !self.pages.is_empty() {
            self.go_to_page(keep.min(self.pages.len() - 1));
        } else {
            self.relayout(None);
        }
    }

    /// Viewport size in logical pixels and the device scale factor
    /// (e.g. 2.0 on a Retina display, 1.25 on a fractionally scaled one).
    pub fn set_viewport(&mut self, size: ViewSize, scale_factor: f64) {
        let anchor = self.anchor_at(ViewPoint::new(self.viewport.width / 2.0, 0.0));
        self.viewport = size;
        self.scale = scale_factor.max(0.1);
        self.relayout(anchor.map(|a| Anchor {
            view: ViewPoint::new(size.width / 2.0, 0.0),
            ..a
        }));
    }

    pub fn set_layout_mode(&mut self, mode: LayoutMode) {
        if mode == self.mode {
            return;
        }
        let page = self.current_page();
        self.mode = mode;
        self.regroup();
        self.go_to_page(page);
    }

    pub fn set_cover_alone(&mut self, cover_alone: bool) {
        if cover_alone == self.settings.cover_alone {
            return;
        }
        let page = self.current_page();
        self.settings.cover_alone = cover_alone;
        self.regroup();
        self.go_to_page(page);
    }

    pub fn set_zoom_mode(&mut self, mode: ZoomMode) {
        self.zoom_mode = mode;
        let anchor = self.anchor_at(ViewPoint::new(self.viewport.width / 2.0, 0.0));
        self.relayout(anchor);
    }

    /// Sets a fixed zoom, keeping the content under `anchor` in place.
    pub fn set_zoom(&mut self, zoom: f64, anchor: ViewPoint) {
        let a = self.anchor_at(anchor);
        self.zoom_mode = ZoomMode::Fixed;
        self.zoom = zoom.clamp(self.settings.min_zoom, self.settings.max_zoom);
        self.relayout(a);
    }

    /// Multiplies the zoom (pinch, ctrl+scroll), keeping `anchor` in place.
    pub fn zoom_by(&mut self, factor: f64, anchor: ViewPoint) {
        self.set_zoom(self.zoom * factor, anchor);
    }

    /// A zoom gesture (pinch) started: until [`end_zoom_gesture`](Self::end_zoom_gesture)
    /// the view never counts as settled, so pausing mid-pinch doesn't
    /// render (and cache) exact-scale tiles for an intermediate zoom.
    pub fn begin_zoom_gesture(&mut self) {
        self.gesture = true;
    }

    /// The zoom gesture ended; the view settles after `settle_delay`.
    pub fn end_zoom_gesture(&mut self) {
        self.gesture = false;
        self.zoom_dirty = true;
    }

    /// Whether a frame planned at `now` would use exact-scale tiles.
    pub fn is_settled(&self, now: Instant) -> bool {
        !self.gesture
            && !self.zoom_dirty
            && self
                .last_device_scale
                .is_none_or(|s| s == self.device_scale())
            && self
                .zoom_changed_at
                .is_none_or(|t| now.saturating_duration_since(t) >= self.settings.settle_delay)
    }

    /// Device pixels per point actually used for drawing: zoom × device
    /// scale, quantized like tile scales so exact tiles map 1:1.
    pub fn device_scale(&self) -> TileScale {
        TileScale::exact(self.zoom * self.scale)
    }

    /// Sets the layout position of the viewport's top-left corner (scrollbars).
    pub fn set_scroll_offset(&mut self, offset: LayoutVector) {
        self.scroll = offset;
        self.clamp_scroll();
    }

    /// The low-resolution whole-page tile for page `i`, shown while sharp
    /// tiles load. Also suitable for thumbnails, so both share one render.
    pub fn placeholder_key(&self, i: usize) -> Option<TileKey> {
        let page = self.pages.get(i)?;
        Some(TileKey {
            doc: page.doc,
            page: page.page,
            rotation: page.rotation,
            content: self.settings.content,
            scale: TileScale::from_level(placeholder_level(&page.displayed())?),
            tx: 0,
            ty: 0,
        })
    }

    pub fn scroll_by(&mut self, delta: ViewVector) {
        self.scroll += LayoutVector::new(delta.x, delta.y);
        self.clamp_scroll();
    }

    /// Shows page `i` at the top of the viewport.
    pub fn go_to_page(&mut self, i: usize) {
        let Some(&spread) = self.page_spread.get(i) else {
            return;
        };
        self.current = spread;
        self.relayout(None);
        if let Some(r) = self.layout.page_rects[i] {
            let spread_top = self.layout.spreads[spread - self.layout.first_spread]
                .rect
                .origin
                .y;
            self.scroll.y = spread_top.min(r.origin.y) - self.settings.margin;
        }
        self.clamp_scroll();
    }

    pub fn next_page(&mut self) {
        let spread = self
            .page_spread
            .get(self.current_page())
            .copied()
            .unwrap_or(0);
        if let Some(s) = self.spreads.get(spread + 1) {
            self.go_to_page(s[0]);
        }
    }

    pub fn previous_page(&mut self) {
        let spread = self
            .page_spread
            .get(self.current_page())
            .copied()
            .unwrap_or(0);
        if spread > 0 {
            self.go_to_page(self.spreads[spread - 1][0]);
        }
    }

    // --- drawing ------------------------------------------------------------

    /// Plans a frame at time `now`. `cached` tells whether the frontend has a tile.
    ///
    /// While zooming, pages are drawn from √2-level tiles (rendered at or
    /// above the device scale and downsampled), so continuous zooming reuses
    /// a few cached levels. Once the view is settled (no zoom change for
    /// `settle_delay` and no gesture in progress), it requests tiles at the
    /// exact device scale and draws them 1:1, with level tiles as fallback.
    pub fn frame(&mut self, now: Instant, cached: impl Fn(&TileKey) -> bool) -> Frame {
        let device = self.device_scale();
        let rescaled = self.last_device_scale.replace(device) != Some(device);
        if (std::mem::take(&mut self.zoom_dirty) || rescaled) && self.framed {
            self.zoom_changed_at = Some(now);
        }
        self.framed = true;
        let settled = self.is_settled(now);
        let level = level_for_scale(device.get());
        let level_scale = TileScale::from_level(level);
        // Exact tiles only when they differ from the level (they're the same
        // key when the zoom lands on a level, e.g. 100% at scale 2).
        let target = if settled { device } else { level_scale };
        let fallbacks: Vec<TileScale> = if target == level_scale {
            [level - 2, level - 1, level + 1, level + 2]
                .map(TileScale::from_level)
                .to_vec()
        } else {
            [level - 2, level - 1, level, level + 1]
                .map(TileScale::from_level)
                .to_vec()
        };

        let view = self.view_layout_rect();
        let band = self.settings.prefetch * self.viewport.height;
        let viewport_dev = DeviceRect::new(
            DevicePoint::zero(),
            (
                self.viewport.width * self.scale,
                self.viewport.height * self.scale,
            )
                .into(),
        );

        let mut frame = Frame {
            complete: true,
            settles_at: match self.zoom_changed_at {
                Some(t) if !settled && !self.gesture && device != level_scale => {
                    Some(t + self.settings.settle_delay)
                }
                _ => None,
            },
            ..Frame::default()
        };
        let mut missing_sharp = Vec::new();
        let mut prefetch = Vec::new();
        let mut visible_pages = Vec::new();

        for spread in self.visible_spreads(view.min_y() - band, view.max_y() + band) {
            for &i in &spread.pages {
                let Some(r) = self.layout.page_rects[i] else {
                    continue;
                };
                let page = &self.pages[i];
                let displayed = page.displayed();
                let origin = DevicePoint::new(
                    ((r.origin.x - self.scroll.x) * self.scale).round(),
                    ((r.origin.y - self.scroll.y) * self.scale).round(),
                );
                // Sized from the quantized device scale, exactly like an
                // exact-scale page bitmap.
                let px = displayed.pixel_size(device.get());
                let rect = DeviceRect::new(origin, (px.width as f64, px.height as f64).into());
                let placeholder = placeholder_level(&displayed)
                    .map(TileScale::from_level)
                    .filter(|&p| p < target);
                let key = |scale: TileScale, (tx, ty): (u32, u32)| TileKey {
                    doc: page.doc,
                    page: page.page,
                    rotation: page.rotation,
                    content: self.settings.content,
                    scale,
                    tx,
                    ty,
                };

                let Some(on_screen) = rect.intersection(&viewport_dev) else {
                    // In the prefetch band only.
                    let band_dev = viewport_dev.inflate(0.0, band * self.scale);
                    if let Some(area) = rect.intersection(&band_dev) {
                        if let Some(p) = placeholder
                            && !cached(&key(p, (0, 0)))
                        {
                            prefetch.insert(0, key(p, (0, 0)));
                        }
                        let a = scaled_area(rect, area, &displayed, target);
                        for t in tiles_covering(&displayed, target, a) {
                            if !cached(&key(target, t)) {
                                prefetch.push(key(target, t));
                            }
                        }
                    }
                    continue;
                };
                visible_pages.push(i);

                let mut layers = Vec::new();
                if let Some(p) = placeholder {
                    let k = key(p, (0, 0));
                    if cached(&k) {
                        layers.extend(tile_draw(k, page, rect));
                    } else {
                        frame.requests.push(k);
                    }
                }

                let area = scaled_area(rect, on_screen, &displayed, target);
                let mut sharp = Vec::new();
                let mut missing = false;
                for t in tiles_covering(&displayed, target, area) {
                    let k = key(target, t);
                    if cached(&k) {
                        sharp.push(k);
                    } else {
                        missing = true;
                        missing_sharp.push(k);
                    }
                }
                if missing {
                    frame.complete = false;
                    // Fill gaps with whatever neighbouring levels are cached,
                    // coarser first so finer ones end up on top.
                    for &f in &fallbacks {
                        if placeholder.is_some_and(|p| f <= p) {
                            continue;
                        }
                        let a = scaled_area(rect, on_screen, &displayed, f);
                        for t in tiles_covering(&displayed, f, a) {
                            let k = key(f, t);
                            if cached(&k) {
                                layers.extend(tile_draw(k, page, rect));
                            }
                        }
                    }
                }
                layers.extend(sharp.into_iter().filter_map(|k| tile_draw(k, page, rect)));
                frame.pages.push(PageDraw {
                    page: i,
                    rect,
                    layers,
                });
            }
        }

        // Placeholders for visible pages first (already queued), then sharp
        // visible tiles, then neighbouring placeholders and the prefetch band.
        frame.requests.extend(missing_sharp);
        if let (Some(&first), Some(&last)) = (visible_pages.first(), visible_pages.last()) {
            for i in first.saturating_sub(2)..(last + 3).min(self.pages.len()) {
                if let Some(k) = self.placeholder_key(i).filter(|k| k.scale < target)
                    && !cached(&k)
                    && !frame.requests.contains(&k)
                {
                    frame.requests.push(k);
                }
            }
        }
        frame.requests.extend(prefetch);
        // Drop duplicates (a placeholder can be both a neighbour and in the
        // prefetch band), keeping the first, highest-priority occurrence.
        let mut seen = std::collections::HashSet::new();
        frame.requests.retain(|k| seen.insert(*k));
        frame
    }

    // --- internals ----------------------------------------------------------

    fn regroup(&mut self) {
        self.spreads = group_spreads(
            self.pages.len(),
            self.mode.is_two_page(),
            self.settings.cover_alone,
        );
        self.page_spread = vec![0; self.pages.len()];
        for (s, pages) in self.spreads.iter().enumerate() {
            for &p in pages {
                self.page_spread[p] = s;
            }
        }
        self.current = self.current.min(self.spreads.len().saturating_sub(1));
    }

    fn fit_zoom(&self) -> Option<f64> {
        let spreads: &[Vec<usize>] = if self.mode.is_continuous() {
            &self.spreads
        } else {
            std::slice::from_ref(self.spreads.get(self.current)?)
        };
        let m = self.settings.margin;
        let mut zoom = f64::INFINITY;
        for s in spreads {
            let (w, h) = spread_points(s, &self.sizes);
            let gaps = self.settings.gap * (s.len().saturating_sub(1)) as f64;
            zoom = zoom.min((self.viewport.width - 2.0 * m - gaps) / w);
            if self.zoom_mode == ZoomMode::FitPage {
                zoom = zoom.min((self.viewport.height - 2.0 * m) / h);
            }
        }
        zoom.is_finite().then_some(zoom)
    }

    fn relayout(&mut self, anchor: Option<Anchor>) {
        if self.zoom_mode != ZoomMode::Fixed
            && let Some(z) = self.fit_zoom()
        {
            self.zoom = z;
        }
        self.zoom = self
            .zoom
            .clamp(self.settings.min_zoom, self.settings.max_zoom);
        let range = if self.mode.is_continuous() {
            0..self.spreads.len()
        } else if self.spreads.is_empty() {
            0..0
        } else {
            self.current..self.current + 1
        };
        self.layout = layout(&LayoutParams {
            sizes: &self.sizes,
            spreads: &self.spreads,
            range,
            zoom: self.zoom,
            gap: self.settings.gap,
            margin: self.settings.margin,
            viewport_width: self.viewport.width,
        });
        if let Some(a) = anchor
            && let Some(s) = a
                .spread
                .checked_sub(self.layout.first_spread)
                .and_then(|i| self.layout.spreads.get(i))
        {
            let lp = LayoutPoint::new(
                s.rect.origin.x + a.rel.0 * self.zoom + a.outside.0,
                s.rect.origin.y + a.rel.1 * self.zoom + a.outside.1,
            );
            self.scroll = LayoutVector::new(lp.x - a.view.x, lp.y - a.view.y);
        }
        self.clamp_scroll();
    }

    fn clamp_scroll(&mut self) {
        let size = self.layout.size;
        let (vw, vh) = (self.viewport.width, self.viewport.height);
        self.scroll.x = if size.width <= vw {
            0.0
        } else {
            self.scroll.x.clamp(0.0, size.width - vw)
        };
        // Content shorter than the viewport is centred vertically.
        self.scroll.y = if size.height <= vh {
            -(vh - size.height) / 2.0
        } else {
            self.scroll.y.clamp(0.0, size.height - vh)
        };
    }

    fn to_layout(&self, p: ViewPoint) -> LayoutPoint {
        LayoutPoint::new(p.x + self.scroll.x, p.y + self.scroll.y)
    }

    fn view_layout_rect(&self) -> LayoutRect {
        LayoutRect::new(
            LayoutPoint::new(self.scroll.x, self.scroll.y),
            (self.viewport.width, self.viewport.height).into(),
        )
    }

    /// Laid-out spreads overlapping the vertical range `[y0, y1]`.
    fn visible_spreads(
        &self,
        y0: f64,
        y1: f64,
    ) -> impl Iterator<Item = &crate::layout::LaidSpread> {
        let spreads = &self.layout.spreads;
        let first = spreads.partition_point(|s| s.rect.max_y() < y0);
        spreads[first..]
            .iter()
            .take_while(move |s| s.rect.origin.y <= y1)
    }

    fn anchor_at(&self, view: ViewPoint) -> Option<Anchor> {
        let lp = self.to_layout(view);
        let spreads = &self.layout.spreads;
        if spreads.is_empty() || self.zoom <= 0.0 {
            return None;
        }
        // The spread containing the point, or the next one below it (gaps).
        let i = spreads
            .partition_point(|s| s.rect.max_y() < lp.y)
            .min(spreads.len() - 1);
        let r = spreads[i].rect;
        let (dx, dy) = (lp.x - r.origin.x, lp.y - r.origin.y);
        let (cx, cy) = (dx.clamp(0.0, r.size.width), dy.clamp(0.0, r.size.height));
        Some(Anchor {
            spread: self.layout.first_spread + i,
            rel: (cx / self.zoom, cy / self.zoom),
            outside: (dx - cx, dy - cy),
            view,
        })
    }
}

/// The largest level at which the whole page fits in one tile, used for the
/// blurry placeholder shown before sharp tiles arrive. Capped so tiny pages
/// don't get large placeholders.
pub fn placeholder_level(g: &PageGeometry) -> Option<i32> {
    let s = g.size();
    let max = s.width.max(s.height);
    if max <= 0.0 {
        return None;
    }
    let mut l = ((TILE_SIZE as f64 / max).log2() * 2.0).floor() as i32;
    while l > -40 {
        let px = g.pixel_size(level_scale(l));
        if px.width <= TILE_SIZE && px.height <= TILE_SIZE {
            return Some(l.min(-2));
        }
        l -= 1;
    }
    None
}

/// Converts part of a page's device rectangle into pixels of the page bitmap
/// at `scale`, rounded outwards.
fn scaled_area(
    page: DeviceRect,
    part: DeviceRect,
    g: &PageGeometry,
    scale: TileScale,
) -> DeviceIntRect {
    let full = g.pixel_size(scale.get());
    let fx = full.width as f64 / page.size.width;
    let fy = full.height as f64 / page.size.height;
    let x0 = ((part.min_x() - page.min_x()) * fx).floor().max(0.0);
    let y0 = ((part.min_y() - page.min_y()) * fy).floor().max(0.0);
    let x1 = ((part.max_x() - page.min_x()) * fx).ceil().max(0.0);
    let y1 = ((part.max_y() - page.min_y()) * fy).ceil().max(0.0);
    DeviceIntRect::new((x0 as u32, y0 as u32).into(), (x1 as u32, y1 as u32).into())
}

/// Where a tile lands on screen, given the page's device rectangle.
fn tile_draw(key: TileKey, page: &ViewPage, rect: DeviceRect) -> Option<TileDraw> {
    let region = key.region(&page.geometry)?;
    let full = page.displayed().pixel_size(key.scale.get());
    let fx = rect.size.width / full.width as f64;
    let fy = rect.size.height / full.height as f64;
    Some(TileDraw {
        key,
        dest: DeviceRect::new(
            DevicePoint::new(
                rect.origin.x + region.min.x as f64 * fx,
                rect.origin.y + region.min.y as f64 * fy,
            ),
            (region.width() as f64 * fx, region.height() as f64 * fy).into(),
        ),
        // Page origins are whole device pixels, so scale 1 means 1:1.
        exact: fx == 1.0 && fy == 1.0,
    })
}
