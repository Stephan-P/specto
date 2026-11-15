//! The page view: draws `specto_view::Frame`s and implements `GtkScrollable`,
//! so a `GtkScrolledWindow` around it provides scrollbars and kinetic scrolling.

use std::cell::{Cell, RefCell};
use std::time::Instant;

use adw::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib, graphene, gsk};
use specto_geom::{DeviceRect, LayoutVector, ViewPoint, ViewSize, ViewVector};
use specto_source::{NamedAction, Target};
use specto_view::{
    Granularity, Selection, Viewer, over_text, text_char, text_position, text_quads,
};

use crate::document::SharedDocument;
use specto_doc::SourceId;
use specto_session::ViewShape;

/// Dragging a selection this close to the top or bottom edge (logical px)
/// scrolls the view.
const AUTOSCROLL_EDGE: f64 = 24.0;

/// Handles a clicked link's URI (the window decides whether to open it).
type UriHandler = Box<dyn Fn(&str)>;

mod imp {
    use super::*;

    #[derive(glib::Properties)]
    #[properties(wrapper_type = super::Canvas)]
    pub struct Canvas {
        #[property(override_interface = gtk::Scrollable, get, set = Self::set_hadjustment, nullable)]
        hadjustment: RefCell<Option<gtk::Adjustment>>,
        #[property(override_interface = gtk::Scrollable, get, set = Self::set_vadjustment, nullable)]
        vadjustment: RefCell<Option<gtk::Adjustment>>,
        #[property(override_interface = gtk::Scrollable, get, set, builder(gtk::ScrollablePolicy::Minimum))]
        hscroll_policy: Cell<gtk::ScrollablePolicy>,
        #[property(override_interface = gtk::Scrollable, get, set, builder(gtk::ScrollablePolicy::Minimum))]
        vscroll_policy: Cell<gtk::ScrollablePolicy>,

        pub document: RefCell<Option<SharedDocument>>,
        /// Set while we push viewer state into the adjustments, so their
        /// value-changed handlers don't feed it back.
        pub syncing: Cell<bool>,
        pub on_view_changed: RefCell<Option<Box<dyn Fn()>>>,
        /// Called with a link's URI; the window decides whether to open it.
        pub on_open_uri: RefCell<Option<UriHandler>>,
        /// The cursor currently set: `None` (default), "pointer" over links,
        /// "text" over text.
        pub cursor: Cell<Option<&'static str>>,
        /// Clicks in the current click sequence (1, 2, 3), from the last press.
        pub presses: Cell<i32>,
        /// Where the current drag started, and whether it is selecting yet
        /// (it starts once past the drag threshold, so clicks stay clicks).
        pub drag_start: Cell<Option<(f64, f64)>>,
        pub selecting: Cell<bool>,
        /// Scrolls while a selection drag is outside the view.
        pub autoscroll: RefCell<Option<glib::SourceId>>,
        pub pointer: Cell<(f64, f64)>,
        /// Pending redraw for when the view settles (switches to exact tiles).
        pub settle_timer: RefCell<Option<glib::SourceId>>,
        /// Spinner for pages that take long to render. Only exists while one
        /// is shown: it asks for a redraw every frame while it animates.
        pub spinner: RefCell<Option<(adw::SpinnerPaintable, glib::SignalHandlerId)>>,
        /// Whether the last frame was final: every visible page sharp and
        /// nothing left to settle. Reset when the view changes.
        pub complete: Cell<bool>,
    }

    impl Default for Canvas {
        fn default() -> Self {
            Self {
                hadjustment: RefCell::default(),
                vadjustment: RefCell::default(),
                hscroll_policy: Cell::new(gtk::ScrollablePolicy::Minimum),
                vscroll_policy: Cell::new(gtk::ScrollablePolicy::Minimum),
                document: RefCell::default(),
                syncing: Cell::default(),
                on_view_changed: RefCell::default(),
                on_open_uri: RefCell::default(),
                cursor: Cell::default(),
                presses: Cell::default(),
                drag_start: Cell::default(),
                selecting: Cell::default(),
                autoscroll: RefCell::default(),
                pointer: Cell::default(),
                settle_timer: RefCell::default(),
                spinner: RefCell::default(),
                complete: Cell::default(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Canvas {
        const NAME: &'static str = "SpectoCanvas";
        type Type = super::Canvas;
        type ParentType = gtk::Widget;
        type Interfaces = (gtk::Scrollable,);
    }

    impl Canvas {
        fn set_hadjustment(&self, adj: Option<gtk::Adjustment>) {
            self.connect_adjustment(&adj);
            self.hadjustment.replace(adj);
            self.obj().sync_adjustments();
        }

        fn set_vadjustment(&self, adj: Option<gtk::Adjustment>) {
            self.connect_adjustment(&adj);
            self.vadjustment.replace(adj);
            self.obj().sync_adjustments();
        }

        fn connect_adjustment(&self, adj: &Option<gtk::Adjustment>) {
            if let Some(adj) = adj {
                let obj = self.obj().downgrade();
                adj.connect_value_changed(move |_| {
                    let Some(obj) = obj.upgrade() else { return };
                    if obj.imp().syncing.get() {
                        return;
                    }
                    let (h, v) = (
                        obj.hadjustment().map_or(0.0, |a| a.value()),
                        obj.vadjustment().map_or(0.0, |a| a.value()),
                    );
                    obj.update_viewer(|v_| v_.set_scroll_offset(LayoutVector::new(h, v)));
                });
            }
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for Canvas {}

    impl ScrollableImpl for Canvas {}

    impl WidgetImpl for Canvas {
        fn size_allocate(&self, width: i32, height: i32, _baseline: i32) {
            let obj = self.obj();
            let scale = obj.device_scale();
            obj.update_viewer(|v| {
                v.set_viewport(ViewSize::new(width as f64, height as f64), scale)
            });
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let Some(doc) = self.document.borrow().clone() else {
                return;
            };
            let obj = self.obj();
            let scale = obj.device_scale();
            let mut guard = doc.borrow_mut();
            let d = &mut *guard;
            // The surface scale can change without a new allocation (moving
            // the window to another monitor).
            if d.viewer.scale_factor() != scale {
                let size = d.viewer.viewport();
                d.viewer.set_viewport(size, scale);
            }
            let now = Instant::now();
            let frame = d.session.viewer.frame(now, |k| d.cache.contains(k));
            let (dx, dy) = device_pixel_snap(&*obj, scale);
            snapshot.save();
            snapshot.translate(&graphene::Point::new(dx, dy));
            d.cache.begin_frame();
            for page in &frame.pages {
                let rect = logical(page.rect, scale);
                snapshot.append_outset_shadow(
                    &gsk::RoundedRect::from_rect(rect, 0.0),
                    &gdk::RGBA::new(0.0, 0.0, 0.0, 0.25),
                    0.0,
                    1.0,
                    0.0,
                    3.0,
                );
                // Annotations made here lie over the page (see
                // `Session::overlay`). A highlight darkens what is beneath
                // it, so the page is the bottom of a blend with those.
                let overlay = d.session.overlay(page.page);
                let (darkening, covering): (Vec<ViewShape>, Vec<ViewShape>) = overlay
                    .into_iter()
                    .partition(|s| matches!(s, ViewShape::Fill { multiply: true, .. }));
                if !darkening.is_empty() {
                    snapshot.push_blend(gsk::BlendMode::Multiply);
                }
                snapshot.append_color(&gdk::RGBA::WHITE, &rect);
                snapshot.push_clip(&rect);
                for layer in &page.layers {
                    if let Some(texture) = d.cache.get(&layer.key) {
                        // Not `append_scaled_texture`: on a HiDPI surface GSK
                        // renders that node at logical resolution and scales
                        // the result up, blurring every tile (measured on
                        // macOS, see M0-FINDINGS.md). A plain texture node is
                        // sampled at device resolution, 1:1 when `layer.exact`.
                        snapshot.append_texture(texture, &logical(layer.dest, scale));
                    }
                }
                snapshot.pop();
                if !darkening.is_empty() {
                    snapshot.pop();
                    snapshot.push_clip(&rect);
                    draw_shapes(snapshot, &darkening);
                    snapshot.pop();
                    snapshot.pop();
                }
                if !covering.is_empty() {
                    snapshot.push_clip(&rect);
                    draw_shapes(snapshot, &covering);
                    snapshot.pop();
                }
            }
            obj.draw_picked(snapshot, d, &frame.pages, scale);
            obj.draw_matches(snapshot, d, &frame.pages, scale);
            obj.draw_selection(snapshot, d, &frame.pages, scale);
            // A spinner over expensive pages (huge images, or renders that
            // already took > 500 ms) that aren't sharp yet. Cheap pages
            // render in milliseconds and never flash one.
            let viewport = DeviceRect::new(
                specto_geom::DevicePoint::origin(),
                (obj.width() as f64 * scale, obj.height() as f64 * scale).into(),
            );
            let busy: Vec<DeviceRect> = frame
                .pages
                .iter()
                .filter(|p| !p.complete && d.is_expensive(p.page))
                .filter_map(|p| p.rect.intersection(&viewport))
                .collect();
            obj.draw_spinners(snapshot, &busy, scale);
            snapshot.restore();
            d.set_canvas_requests(frame.requests);
            drop(guard);
            self.complete
                .set(frame.complete && frame.settles_at.is_none());
            if let Some(at) = frame.settles_at {
                obj.redraw_at(now, at);
            }
        }
    }
}

glib::wrapper! {
    pub struct Canvas(ObjectSubclass<imp::Canvas>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Scrollable;
}

impl Default for Canvas {
    fn default() -> Self {
        Self::new()
    }
}

impl Canvas {
    pub fn new() -> Self {
        let obj: Self = glib::Object::new();
        obj.set_hexpand(true);
        obj.set_vexpand(true);
        obj.set_overflow(gtk::Overflow::Hidden);
        obj.set_focusable(true);
        obj.add_css_class("view");
        obj.setup_input();
        obj.connect_scale_factor_notify(|c| c.queue_resize());
        obj
    }

    pub fn set_document(&self, doc: Option<SharedDocument>) {
        self.imp().document.replace(doc);
        self.queue_resize();
        self.queue_draw();
    }

    /// Called after anything changes the view (scroll, zoom, page, mode).
    /// Called when a link to a URI is clicked.
    pub fn connect_open_uri(&self, f: impl Fn(&str) + 'static) {
        self.imp().on_open_uri.replace(Some(Box::new(f)));
    }

    /// Follows a link or outline target in file `source`.
    pub fn follow(&self, source: SourceId, target: &Target) {
        match target {
            Target::Destination(d) => {
                // Destinations name pages of the file; pages may have moved
                // or been deleted since.
                let Some(doc) = self.imp().document.borrow().clone() else {
                    return;
                };
                let Some(page) = doc.borrow().position_of(source, d.page) else {
                    return;
                };
                self.update_viewer(|v| v.go_to_pdf_position(page, d.left, d.top))
            }
            Target::Named(NamedAction::NextPage) => self.update_viewer(Viewer::next_page),
            Target::Named(NamedAction::PrevPage) => self.update_viewer(Viewer::previous_page),
            Target::Named(NamedAction::FirstPage) => self.update_viewer(|v| v.go_to_page(0)),
            Target::Named(NamedAction::LastPage) => {
                self.update_viewer(|v| v.go_to_page(v.page_count().saturating_sub(1)))
            }
            Target::Uri(uri) => {
                if let Some(f) = self.imp().on_open_uri.borrow().as_ref() {
                    f(uri);
                }
            }
        }
    }

    fn link_at(&self, x: f64, y: f64) -> Option<(SourceId, Target)> {
        let doc = self.imp().document.borrow().clone()?;
        let mut d = doc.borrow_mut();
        d.link_at(ViewPoint::new(x, y))
    }

    pub fn connect_view_changed(&self, f: impl Fn() + 'static) {
        self.imp().on_view_changed.replace(Some(Box::new(f)));
    }

    /// Applies a change to the viewer, then updates scrollbars and redraws.
    pub fn update_viewer(&self, f: impl FnOnce(&mut Viewer)) {
        let Some(doc) = self.imp().document.borrow().clone() else {
            return;
        };
        f(&mut doc.borrow_mut().viewer);
        self.imp().complete.set(false);
        self.sync_adjustments();
        self.queue_draw();
        if let Some(cb) = self.imp().on_view_changed.borrow().as_ref() {
            cb();
        }
    }

    /// Whether the last frame was drawn completely, with sharp tiles.
    pub fn is_complete(&self) -> bool {
        self.imp().complete.get()
    }

    /// Read-only access to the viewer.
    pub fn with_viewer<R>(&self, f: impl FnOnce(&Viewer) -> R) -> Option<R> {
        let doc = self.imp().document.borrow().clone()?;
        let d = doc.borrow();
        Some(f(&d.viewer))
    }

    // --- text selection ---------------------------------------------------

    /// Highlights search matches on the visible pages: all in yellow, the
    /// current one in orange (Adwaita's palette).
    fn draw_matches(
        &self,
        snapshot: &gtk::Snapshot,
        d: &mut crate::document::Document,
        pages: &[specto_view::PageDraw],
        scale: f64,
    ) {
        let yellow = gdk::RGBA::new(0.965, 0.827, 0.176, 0.5);
        let orange = gdk::RGBA::new(1.0, 0.471, 0.0, 0.55);
        for page in pages {
            let matches = d.matches_on(page.page);
            if matches.is_empty() {
                continue;
            }
            let (viewer, mut texts, _) = d.text();
            let Some(text) = specto_view::TextPages::text_page(&mut texts, page.page) else {
                continue;
            };
            snapshot.push_clip(&logical(page.rect, scale));
            for (m, current) in matches {
                let quads = text_quads(viewer, &text, page.page, m.chars);
                fill_quads(snapshot, &quads, if current { &orange } else { &yellow });
            }
            snapshot.pop();
        }
    }

    /// Fills the selected text of the visible pages with the accent colour.
    fn draw_selection(
        &self,
        snapshot: &gtk::Snapshot,
        d: &mut crate::document::Document,
        pages: &[specto_view::PageDraw],
        scale: f64,
    ) {
        let (viewer, mut texts, selection) = d.text();
        let Some(selection) = selection.as_ref().filter(|s| !s.is_empty()) else {
            return;
        };
        let mut colour = adw::StyleManager::default().accent_color_rgba();
        colour.set_alpha(0.35);
        for page in pages {
            let quads = selection.view_quads(viewer, &mut texts, page.page);
            if quads.is_empty() {
                continue;
            }
            snapshot.push_clip(&logical(page.rect, scale));
            fill_quads(snapshot, &quads, &colour);
            snapshot.pop();
        }
    }

    /// A frame around the annotation that is picked.
    fn draw_picked(
        &self,
        snapshot: &gtk::Snapshot,
        d: &mut crate::document::Document,
        pages: &[specto_view::PageDraw],
        scale: f64,
    ) {
        let Some((position, corners)) = d.picked_outline() else {
            return;
        };
        let Some(page) = pages.iter().find(|p| p.page == position) else {
            return;
        };
        // A little outside the annotation, so it doesn't cover its edge.
        let (xs, ys) = (corners.map(|c| c.x as f32), corners.map(|c| c.y as f32));
        let min = |v: [f32; 4]| v.into_iter().fold(f32::INFINITY, f32::min);
        let max = |v: [f32; 4]| v.into_iter().fold(f32::NEG_INFINITY, f32::max);
        let frame = graphene::Rect::new(
            min(xs) - 3.0,
            min(ys) - 3.0,
            max(xs) - min(xs) + 6.0,
            max(ys) - min(ys) + 6.0,
        );
        let builder = gsk::PathBuilder::new();
        builder.add_rounded_rect(&gsk::RoundedRect::from_rect(frame, 3.0));
        snapshot.push_clip(&logical(page.rect, scale));
        snapshot.append_stroke(
            &builder.to_path(),
            &gsk::Stroke::new(1.5),
            &adw::StyleManager::default().accent_color_rgba(),
        );
        snapshot.pop();
    }

    /// Picks the annotation under the pointer, or none. Returns whether
    /// one is picked now.
    fn pick_annotation(&self, x: f64, y: f64) -> bool {
        let Some(doc) = self.imp().document.borrow().clone() else {
            return false;
        };
        let mut d = doc.borrow_mut();
        let picked = d.annotation_at(ViewPoint::new(x, y));
        let changed = d.picked != picked;
        d.picked = picked;
        drop(d);
        if changed {
            self.queue_draw();
        }
        picked.is_some()
    }

    /// Lets go of the picked annotation. Returns whether there was one.
    pub fn unpick_annotation(&self) -> bool {
        let Some(doc) = self.imp().document.borrow().clone() else {
            return false;
        };
        let had = doc.borrow_mut().picked.take().is_some();
        if had {
            self.queue_draw();
        }
        had
    }

    /// Applies `f` to the selection (with the viewer and text layer), then
    /// redraws.
    fn edit_selection(
        &self,
        f: impl FnOnce(&Viewer, &mut crate::document::TextLayer<'_>, &mut Option<Selection>),
    ) {
        let Some(doc) = self.imp().document.borrow().clone() else {
            return;
        };
        {
            let mut d = doc.borrow_mut();
            let (viewer, mut texts, selection) = d.text();
            f(viewer, &mut texts, selection);
        }
        self.queue_draw();
    }

    /// A press: one click starts afresh (or extends with Shift), two select
    /// a word, three a line.
    fn press(&self, presses: i32, x: f64, y: f64, extend: bool) {
        let p = ViewPoint::new(x, y);
        self.edit_selection(|viewer, texts, selection| match presses {
            1 if extend && selection.is_some() => {
                if let (Some(s), Some(to)) = (selection.as_mut(), text_position(viewer, texts, p)) {
                    s.extend(texts, to);
                }
            }
            1 => *selection = None,
            n => {
                let g = if n == 2 {
                    Granularity::Word
                } else {
                    Granularity::Line
                };
                *selection = text_char(viewer, texts, p).map(|at| Selection::new(texts, at, g));
            }
        });
    }

    /// Starts a character selection where a drag began.
    fn begin_selection(&self, x: f64, y: f64) {
        self.edit_selection(|viewer, texts, selection| {
            *selection = text_position(viewer, texts, ViewPoint::new(x, y))
                .map(|at| Selection::new(texts, at, Granularity::Char));
        });
    }

    /// Extends the selection to the point under the pointer.
    fn extend_selection(&self, x: f64, y: f64) {
        self.edit_selection(|viewer, texts, selection| {
            if let (Some(s), Some(to)) = (
                selection.as_mut(),
                text_position(viewer, texts, ViewPoint::new(x, y)),
            ) {
                s.extend(texts, to);
            }
        });
    }

    pub fn select_all(&self) {
        self.edit_selection(|viewer, _, selection| {
            *selection = Some(Selection::all(viewer.page_count()));
        });
    }

    pub fn clear_selection(&self) -> bool {
        let mut had = false;
        self.edit_selection(|_, _, selection| had = selection.take().is_some());
        had
    }

    /// The selected text, if any.
    pub fn selected_text(&self) -> Option<String> {
        let doc = self.imp().document.borrow().clone()?;
        doc.borrow_mut().selected_text()
    }

    /// Copies the selection to the clipboard. False if nothing is selected.
    pub fn copy(&self) -> bool {
        let Some(text) = self.selected_text() else {
            return false;
        };
        self.clipboard().set_text(&text);
        true
    }

    /// Also offers the selection as the primary selection (middle-click
    /// paste on Linux), as other apps do.
    fn offer_primary(&self) {
        if let Some(text) = self.selected_text() {
            self.primary_clipboard().set_text(&text);
        }
    }

    /// How far the pointer is into the autoscroll zone at the top (negative)
    /// or bottom (positive) of the view: the last `AUTOSCROLL_EDGE` pixels
    /// and beyond. A maximized window's edge is the screen's, and the
    /// pointer can't go past that.
    fn autoscroll_overshoot(&self, y: f64) -> f64 {
        let height = self.height() as f64;
        if y < AUTOSCROLL_EDGE {
            y - AUTOSCROLL_EDGE
        } else if y > height - AUTOSCROLL_EDGE {
            y - (height - AUTOSCROLL_EDGE)
        } else {
            0.0
        }
    }

    /// While a selection drag is near or beyond the top or bottom of the
    /// view, scrolls towards the pointer, faster the further out it is.
    fn autoscroll(&self) {
        let imp = self.imp();
        if imp.autoscroll.borrow().is_some() {
            return;
        }
        let this = self.downgrade();
        let id = glib::timeout_add_local(std::time::Duration::from_millis(30), move || {
            let Some(c) = this.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let (x, y) = c.imp().pointer.get();
            let over = c.autoscroll_overshoot(y);
            if over == 0.0 || !c.imp().selecting.get() {
                c.imp().autoscroll.take();
                return glib::ControlFlow::Break;
            }
            let step = over.clamp(-80.0, 80.0) / 2.0;
            c.update_viewer(|v| v.scroll_by(ViewVector::new(0.0, step)));
            c.extend_selection(x, y);
            glib::ControlFlow::Continue
        });
        imp.autoscroll.replace(Some(id));
    }

    /// The cursor for the pointer at `(x, y)`: a hand over links, an I-beam
    /// over text.
    fn update_cursor(&self, x: f64, y: f64) {
        let p = ViewPoint::new(x, y);
        let name = if self.link_at(x, y).is_some() {
            Some("pointer")
        } else {
            let mut over = false;
            if let Some(doc) = self.imp().document.borrow().clone() {
                let mut d = doc.borrow_mut();
                let (viewer, mut texts, _) = d.text();
                over = over_text(viewer, &mut texts, p);
            }
            over.then_some("text")
        };
        if name != self.imp().cursor.replace(name) {
            self.set_cursor_from_name(name);
        }
    }

    /// The last pointer position over the canvas (for tests).
    pub fn pointer(&self) -> (f64, f64) {
        self.imp().pointer.get()
    }

    /// The cursor name currently shown (for tests).
    pub fn cursor_name(&self) -> Option<&'static str> {
        self.imp().cursor.get()
    }

    /// Draws a spinner in the middle of each rectangle (device pixels), and
    /// creates or drops the animating paintable as needed.
    fn draw_spinners(&self, snapshot: &gtk::Snapshot, at: &[DeviceRect], scale: f64) {
        const SIZE: f32 = 32.0;
        const BACKING: f32 = 56.0;
        let imp = self.imp();
        if at.is_empty() {
            if let Some((paintable, handler)) = imp.spinner.take() {
                paintable.disconnect(handler);
            }
            return;
        }
        if imp.spinner.borrow().is_none() {
            let paintable = adw::SpinnerPaintable::new(Some(self));
            let this = self.downgrade();
            let handler = paintable.connect_invalidate_contents(move |_| {
                if let Some(c) = this.upgrade() {
                    c.queue_draw();
                }
            });
            imp.spinner.replace(Some((paintable, handler)));
        }
        let spinner = imp.spinner.borrow();
        let Some((paintable, _)) = spinner.as_ref() else {
            return;
        };
        for r in at {
            let centre = graphene::Point::new(
                ((r.origin.x + r.size.width / 2.0) / scale) as f32,
                ((r.origin.y + r.size.height / 2.0) / scale) as f32,
            );
            let backing = gsk::RoundedRect::from_rect(
                graphene::Rect::new(
                    centre.x() - BACKING / 2.0,
                    centre.y() - BACKING / 2.0,
                    BACKING,
                    BACKING,
                ),
                12.0,
            );
            snapshot.append_outset_shadow(
                &backing,
                &gdk::RGBA::new(0.0, 0.0, 0.0, 0.2),
                0.0,
                1.0,
                0.0,
                4.0,
            );
            snapshot.push_rounded_clip(&backing);
            snapshot.append_color(&gdk::RGBA::new(1.0, 1.0, 1.0, 0.92), backing.bounds());
            snapshot.pop();
            snapshot.save();
            snapshot.translate(&graphene::Point::new(
                centre.x() - SIZE / 2.0,
                centre.y() - SIZE / 2.0,
            ));
            paintable.snapshot(snapshot, SIZE as f64, SIZE as f64);
            snapshot.restore();
        }
    }

    /// Schedules one redraw at `at` (when the view settles), unless one is
    /// already pending; a redraw then reschedules if the deadline moved.
    fn redraw_at(&self, now: Instant, at: Instant) {
        let imp = self.imp();
        if imp.settle_timer.borrow().is_some() {
            return;
        }
        let this = self.downgrade();
        let id = glib::timeout_add_local_once(at.saturating_duration_since(now), move || {
            if let Some(c) = this.upgrade() {
                c.imp().settle_timer.take();
                c.queue_draw();
            }
        });
        imp.settle_timer.replace(Some(id));
    }

    pub fn device_scale(&self) -> f64 {
        self.native()
            .and_then(|n| n.surface())
            .map_or(self.scale_factor() as f64, |s| s.scale())
    }

    /// Pushes the viewer's content size and scroll offset into the adjustments.
    fn sync_adjustments(&self) {
        let Some((content, scroll, view)) =
            self.with_viewer(|v| (v.content_size(), v.scroll_offset(), v.viewport()))
        else {
            return;
        };
        let imp = self.imp();
        imp.syncing.set(true);
        let configure = |adj: Option<gtk::Adjustment>, value: f64, upper: f64, page: f64| {
            if let Some(adj) = adj {
                // Centred short content has a negative offset; scrollbars start at 0.
                adj.configure(
                    value.max(0.0),
                    0.0,
                    upper.max(page),
                    page * 0.1,
                    page * 0.9,
                    page,
                );
            }
        };
        configure(self.hadjustment(), scroll.x, content.width, view.width);
        configure(self.vadjustment(), scroll.y, content.height, view.height);
        imp.syncing.set(false);
    }

    fn setup_input(&self) {
        let motion = gtk::EventControllerMotion::new();
        let this = self.downgrade();
        motion.connect_motion(move |_, x, y| {
            if let Some(c) = this.upgrade() {
                c.imp().pointer.set((x, y));
                if !c.imp().selecting.get() {
                    c.update_cursor(x, y);
                }
            }
        });
        self.add_controller(motion);

        // Presses: one click clears the selection (Shift extends it), two
        // select a word, three a line. A click without movement on a link
        // follows it.
        let click = gtk::GestureClick::new();
        click.set_button(gdk::BUTTON_PRIMARY);
        let this = self.downgrade();
        click.connect_pressed(move |g, n_press, x, y| {
            let Some(c) = this.upgrade() else { return };
            c.grab_focus();
            c.imp().presses.set(n_press);
            let shift = g
                .current_event_state()
                .contains(gdk::ModifierType::SHIFT_MASK);
            c.press(n_press.min(3), x, y, shift);
        });
        let this = self.downgrade();
        click.connect_released(move |g, n_press, x, y| {
            let Some(c) = this.upgrade() else { return };
            if n_press != 1 || c.imp().selecting.get() {
                return;
            }
            if let Some((source, target)) = c.link_at(x, y) {
                g.set_state(gtk::EventSequenceState::Claimed);
                c.follow(source, &target);
                return;
            }
            // A click on an annotation picks it; one elsewhere lets go.
            c.pick_annotation(x, y);
        });
        self.add_controller(click);

        // Dragging selects text: by characters after a single press (once
        // past the drag threshold, so clicks and links still work), by words
        // or lines after a double or triple one.
        let drag = gtk::GestureDrag::new();
        drag.set_button(gdk::BUTTON_PRIMARY);
        let this = self.downgrade();
        drag.connect_drag_begin(move |_, x, y| {
            if let Some(c) = this.upgrade() {
                c.imp().drag_start.set(Some((x, y)));
                c.imp().selecting.set(false);
            }
        });
        let this = self.downgrade();
        drag.connect_drag_update(move |g, dx, dy| {
            let Some(c) = this.upgrade() else { return };
            let imp = c.imp();
            let Some((x0, y0)) = imp.drag_start.get() else {
                return;
            };
            let (x, y) = (x0 + dx, y0 + dy);
            imp.pointer.set((x, y));
            if !imp.selecting.get() {
                let threshold =
                    gtk::Settings::default().map_or(8, |s| s.gtk_dnd_drag_threshold()) as f64;
                if imp.presses.get() == 1 && dx.hypot(dy) < threshold {
                    return;
                }
                imp.selecting.set(true);
                g.set_state(gtk::EventSequenceState::Claimed);
                if imp.presses.get() == 1 {
                    c.begin_selection(x0, y0);
                }
                c.set_cursor_from_name(Some("text"));
                imp.cursor.set(Some("text"));
            }
            c.extend_selection(x, y);
            if c.autoscroll_overshoot(y) != 0.0 {
                c.autoscroll();
            }
        });
        let this = self.downgrade();
        drag.connect_drag_end(move |_, _, _| {
            let Some(c) = this.upgrade() else { return };
            let imp = c.imp();
            imp.drag_start.set(None);
            if imp.selecting.replace(false) {
                c.offer_primary();
            }
            if let Some(id) = imp.autoscroll.take() {
                id.remove();
            }
        });
        self.add_controller(drag);

        // Ctrl+scroll zooms. Plain scrolling bubbles up to the GtkScrolledWindow.
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        let this = self.downgrade();
        scroll.connect_scroll(move |ctl, _dx, dy| {
            let Some(c) = this.upgrade() else {
                return glib::Propagation::Proceed;
            };
            // Ctrl+scroll zooms; on macOS Cmd+scroll too.
            if !ctl
                .current_event_state()
                .intersects(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::META_MASK)
            {
                return glib::Propagation::Proceed;
            }
            let step = if ctl.unit() == gdk::ScrollUnit::Wheel {
                0.15
            } else {
                0.01
            };
            let (x, y) = c.imp().pointer.get();
            c.update_viewer(|v| v.zoom_by((-dy * step).exp(), ViewPoint::new(x, y)));
            glib::Propagation::Stop
        });
        self.add_controller(scroll);

        // Touchpad / touchscreen pinch.
        // While it lasts the view isn't "settled", so pauses mid-pinch don't
        // render exact-scale tiles for intermediate zooms.
        let pinch = gtk::GestureZoom::new();
        let last = std::rc::Rc::new(Cell::new(1.0));
        let l = last.clone();
        let this = self.downgrade();
        pinch.connect_begin(move |_, _| {
            l.set(1.0);
            if let Some(c) = this.upgrade() {
                c.update_viewer(Viewer::begin_zoom_gesture);
            }
        });
        let this = self.downgrade();
        pinch.connect_end(move |_, _| {
            if let Some(c) = this.upgrade() {
                c.update_viewer(Viewer::end_zoom_gesture);
            }
        });
        let this = self.downgrade();
        pinch.connect_cancel(move |_, _| {
            if let Some(c) = this.upgrade() {
                c.update_viewer(Viewer::end_zoom_gesture);
            }
        });
        let this = self.downgrade();
        pinch.connect_scale_changed(move |g, scale| {
            let Some(c) = this.upgrade() else { return };
            let (x, y) = g.bounding_box_center().unwrap_or((0.0, 0.0));
            let factor = scale / last.replace(scale);
            c.update_viewer(|v| v.zoom_by(factor, ViewPoint::new(x, y)));
        });
        self.add_controller(pinch);

        // Paging keys. Arrow keys and space are handled here too, so they
        // work in paged modes where there's nothing to scroll.
        let keys = gtk::EventControllerKey::new();
        let this = self.downgrade();
        keys.connect_key_pressed(move |_, key, _, state| {
            let Some(c) = this.upgrade() else {
                return glib::Propagation::Proceed;
            };
            if state.intersects(
                gdk::ModifierType::CONTROL_MASK
                    | gdk::ModifierType::ALT_MASK
                    | gdk::ModifierType::META_MASK,
            ) {
                return glib::Propagation::Proceed;
            }
            if key == gdk::Key::Escape && (c.unpick_annotation() || c.clear_selection()) {
                return glib::Propagation::Stop;
            }
            let Some((continuous, height)) =
                c.with_viewer(|v| (v.layout_mode().is_continuous(), v.viewport().height))
            else {
                return glib::Propagation::Proceed;
            };
            let page = (height - 40.0).max(40.0);
            match key {
                gdk::Key::Page_Down | gdk::Key::space if continuous => {
                    c.update_viewer(|v| v.scroll_by(ViewVector::new(0.0, page)))
                }
                gdk::Key::Page_Up | gdk::Key::BackSpace if continuous => {
                    c.update_viewer(|v| v.scroll_by(ViewVector::new(0.0, -page)))
                }
                gdk::Key::Page_Down | gdk::Key::space | gdk::Key::Right => {
                    c.update_viewer(Viewer::next_page)
                }
                gdk::Key::Page_Up | gdk::Key::BackSpace | gdk::Key::Left => {
                    c.update_viewer(Viewer::previous_page)
                }
                gdk::Key::Down => c.update_viewer(|v| v.scroll_by(ViewVector::new(0.0, 50.0))),
                gdk::Key::Up => c.update_viewer(|v| v.scroll_by(ViewVector::new(0.0, -50.0))),
                gdk::Key::Home => c.update_viewer(|v| v.go_to_page(0)),
                gdk::Key::End => {
                    c.update_viewer(|v| v.go_to_page(v.page_count().saturating_sub(1)))
                }
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        self.add_controller(keys);
    }
}

/// Draws the shapes of annotations, in order.
fn draw_shapes(snapshot: &gtk::Snapshot, shapes: &[ViewShape]) {
    let rgba = |c: specto_annot::Color, opacity: f32| gdk::RGBA::new(c.r, c.g, c.b, opacity);
    for shape in shapes {
        match *shape {
            ViewShape::Fill {
                quad,
                color,
                opacity,
                ..
            } => fill_quads(snapshot, &[quad], &rgba(color, opacity)),
            ViewShape::Stroke {
                from,
                to,
                width,
                color,
                opacity,
            } => {
                let builder = gsk::PathBuilder::new();
                builder.move_to(from.x as f32, from.y as f32);
                builder.line_to(to.x as f32, to.y as f32);
                snapshot.append_stroke(
                    &builder.to_path(),
                    &gsk::Stroke::new(width as f32),
                    &rgba(color, opacity),
                );
            }
        }
    }
}

/// Fills quadrilaterals (view coordinates) with `colour`.
fn fill_quads(snapshot: &gtk::Snapshot, quads: &[[ViewPoint; 4]], colour: &gdk::RGBA) {
    if quads.is_empty() {
        return;
    }
    let builder = gsk::PathBuilder::new();
    for q in quads {
        builder.move_to(q[0].x as f32, q[0].y as f32);
        for p in &q[1..] {
            builder.line_to(p.x as f32, p.y as f32);
        }
        builder.close();
    }
    snapshot.append_fill(&builder.to_path(), gsk::FillRule::Winding, colour);
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
