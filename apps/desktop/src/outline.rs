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
    /// `on_activate` is called with the target of the item the user picks.
    pub fn new(items: &[OutlineItem], on_activate: impl Fn(&Target) + 'static) -> Self {
        let tree = gtk::TreeListModel::new(store(items), false, false, |obj| {
            let item = item_of(obj)?;
            (!item.children.is_empty()).then(|| store(&item.children).upcast())
        });
        let selection = gtk::SingleSelection::new(Some(tree));
        selection.set_autoselect(false);
        selection.set_can_unselect(true);
        selection.set_selected(gtk::INVALID_LIST_POSITION);

        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().expect("ListItem");
            let label = gtk::Label::builder()
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .build();
            let expander = gtk::TreeExpander::new();
            expander.set_child(Some(&label));
            item.set_child(Some(&expander));
        });
        factory.connect_bind(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().expect("ListItem");
            let Some(row) = item.item().and_downcast::<gtk::TreeListRow>() else {
                return;
            };
            let expander = item
                .child()
                .and_downcast::<gtk::TreeExpander>()
                .expect("expander");
            let label = expander
                .child()
                .and_downcast::<gtk::Label>()
                .expect("label");
            expander.set_list_row(Some(&row));
            if let Some(entry) = row.item().as_ref().and_then(item_of) {
                label.set_label(&entry.title);
                label.set_tooltip_text(Some(&entry.title));
                // Items without a supported target are shown, but dimmed.
                label.set_opacity(if entry.target.is_some() { 1.0 } else { 0.55 });
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
