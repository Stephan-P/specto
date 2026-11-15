//! The list of a document's markup in the sidebar: every annotation with
//! what it is, its text and its page. Picking a row shows the annotation.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use specto_annot::{AnnotKind, Annotation, Color, MarkupStyle};
use specto_doc::AnnotId;

use crate::i18n::gettext;

/// One row: an annotation, where it is, and what the row says.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// The position of its page, and which annotation of it.
    pub at: (usize, AnnotId),
    /// What kind of markup it is, in words.
    pub kind: String,
    /// Its text: what was marked, a note's text, a comment. May be empty.
    pub text: String,
    /// The label of its page.
    pub page: String,
    pub color: Option<Color>,
}

impl Entry {
    pub fn new(at: (usize, AnnotId), annotation: &Annotation, page: String) -> Self {
        Self {
            at,
            kind: kind_name(&annotation.kind),
            text: annotation.contents.clone().unwrap_or_default(),
            page,
            color: annotation.color,
        }
    }
}

/// What a kind of markup is called in the list.
fn kind_name(kind: &AnnotKind) -> String {
    match kind {
        AnnotKind::TextMarkup { style, .. } => match style {
            MarkupStyle::Highlight => gettext("Highlight"),
            MarkupStyle::Underline | MarkupStyle::Squiggly => gettext("Underline"),
            MarkupStyle::StrikeOut => gettext("Strikethrough"),
        },
        AnnotKind::Ink { .. } => gettext("Drawing"),
        AnnotKind::Square { .. } => gettext("Rectangle"),
        AnnotKind::Circle { .. } => gettext("Ellipse"),
        AnnotKind::Line { .. } => gettext("Line"),
        AnnotKind::Note => gettext("Note"),
        AnnotKind::FreeText { .. } => gettext("Text"),
        AnnotKind::Stamp(_) => gettext("Stamp"),
        // What other programs made, by what the file calls it.
        AnnotKind::Other { subtype } => match subtype.as_str() {
            "Text" => gettext("Note"),
            "FreeText" => gettext("Text"),
            "Stamp" => gettext("Stamp"),
            "Polygon" | "PolyLine" => gettext("Shape"),
            "FileAttachment" => gettext("Attachment"),
            other => other.to_owned(),
        },
    }
}

pub struct MarkupList {
    pub widget: gtk::Stack,
    list: gtk::ListBox,
    /// What the rows show, in order.
    entries: Rc<RefCell<Vec<Entry>>>,
}

impl MarkupList {
    /// `on_activate` is called with the annotation of the row the user
    /// picks.
    pub fn new(on_activate: impl Fn((usize, AnnotId)) + 'static) -> Self {
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .css_classes(["navigation-sidebar"])
            .build();
        let entries: Rc<RefCell<Vec<Entry>>> = Rc::default();
        let shown = entries.clone();
        list.connect_row_activated(move |_, row| {
            let at = usize::try_from(row.index())
                .ok()
                .and_then(|i| shown.borrow().get(i).map(|e| e.at));
            if let Some(at) = at {
                on_activate(at);
            }
        });
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&list)
            .build();
        let empty = adw::StatusPage::builder()
            .icon_name("document-edit-symbolic")
            .title(gettext("No Markup"))
            .description(gettext("Highlights, drawings and notes are listed here"))
            .css_classes(["compact"])
            .build();
        let widget = gtk::Stack::new();
        widget.add_named(&empty, Some("empty"));
        widget.add_named(&scroller, Some("list"));
        Self {
            widget,
            list,
            entries,
        }
    }

    /// What the list shows.
    pub fn entries(&self) -> Vec<Entry> {
        self.entries.borrow().clone()
    }

    /// Shows `entries`, and marks the row of `picked`, if it has one.
    pub fn set(&self, entries: Vec<Entry>, picked: Option<(usize, AnnotId)>) {
        if *self.entries.borrow() != entries {
            self.list.remove_all();
            for entry in &entries {
                self.list.append(&row(entry));
            }
            self.widget
                .set_visible_child_name(if entries.is_empty() { "empty" } else { "list" });
            self.entries.replace(entries);
        }
        self.select(picked);
    }

    /// Marks the row of `picked`, or none.
    pub fn select(&self, picked: Option<(usize, AnnotId)>) {
        let index = picked.and_then(|at| self.entries.borrow().iter().position(|e| e.at == at));
        let row = index.and_then(|i| self.list.row_at_index(i as i32));
        match row {
            Some(row) => self.list.select_row(Some(&row)),
            None => self.list.unselect_all(),
        }
    }
}

/// A row: a dot in the annotation's colour, its text (or what it is, if it
/// has none) over what it is, and its page.
fn row(entry: &Entry) -> gtk::ListBoxRow {
    let dot = gtk::DrawingArea::builder()
        .content_width(10)
        .content_height(10)
        .valign(gtk::Align::Start)
        .margin_top(5)
        .build();
    let color = entry.color;
    dot.set_draw_func(move |area, cr, width, height| {
        // Without a colour of its own, a ring in the text colour.
        let (w, h) = (f64::from(width), f64::from(height));
        cr.arc(
            w / 2.0,
            h / 2.0,
            w.min(h) / 2.0 - 1.0,
            0.0,
            std::f64::consts::TAU,
        );
        match color {
            Some(c) => {
                cr.set_source_rgb(c.r.into(), c.g.into(), c.b.into());
                let _ = cr.fill_preserve();
                cr.set_source_rgba(0.0, 0.0, 0.0, 0.25);
            }
            None => {
                let c = area.color();
                cr.set_source_rgba(c.red().into(), c.green().into(), c.blue().into(), 0.5);
            }
        }
        cr.set_line_width(1.0);
        let _ = cr.stroke();
    });
    let has_text = !entry.text.trim().is_empty();
    let title = gtk::Label::builder()
        .label(if has_text { &entry.text } else { &entry.kind })
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .lines(3)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    let words = gtk::Box::new(gtk::Orientation::Vertical, 2);
    words.set_hexpand(true);
    words.append(&title);
    if has_text {
        title.set_tooltip_text(Some(&entry.text));
        words.append(
            &gtk::Label::builder()
                .label(&entry.kind)
                .xalign(0.0)
                .css_classes(["caption", "dim-label"])
                .build(),
        );
    }
    let page = gtk::Label::builder()
        .label(&entry.page)
        .valign(gtk::Align::Start)
        .css_classes(["dim-label", "numeric"])
        .build();
    let line = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .margin_top(8)
        .margin_bottom(8)
        .build();
    line.append(&dot);
    line.append(&words);
    line.append(&page);
    gtk::ListBoxRow::builder().child(&line).build()
}
