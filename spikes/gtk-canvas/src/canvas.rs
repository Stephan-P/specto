//! `PdfCanvas`: a custom widget that draws `pdfrs_view::Frame`s.
//!
//! All layout, zoom and tile planning lives in `pdfrs_view::Viewer`; this
//! widget only forwards its size and draws rectangles.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib, graphene};
use pdfrs_geom::{DeviceRect, ViewSize};
use pdfrs_render::scheduler::Scheduler;
use pdfrs_render::tile::TileCache;
use pdfrs_view::{Viewer, placeholder_level};

const CACHE_BUDGET: usize = 384 << 20;

#[derive(Default)]
pub struct Stats {
    pub frames: u64,
    pub snapshot_ms: Vec<f64>,
    /// Frames in which some visible tile wasn't sharp yet.
    pub blurry_frames: u64,
    pub tiles_rendered: u64,
    pub tile_ms: Vec<f64>,
}

pub struct State {
    pub viewer: Viewer,
    pub cache: TileCache<gdk::Texture>,
    pub scheduler: Scheduler,
    pub stats: Stats,
    pub settle_redraw_pending: bool,
}

impl State {
    pub fn new(scheduler: Scheduler, viewer: Viewer) -> Self {
        // Keep placeholders: they are tiny and make scrolling back instant.
        let placeholders: std::collections::HashSet<_> = viewer
            .pages()
            .iter()
            .filter_map(|p| placeholder_level(&p.displayed()))
            .collect();
        Self {
            viewer,
            cache: TileCache::new(CACHE_BUDGET)
                .with_pinned(move |k| k.scale.level().is_some_and(|l| placeholders.contains(&l))),
            scheduler,
            stats: Stats::default(),
            settle_redraw_pending: false,
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
            let size = ViewSize::new(widget.width() as f64, widget.height() as f64);
            let scale = widget
                .native()
                .and_then(|n| n.surface())
                .map_or(1.0, |s| s.scale());
            let (dx, dy) = device_pixel_snap(&*widget, scale);
            snapshot.save();
            snapshot.translate(&graphene::Point::new(dx, dy));
            let settles_at = draw(&mut state.borrow_mut(), snapshot, size, scale);
            snapshot.restore();
            let mut s = state.borrow_mut();
            // Redraw once the view settles, to switch to exact-scale tiles.
            if let Some(at) = settles_at
                && !s.settle_redraw_pending
            {
                s.settle_redraw_pending = true;
                let (widget, state) = (widget.downgrade(), state.clone());
                glib::timeout_add_local_once(
                    at.saturating_duration_since(Instant::now()),
                    move || {
                        state.borrow_mut().settle_redraw_pending = false;
                        if let Some(w) = widget.upgrade() {
                            w.queue_draw();
                        }
                    },
                );
            }
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

/// Device-pixel rectangle → the logical coordinates GTK snapshots use.
fn logical(r: DeviceRect, scale: f64) -> graphene::Rect {
    graphene::Rect::new(
        (r.origin.x / scale) as f32,
        (r.origin.y / scale) as f32,
        (r.size.width / scale) as f32,
        (r.size.height / scale) as f32,
    )
}

/// Draws a frame; returns when the view settles, if it hasn't yet.
fn draw(s: &mut State, snap: &gtk::Snapshot, size: ViewSize, scale: f64) -> Option<Instant> {
    if s.viewer.viewport() != size || s.viewer.scale_factor() != scale {
        s.viewer.set_viewport(size, scale);
    }
    snap.append_color(
        &gdk::RGBA::new(0.85, 0.85, 0.86, 1.0),
        &graphene::Rect::new(0.0, 0.0, size.width as f32, size.height as f32),
    );

    let frame = s.viewer.frame(Instant::now(), |k| s.cache.contains(k));
    s.cache.begin_frame();
    for page in &frame.pages {
        let rect = logical(page.rect, scale);
        let shadow = logical(page.rect.inflate(scale, scale), scale);
        snap.append_color(&gdk::RGBA::new(0.0, 0.0, 0.0, 0.15), &shadow);
        snap.append_color(&gdk::RGBA::WHITE, &rect);
        snap.push_clip(&rect);
        for layer in &page.layers {
            if let Some(texture) = s.cache.get(&layer.key) {
                // Not `append_scaled_texture`: on a HiDPI surface GSK renders
                // that node at logical resolution and scales the result up,
                // which blurs every tile (measured on macOS at scale 2, see
                // M0-FINDINGS.md). A plain texture node is sampled at device
                // resolution, and is pixel-exact when `layer.exact`.
                snap.append_texture(texture, &logical(layer.dest, scale));
            }
        }
        snap.pop();
    }
    if !frame.complete {
        s.stats.blurry_frames += 1;
    }
    s.scheduler.request(frame.requests);
    frame.settles_at
}

/// Logical offset that moves this widget's origin onto a whole device pixel
/// of its surface. At fractional scales the widget can sit at e.g. 31.25
/// device px (a 25 px CSD shadow × 1.25), and then every tile is resampled
/// between pixels. Measured at scale 1.25: edge contrast 80.7 without this,
/// identical to an exact render with it (M0-FINDINGS.md).
fn device_pixel_snap(widget: &impl IsA<gtk::Widget>, scale: f64) -> (f32, f32) {
    let Some(native) = widget.native() else {
        return (0.0, 0.0);
    };
    let Some(p) = widget.compute_point(&native, &graphene::Point::new(0.0, 0.0)) else {
        return (0.0, 0.0);
    };
    let (sx, sy) = native.surface_transform();
    let snap = |logical: f64| {
        let device = logical * scale;
        ((device.round() - device) / scale) as f32
    };
    (snap(p.x() as f64 + sx), snap(p.y() as f64 + sy))
}
