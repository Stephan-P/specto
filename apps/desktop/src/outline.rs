//! The outline (table of contents) sidebar: a `GtkListView` over a
//! `GtkTreeListModel`, so large outlines only create widgets for visible rows.

use gtk::prelude::*;
use gtk::{gio, glib};
use pdfrs_source::{OutlineItem, Target};

fn store(items: &[OutlineItem]) -> gio::ListStore {
    let store = gio::ListStore::new::<glib::BoxedAnyObject>();
    for item in items {
        store.append(&glib::BoxedAnyObject::new(item.clone()));
    }
    store
}

fn item_of(obj: &glib::Object) -> Option<std::cell::Ref<'_, OutlineItem>> {
    obj.downcast_ref::<glib::BoxedAnyObject>()
        .map(|b| b.borrow::<OutlineItem>())
}

pub struct OutlineView {
    pub widget: gtk::ScrolledWindow,
}

impl OutlineView {
    /// `page_label` turns a page index into what to show ("12", "iv");
    /// `on_activate` is called with the target of the item the user picks.
    pub fn new(
        items: &[OutlineItem],
        page_label: impl Fn(usize) -> String + 'static,
        on_activate: impl Fn(&Target) + 'static,
    ) -> Self {
        let tree = gtk::TreeListModel::new(store(items), false, false, |obj| {
            let item = item_of(obj)?;
            (!item.children.is_empty()).then(|| store(&item.children).upcast())
        });
        let selection = gtk::SingleSelection::new(Some(tree));
        selection.set_autoselect(false);
        selection.set_can_unselect(true);
        selection.set_selected(gtk::INVALID_LIST_POSITION);

        let factory = gtk::SignalListItemFactory::new();
        // Row: [title, up to three lines] [page label, grey, right-aligned].
        factory.connect_setup(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().expect("ListItem");
            let title = gtk::Label::builder()
                .xalign(0.0)
                .hexpand(true)
                .wrap(true)
                .wrap_mode(gtk::pango::WrapMode::WordChar)
                .lines(3)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .build();
            // Both top-aligned: same font, so the page number lines up with
            // the title's first line.
            let page = gtk::Label::builder()
                .xalign(1.0)
                .valign(gtk::Align::Start)
                .css_classes(["dim-label", "numeric"])
                .build();
            // The sidebar has a minimum row height. Centring the box keeps
            // single-line titles centred in it; the margins give multi-line
            // titles room (one line plus margins still fits the minimum, so
            // single-line rows don't get taller).
            let row = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(6)
                .valign(gtk::Align::Center)
                .margin_top(8)
                .margin_bottom(8)
                .build();
            title.set_valign(gtk::Align::Start);
            row.append(&title);
            row.append(&page);
            let expander = gtk::TreeExpander::new();
            expander.set_child(Some(&row));
            item.set_child(Some(&expander));
        });
        let label_of = std::rc::Rc::new(page_label);
        factory.connect_bind(move |_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().expect("ListItem");
            let Some(row) = item.item().and_downcast::<gtk::TreeListRow>() else {
                return;
            };
            let expander = item
                .child()
                .and_downcast::<gtk::TreeExpander>()
                .expect("expander");
            let line = expander.child().expect("row");
            let title = line
                .first_child()
                .and_downcast::<gtk::Label>()
                .expect("title");
            let page = title
                .next_sibling()
                .and_downcast::<gtk::Label>()
                .expect("page");
            expander.set_list_row(Some(&row));
            if let Some(entry) = row.item().as_ref().and_then(item_of) {
                title.set_label(&entry.title);
                // The full title, for the rare ones longer than three lines.
                title.set_tooltip_text(Some(&entry.title));
                let number = match &entry.target {
                    Some(Target::Destination(d)) => label_of(d.page),
                    _ => String::new(),
                };
                page.set_label(&number);
                // Items without a supported target are shown, but dimmed.
                title.set_opacity(if entry.target.is_some() { 1.0 } else { 0.55 });
            }
        });

        let list = gtk::ListView::new(Some(selection.clone()), Some(factory));
        list.add_css_class("navigation-sidebar");
        list.set_single_click_activate(true);
        list.connect_activate(move |_, position| {
            let Some(row) = selection.item(position).and_downcast::<gtk::TreeListRow>() else {
                return;
            };
            let target = row
                .item()
                .as_ref()
                .and_then(item_of)
                .and_then(|i| i.target.clone());
            if let Some(target) = target {
                on_activate(&target);
            }
        });

        let widget = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&list)
            .build();
        Self { widget }
    }
}
