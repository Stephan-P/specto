//! The page view: draws `pdfrs_view::Frame`s and implements `GtkScrollable`,
//! so a `GtkScrolledWindow` around it provides scrollbars and kinetic scrolling.

use std::cell::{Cell, RefCell};

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib, graphene, gsk};
use pdfrs_geom::{DeviceRect, LayoutVector, ViewPoint, ViewSize, ViewVector};
use pdfrs_view::Viewer;

use crate::document::SharedDocument;

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
        pub pointer: Cell<(f64, f64)>,
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
                pointer: Cell::default(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Canvas {
        const NAME: &'static str = "PdfrsCanvas";
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
            let mut d = doc.borrow_mut();
            // The surface scale can change without a new allocation (moving
            // the window to another monitor).
            if d.viewer.scale_factor() != scale {
                let size = d.viewer.viewport();
                d.viewer.set_viewport(size, scale);
            }
            let frame = d.viewer.frame(|k| d.cache.contains(k));
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
                snapshot.append_color(&gdk::RGBA::WHITE, &rect);
                snapshot.push_clip(&rect);
                for layer in &page.layers {
                    if let Some(texture) = d.cache.get(&layer.key) {
                        let filter = if layer.exact {
                            gsk::ScalingFilter::Nearest
                        } else {
                            gsk::ScalingFilter::Linear
                        };
                        snapshot.append_scaled_texture(
                            texture,
                            filter,
                            &logical(layer.dest, scale),
                        );
                    }
                }
                snapshot.pop();
            }
            d.set_canvas_requests(frame.requests);
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
    pub fn connect_view_changed(&self, f: impl Fn() + 'static) {
        self.imp().on_view_changed.replace(Some(Box::new(f)));
    }

    /// Applies a change to the viewer, then updates scrollbars and redraws.
    pub fn update_viewer(&self, f: impl FnOnce(&mut Viewer)) {
        let Some(doc) = self.imp().document.borrow().clone() else {
            return;
        };
        f(&mut doc.borrow_mut().viewer);
        self.sync_adjustments();
        self.queue_draw();
        if let Some(cb) = self.imp().on_view_changed.borrow().as_ref() {
            cb();
        }
    }

    /// Read-only access to the viewer.
    pub fn with_viewer<R>(&self, f: impl FnOnce(&Viewer) -> R) -> Option<R> {
        let doc = self.imp().document.borrow().clone()?;
        let d = doc.borrow();
        Some(f(&d.viewer))
    }

    fn device_scale(&self) -> f64 {
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
            }
        });
        self.add_controller(motion);

        // Ctrl+scroll zooms. Plain scrolling bubbles up to the GtkScrolledWindow.
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        let this = self.downgrade();
        scroll.connect_scroll(move |ctl, _dx, dy| {
            let Some(c) = this.upgrade() else {
                return glib::Propagation::Proceed;
            };
            if !ctl
                .current_event_state()
                .contains(gdk::ModifierType::CONTROL_MASK)
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
        let pinch = gtk::GestureZoom::new();
        let last = std::rc::Rc::new(Cell::new(1.0));
        let l = last.clone();
        pinch.connect_begin(move |_, _| l.set(1.0));
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
            if state.intersects(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::ALT_MASK) {
                return glib::Propagation::Proceed;
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

/// Device-pixel rectangle → the logical coordinates GTK snapshots use.
fn logical(r: DeviceRect, scale: f64) -> graphene::Rect {
    graphene::Rect::new(
        (r.origin.x / scale) as f32,
        (r.origin.y / scale) as f32,
        (r.size.width / scale) as f32,
        (r.size.height / scale) as f32,
    )
}
