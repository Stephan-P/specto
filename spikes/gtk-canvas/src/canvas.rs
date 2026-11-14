//! `PdfCanvas`: a custom widget that lays pages out continuously and draws cached tiles.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib, graphene, gsk};
use pdfrs_render::scheduler::Scheduler;
use pdfrs_render::tile::{DocId, TileCache, TileKey, level_for_scale, level_scale, tiles_covering};
use pdfrs_render::{Content, DeviceIntRect, PageGeometry, Rotation};

pub const GAP: f64 = 12.0;
/// Placeholder level: scale 2^(-4/2) = 0.25 → a whole page is one small tile.
pub const THUMB_LEVEL: i32 = -4;
const CACHE_BUDGET: usize = 384 << 20;

#[derive(Default)]
pub struct Stats {
    pub frames: u64,
    pub snapshot_ms: Vec<f64>,
    /// Frames in which at least one visible tile was a placeholder or missing.
    pub blurry_frames: u64,
    pub tiles_rendered: u64,
    pub tile_ms: Vec<f64>,
}

pub struct State {
    pub pages: Vec<PageGeometry>,
    /// Top of each page in points (excluding gaps), prefix sums.
    tops: Vec<f64>,
    /// Logical px per point.
    pub zoom: f64,
    pub scroll_x: f64,
    pub scroll_y: f64,
    pub cache: TileCache<gdk::Texture>,
    pub scheduler: Scheduler,
    pub doc: DocId,
    pub stats: Stats,
}

impl State {
    pub fn new(scheduler: Scheduler, doc: DocId, pages: Vec<PageGeometry>) -> Self {
        let mut tops = Vec::with_capacity(pages.len());
        let mut y = 0.0;
        for g in &pages {
            tops.push(y);
            y += g.size().height;
        }
        Self {
            pages,
            tops,
            zoom: 1.0,
            scroll_x: 0.0,
            scroll_y: 0.0,
            cache: TileCache::new(CACHE_BUDGET).with_pinned(|k| k.level == THUMB_LEVEL),
            scheduler,
            doc,
            stats: Stats::default(),
        }
    }

    fn page_top(&self, i: usize) -> f64 {
        GAP + self.tops[i] * self.zoom + i as f64 * GAP
    }

    fn page_height(&self, i: usize) -> f64 {
        self.pages[i].size().height * self.zoom
    }

    pub fn max_page_width(&self) -> f64 {
        self.pages
            .iter()
            .map(|g| g.size().width)
            .fold(0.0, f64::max)
    }

    fn content_height(&self) -> f64 {
        match self.pages.len() {
            0 => 0.0,
            n => self.page_top(n - 1) + self.page_height(n - 1) + GAP,
        }
    }

    fn content_width(&self) -> f64 {
        self.max_page_width() * self.zoom + 2.0 * GAP
    }

    /// First page whose bottom is below `y`.
    fn first_visible(&self, y: f64) -> usize {
        let (mut lo, mut hi) = (0, self.pages.len());
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.page_top(mid) + self.page_height(mid) < y {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo
    }

    pub fn clamp_scroll(&mut self, view_w: f64, view_h: f64) {
        let max_y = (self.content_height() - view_h).max(0.0);
        self.scroll_y = self.scroll_y.clamp(0.0, max_y);
        let max_x = (self.content_width() - view_w).max(0.0);
        self.scroll_x = self.scroll_x.clamp(0.0, max_x);
    }

    /// Zoom by `factor`, keeping the content point under (`ax`, `ay`) fixed.
    pub fn zoom_at(&mut self, factor: f64, ax: f64, ay: f64, view_w: f64, view_h: f64) {
        let old = self.zoom;
        let new = (old * factor).clamp(0.1, 16.0);
        let cy = (self.scroll_y + ay) / old;
        let cx = (self.scroll_x + ax) / old;
        self.zoom = new;
        self.scroll_y = cy * new - ay;
        self.scroll_x = cx * new - ax;
        self.clamp_scroll(view_w, view_h);
    }

    pub fn key(&self, page: usize, level: i32, tx: u32, ty: u32) -> TileKey {
        TileKey {
            doc: self.doc,
            page: page as u32,
            rotation: Rotation::None,
            content: Content::PageAndAnnotations,
            level,
            tx,
            ty,
        }
    }
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct PdfCanvas {
        pub state: RefCell<Option<Rc<RefCell<State>>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PdfCanvas {
        const NAME: &'static str = "PdfrsCanvas";
        type Type = super::PdfCanvas;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for PdfCanvas {}

    impl WidgetImpl for PdfCanvas {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let Some(state) = self.state.borrow().clone() else {
                return;
            };
            let widget = self.obj();
            let start = Instant::now();
            let (vw, vh) = (widget.width() as f64, widget.height() as f64);
            let device_scale = widget
                .native()
                .and_then(|n| n.surface())
                .map_or(1.0, |s| s.scale());
            draw(&mut state.borrow_mut(), snapshot, vw, vh, device_scale);
            let mut s = state.borrow_mut();
            s.stats
                .snapshot_ms
                .push(start.elapsed().as_secs_f64() * 1000.0);
            s.stats.frames += 1;
        }
    }
}

glib::wrapper! {
    pub struct PdfCanvas(ObjectSubclass<imp::PdfCanvas>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl PdfCanvas {
    pub fn new(state: Rc<RefCell<State>>) -> Self {
        let obj: Self = glib::Object::new();
        obj.set_hexpand(true);
        obj.set_vexpand(true);
        obj.set_overflow(gtk::Overflow::Hidden);
        obj.set_focusable(true);
        *obj.imp().state.borrow_mut() = Some(state);
        obj
    }
}

/// A rectangle given in device pixels, converted to the logical coordinates
/// GTK snapshots use. Keeping positions on whole device pixels means a tile
/// rendered at exactly the device scale is drawn 1:1, without filtering.
fn device_rect(x: f64, y: f64, w: f64, h: f64, scale: f64) -> graphene::Rect {
    graphene::Rect::new(
        (x / scale) as f32,
        (y / scale) as f32,
        (w / scale) as f32,
        (h / scale) as f32,
    )
}

fn draw(s: &mut State, snap: &gtk::Snapshot, vw: f64, vh: f64, scale: f64) {
    s.cache.begin_frame();
    s.clamp_scroll(vw, vh);
    snap.append_color(
        &gdk::RGBA::new(0.85, 0.85, 0.86, 1.0),
        &graphene::Rect::new(0.0, 0.0, vw as f32, vh as f32),
    );

    // Device pixels per point, and the tile level to render at (rounded up).
    let dev_zoom = s.zoom * scale;
    let level = level_for_scale(dev_zoom);
    let (view_dev_w, view_dev_h) = (vw * scale, vh * scale);

    let mut wanted = Vec::new();
    let mut blurry = false;
    let content_w = s.content_width();
    let first = s.first_visible(s.scroll_y);
    let mut i = first;
    while i < s.pages.len() {
        let g = s.pages[i];
        let top_dev = ((s.page_top(i) - s.scroll_y) * scale).round();
        if top_dev > view_dev_h {
            break;
        }
        let page_px = g.pixel_size(dev_zoom);
        let (pw, ph) = (page_px.width as f64, page_px.height as f64);
        let left_dev =
            (((content_w.max(vw) - g.size().width * s.zoom) / 2.0 - s.scroll_x) * scale).round();
        let page_rect = device_rect(left_dev, top_dev, pw, ph, scale);

        snap.append_color(
            &gdk::RGBA::new(0.0, 0.0, 0.0, 0.15),
            &device_rect(
                left_dev - scale,
                top_dev - scale,
                pw + 2.0 * scale,
                ph + 2.0 * scale,
                scale,
            ),
        );
        snap.append_color(&gdk::RGBA::WHITE, &page_rect);

        // Placeholder: low-res render of the page, stretched.
        let thumb = s.key(i, THUMB_LEVEL, 0, 0);
        if let Some(t) = s.cache.get(&thumb) {
            let full = g.pixel_size(level_scale(THUMB_LEVEL));
            let f = pw / full.width as f64;
            snap.append_scaled_texture(
                t,
                gsk::ScalingFilter::Linear,
                &device_rect(
                    left_dev,
                    top_dev,
                    t.width() as f64 * f,
                    t.height() as f64 * f,
                    scale,
                ),
            );
        } else {
            wanted.push(thumb);
        }

        // Sharp tiles intersecting the viewport.
        if level > THUMB_LEVEL {
            let full = g.pixel_size(level_scale(level));
            // Device px per tile px (≤ 1: tiles are only ever downsampled).
            let f = pw / full.width as f64;
            let to_tile = |d: f64| (d / f).max(0.0) as u32;
            let visible = DeviceIntRect::new(
                (to_tile(-left_dev), to_tile(-top_dev)).into(),
                (
                    to_tile(view_dev_w - left_dev).saturating_add(1),
                    to_tile(view_dev_h - top_dev).saturating_add(1),
                )
                    .into(),
            );
            snap.push_clip(&page_rect);
            for (tx, ty) in tiles_covering(&g, level, visible).collect::<Vec<_>>() {
                let key = s.key(i, level, tx, ty);
                let Some(region) = key.region(&g) else {
                    continue;
                };
                if let Some(t) = s.cache.get(&key) {
                    let r = device_rect(
                        left_dev + region.min.x as f64 * f,
                        top_dev + region.min.y as f64 * f,
                        region.width() as f64 * f,
                        region.height() as f64 * f,
                        scale,
                    );
                    // Exactly 1:1 → draw untouched. Otherwise f ∈ (0.71, 1): plain
                    // linear is fine and avoids GSK building mipmaps per tile.
                    let filter = if (f - 1.0).abs() < 1e-9 {
                        gsk::ScalingFilter::Nearest
                    } else {
                        gsk::ScalingFilter::Linear
                    };
                    snap.append_scaled_texture(t, filter, &r);
                } else {
                    blurry = true;
                    wanted.push(key);
                }
            }
            snap.pop();
        }
        i += 1;
    }

    // Placeholders first (cheap, they fill the screen), then sharp tiles,
    // then placeholders for a few pages around the viewport.
    wanted.sort_by_key(|k| k.level != THUMB_LEVEL);
    for p in first.saturating_sub(3)..(i + 3).min(s.pages.len()) {
        let k = s.key(p, THUMB_LEVEL, 0, 0);
        if !s.cache.contains(&k) {
            wanted.push(k);
        }
    }
    if blurry {
        s.stats.blurry_frames += 1;
    }
    s.scheduler.request(wanted);
}
