//! The thumbnail sidebar: a `GtkListView` of pages, so only visible rows exist.
//!
//! Thumbnails draw the viewer's placeholder tiles from the shared cache, so
//! each page is rendered once for both the sidebar and the canvas.

use std::cell::{Cell, RefCell};

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib, graphene, gsk};

use crate::document::SharedDocument;

/// Thumbnail width in logical pixels.
const WIDTH: f64 = 120.0;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Thumbnail {
        pub document: RefCell<Option<SharedDocument>>,
        pub page: Cell<Option<usize>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Thumbnail {
        const NAME: &'static str = "PdfrsThumbnail";
        type Type = super::Thumbnail;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Thumbnail {}

    impl WidgetImpl for Thumbnail {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::ConstantSize
        }

        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let size = match orientation {
                gtk::Orientation::Horizontal => WIDTH,
                _ => self.aspect().map_or(WIDTH * 1.3, |a| WIDTH * a),
            }
            .round() as i32;
            (size, size, -1, -1)
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let obj = self.obj();
            let rect = graphene::Rect::new(0.0, 0.0, obj.width() as f32, obj.height() as f32);
            snapshot.append_outset_shadow(
                &gsk::RoundedRect::from_rect(rect, 0.0),
                &gdk::RGBA::new(0.0, 0.0, 0.0, 0.25),
                0.0,
                1.0,
                0.0,
                2.0,
            );
            snapshot.append_color(&gdk::RGBA::WHITE, &rect);
            let (Some(doc), Some(page)) = (self.document.borrow().clone(), self.page.get()) else {
                return;
            };
            let mut d = doc.borrow_mut();
            let Some(key) = d.viewer.placeholder_key(page) else {
                return;
            };
            if let Some(texture) = d.cache.get(&key) {
                // Not `append_scaled_texture`: GSK renders that at logical
                // resolution on HiDPI surfaces (see canvas.rs).
                snapshot.append_texture(texture, &rect);
            }
        }
    }

    impl Thumbnail {
        /// Height / width of the page as displayed.
        fn aspect(&self) -> Option<f64> {
            let doc = self.document.borrow().clone()?;
            let d = doc.borrow();
            let page = d.viewer.pages().get(self.page.get()?)?;
            let s = page.displayed().size();
            Some(s.height / s.width)
        }
    }
}

glib::wrapper! {
    pub struct Thumbnail(ObjectSubclass<imp::Thumbnail>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Thumbnail {
    fn new(document: SharedDocument) -> Self {
        let obj: Self = glib::Object::new();
        obj.set_halign(gtk::Align::Center);
        obj.imp().document.replace(Some(document));
        obj
    }

    fn set_page(&self, page: Option<usize>) {
        self.imp().page.set(page);
        self.queue_resize();
        self.queue_draw();
    }
}

/// The sidebar for one document.
pub struct Sidebar {
    pub widget: gtk::ScrolledWindow,
    list: gtk::ListView,
    selection: gtk::SingleSelection,
    /// Set while the selection follows the canvas, so it doesn't jump back.
    following: std::rc::Rc<Cell<bool>>,
}

impl Sidebar {
    /// `on_activate` is called with a page index when the user picks a thumbnail.
    pub fn new(document: SharedDocument, on_activate: impl Fn(usize) + 'static) -> Self {
        let count = document.borrow().viewer.page_count();
        let labels: Vec<String> = (1..=count).map(|i| i.to_string()).collect();
        let model = gtk::StringList::new(&labels.iter().map(String::as_str).collect::<Vec<_>>());
        let selection = gtk::SingleSelection::new(Some(model));
        selection.set_autoselect(false);

        let factory = gtk::SignalListItemFactory::new();
        let doc = document.clone();
        factory.connect_setup(move |_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().expect("ListItem");
            let thumbnail = Thumbnail::new(doc.clone());
            let label = gtk::Label::new(None);
            label.add_css_class("caption");
            label.add_css_class("dim-label");
            let column = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(4)
                .margin_top(6)
                .margin_bottom(6)
                .build();
            column.append(&thumbnail);
            column.append(&label);
            item.set_child(Some(&column));
        });
        let doc = document.clone();
        factory.connect_bind(move |_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().expect("ListItem");
            let page = item.position() as usize;
            let column = item.child().expect("child");
            let thumbnail = column
                .first_child()
                .and_downcast::<Thumbnail>()
                .expect("thumbnail");
            let label = thumbnail
                .next_sibling()
                .and_downcast::<gtk::Label>()
                .expect("label");
            thumbnail.set_page(Some(page));
            let mut d = doc.borrow_mut();
            label.set_label(&d.page_label(page));
            d.thumbnails_shown.insert(page);
            d.flush_requests();
        });
        let doc = document.clone();
        factory.connect_unbind(move |_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().expect("ListItem");
            let column = item.child().expect("child");
            let thumbnail = column
                .first_child()
                .and_downcast::<Thumbnail>()
                .expect("thumbnail");
            if let Some(page) = thumbnail.imp().page.take() {
                doc.borrow_mut().thumbnails_shown.remove(&page);
            }
        });

        let list = gtk::ListView::new(Some(selection.clone()), Some(factory));
        list.add_css_class("navigation-sidebar");
        list.set_single_click_activate(false);

        let following = std::rc::Rc::new(Cell::new(false));
        let f = following.clone();
        selection.connect_selected_notify(move |s| {
            if !f.get() && s.selected() != gtk::INVALID_LIST_POSITION {
                on_activate(s.selected() as usize);
            }
        });

        let widget = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&list)
            .build();
        Self {
            widget,
            list,
            selection,
            following,
        }
    }

    /// Selects `page` and scrolls it into view, without reporting it back.
    pub fn follow(&self, page: usize) {
        if self.selection.selected() as usize == page {
            return;
        }
        self.following.set(true);
        self.selection.set_selected(page as u32);
        self.following.set(false);
        if self.widget.is_mapped() {
            self.list
                .scroll_to(page as u32, gtk::ListScrollFlags::NONE, None);
        }
    }

    /// Redraws visible thumbnails (after new tiles arrived).
    pub fn refresh(&self) {
        let mut child = self.list.first_child();
        while let Some(row) = child {
            if let Some(t) = row
                .first_child()
                .and_then(|col| col.first_child())
                .and_downcast::<Thumbnail>()
            {
                t.queue_draw();
            }
            child = row.next_sibling();
        }
    }
}
