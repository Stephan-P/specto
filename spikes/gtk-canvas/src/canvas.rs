//! `PdfCanvas`: a custom widget that lays pages out continuously and draws cached tiles.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib, graphene, gsk};

use crate::workers::{Pool, TILE, TileKey, level_scale, scale_level};

pub const GAP: f64 = 12.0;
/// Placeholder level: scale 2^(-4/2) = 0.25 → a whole page is one small tile.
pub const THUMB_LEVEL: i32 = -4;
const CACHE_BUDGET: usize = 384 << 20;

pub struct Cached {
    texture: gdk::Texture,
    bytes: usize,
    last_used: u64,
}

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
    /// Page sizes in points.
    pub pages: Vec<(f64, f64)>,
    /// Top of each page in points (excluding gaps), prefix sums.
    tops: Vec<f64>,
    /// Logical px per point.
    pub zoom: f64,
    pub scroll_x: f64,
    pub scroll_y: f64,
    tiles: HashMap<TileKey, Cached>,
    cache_bytes: usize,
    frame: u64,
    pub pool: Option<Pool>,
    pub stats: Stats,
}

impl State {
    pub fn new(pages: Vec<(f64, f64)>) -> Self {
        let mut tops = Vec::with_capacity(pages.len());
        let mut y = 0.0;
        for &(_, h) in &pages {
            tops.push(y);
            y += h;
        }
        Self {
            pages,
            tops,
            zoom: 1.0,
            scroll_x: 0.0,
            scroll_y: 0.0,
            tiles: HashMap::new(),
            cache_bytes: 0,
            frame: 0,
            pool: None,
            stats: Stats::default(),
        }
    }

    pub fn page_top(&self, i: usize) -> f64 {
        GAP + self.tops[i] * self.zoom + i as f64 * GAP
    }

    pub fn content_height(&self) -> f64 {
        let n = self.pages.len();
        if n == 0 {
            return 0.0;
        }
        self.page_top(n - 1) + self.pages[n - 1].1 * self.zoom + GAP
    }

    pub fn content_width(&self) -> f64 {
        self.pages.iter().map(|p| p.0).fold(0.0, f64::max) * self.zoom + 2.0 * GAP
    }

    /// First page whose bottom is below `y`.
    fn first_visible(&self, y: f64) -> usize {
        self.pages
            .partition_point_by(|i| self.page_top(i) + self.pages[i].1 * self.zoom < y)
    }

    pub fn clamp_scroll(&mut self, view_w: f64, view_h: f64) {
        let max_y = (self.content_height() - view_h).max(0.0);
        self.scroll_y = self.scroll_y.clamp(0.0, max_y);
        let max_x = (self.content_width() - view_w).max(0.0);
        self.scroll_x = if max_x == 0.0 {
            0.0
        } else {
            self.scroll_x.clamp(0.0, max_x)
        };
    }

    /// Zoom by `factor`, keeping the content point under (`ax`, `ay`) fixed.
    pub fn zoom_at(&mut self, factor: f64, ax: f64, ay: f64, view_w: f64, view_h: f64) {
        let old = self.zoom;
        let new = (old * factor).clamp(0.1, 16.0);
        // Content coordinates in points (approximately; ignores gaps scaling).
        let cy = (self.scroll_y + ay) / old;
        let cx = (self.scroll_x + ax) / old;
        self.zoom = new;
        self.scroll_y = cy * new - ay;
        self.scroll_x = cx * new - ax;
        self.clamp_scroll(view_w, view_h);
    }

    pub fn insert(&mut self, key: TileKey, texture: gdk::Texture, bytes: usize) {
        self.cache_bytes += bytes;
        let frame = self.frame;
        if let Some(old) = self.tiles.insert(
            key,
            Cached {
                texture,
                bytes,
                last_used: frame,
            },
        ) {
            self.cache_bytes -= old.bytes;
        }
        self.evict();
    }

    fn evict(&mut self) {
        if self.cache_bytes <= CACHE_BUDGET {
            return;
        }
        let mut by_age: Vec<(u64, TileKey)> = self
            .tiles
            .iter()
            // Never evict placeholders, and nothing drawn in the current frame.
            .filter(|(k, c)| k.level != THUMB_LEVEL && c.last_used < self.frame)
            .map(|(k, c)| (c.last_used, *k))
            .collect();
        by_age.sort_unstable();
        for (_, k) in by_age {
            if self.cache_bytes <= CACHE_BUDGET * 3 / 4 {
                break;
            }
            if let Some(c) = self.tiles.remove(&k) {
                self.cache_bytes -= c.bytes;
            }
        }
    }
}

trait PartitionPointBy {
    fn partition_point_by(&self, pred: impl Fn(usize) -> bool) -> usize;
}

impl<T> PartitionPointBy for Vec<T> {
    fn partition_point_by(&self, pred: impl Fn(usize) -> bool) -> usize {
        let (mut lo, mut hi) = (0, self.len());
        while lo < hi {
            let mid = (lo + hi) / 2;
            if pred(mid) {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo
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

fn rect(x: f64, y: f64, w: f64, h: f64) -> graphene::Rect {
    graphene::Rect::new(x as f32, y as f32, w as f32, h as f32)
}

fn draw(s: &mut State, snap: &gtk::Snapshot, vw: f64, vh: f64, device_scale: f64) {
    s.frame += 1;
    let frame = s.frame;
    s.clamp_scroll(vw, vh);

    snap.append_color(
        &gdk::RGBA::new(0.85, 0.85, 0.86, 1.0),
        &rect(0.0, 0.0, vw, vh),
    );

    let level = scale_level(s.zoom * device_scale);
    let lscale = level_scale(level);
    let mut wanted = Vec::new();
    let mut prefetch = Vec::new();
    let mut blurry = false;

    let content_w = s.content_width();
    let first = s.first_visible(s.scroll_y);
    let mut i = first;
    while i < s.pages.len() {
        let (pw, ph) = s.pages[i];
        let top = s.page_top(i) - s.scroll_y;
        if top > vh {
            break;
        }
        let (lw, lh) = (pw * s.zoom, ph * s.zoom);
        let left = ((content_w.max(vw) - lw) / 2.0 - s.scroll_x).round();
        let page_rect = rect(left, top.round(), lw, lh);

        // Shadow-ish border + white page.
        snap.append_color(
            &gdk::RGBA::new(0.0, 0.0, 0.0, 0.15),
            &rect(left - 1.0, top - 1.0, lw + 2.0, lh + 2.0),
        );
        snap.append_color(&gdk::RGBA::WHITE, &page_rect);

        // Placeholder: low-res whole page, stretched.
        let thumb = TileKey {
            page: i as u32,
            level: THUMB_LEVEL,
            tx: 0,
            ty: 0,
        };
        if let Some(c) = s.tiles.get_mut(&thumb) {
            c.last_used = frame;
            snap.append_scaled_texture(&c.texture, gsk::ScalingFilter::Linear, &page_rect);
        } else {
            wanted.push(thumb);
        }

        // Sharp tiles at the current level, only those intersecting the viewport.
        let (full_w, full_h) = (
            (pw * lscale).round().max(1.0) as u32,
            (ph * lscale).round().max(1.0) as u32,
        );
        // logical px per tile px
        let (fx, fy) = (lw / full_w as f64, lh / full_h as f64);
        let vis_x0 = ((-left).max(0.0) / fx) as u32;
        let vis_x1 = (((vw - left) / fx).max(0.0) as u32).min(full_w);
        let vis_y0 = ((-top).max(0.0) / fy) as u32;
        let vis_y1 = (((vh - top) / fy).max(0.0) as u32).min(full_h);
        if level != THUMB_LEVEL && vis_x1 > vis_x0 && vis_y1 > vis_y0 {
            snap.push_clip(&page_rect);
            for ty in vis_y0 / TILE..=(vis_y1 - 1) / TILE {
                for tx in vis_x0 / TILE..=(vis_x1 - 1) / TILE {
                    let key = TileKey {
                        page: i as u32,
                        level,
                        tx,
                        ty,
                    };
                    let tw = TILE.min(full_w - tx * TILE) as f64;
                    let th = TILE.min(full_h - ty * TILE) as f64;
                    let r = rect(
                        left + (tx * TILE) as f64 * fx,
                        top.round() + (ty * TILE) as f64 * fy,
                        tw * fx,
                        th * fy,
                    );
                    if let Some(c) = s.tiles.get_mut(&key) {
                        c.last_used = frame;
                        snap.append_scaled_texture(&c.texture, gsk::ScalingFilter::Linear, &r);
                    } else {
                        blurry = true;
                        wanted.push(key);
                    }
                }
            }
            snap.pop();
        }
        i += 1;
    }
    // Prefetch placeholders for a few pages around the viewport.
    for p in first.saturating_sub(3)..(i + 3).min(s.pages.len()) {
        let k = TileKey {
            page: p as u32,
            level: THUMB_LEVEL,
            tx: 0,
            ty: 0,
        };
        if !s.tiles.contains_key(&k) && !wanted.contains(&k) {
            prefetch.push(k);
        }
    }
    // Placeholders first (cheap, they fill the screen), then sharp tiles.
    wanted.sort_by_key(|k| k.level != THUMB_LEVEL);
    wanted.extend(prefetch);
    if blurry {
        s.stats.blurry_frames += 1;
    }
    if let Some(pool) = &s.pool {
        pool.set_wanted(wanted);
    }
}
