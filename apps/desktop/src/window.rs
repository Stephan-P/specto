//! A document window: header bar, thumbnail sidebar and page view.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use specto_annot::{AnnotType, Color, FontFamily, MarkupStyle, TextFont};
use specto_doc::{Edit, Rotation, SourceId};
use specto_geom::ViewPoint;
use specto_render::{OpenError, PdfData};
use specto_session::{DraftKind, TextBoxPlace};
use specto_view::{LayoutMode, Viewer, ZoomMode};

use crate::canvas::Canvas;
use crate::debug;
use crate::document::{Document, Indexed, MAIN, PageBundle, SaveJob, SharedDocument};
use crate::i18n::{gettext, gettext_f, ngettext_f};
use crate::markup_list::{Entry, MarkupList};
use crate::outline::OutlineView;
use crate::print;
use crate::signatures::{self, Signature};
use crate::state;
use crate::thumbnails::Sidebar;

/// Actions that need an open document.
const DOCUMENT_ACTIONS: &[&str] = &[
    "zoom-in",
    "zoom-out",
    "zoom-actual",
    "fit-width",
    "fit-page",
    "layout",
    "cover-alone",
    "go-to-page",
    "next-page",
    "previous-page",
    "first-page",
    "last-page",
    "copy",
    "select-all",
    "find",
    "find-next",
    "find-previous",
    "rotate-left",
    "rotate-right",
    "highlight",
    "underline",
    "strike-out",
    "show-markup",
    "markup-tool",
    "markup-color",
    "save",
    "save-as",
    "insert-file",
    "delete-pages",
    "export-pages",
    "reload",
    "print",
];

#[derive(Clone)]
pub struct Window(Rc<Inner>);

pub struct Inner {
    window: adw::ApplicationWindow,
    toasts: adw::ToastOverlay,
    stack: gtk::Stack,
    split: adw::OverlaySplitView,
    title: adw::WindowTitle,
    canvas: Canvas,
    page_entry: gtk::Entry,
    page_total: gtk::Label,
    zoom_button: gtk::MenuButton,
    /// Header bar widgets that only make sense with a document open.
    document_widgets: Vec<gtk::Widget>,
    document: RefCell<Option<SharedDocument>>,
    /// URI of the open file, for remembering its view state.
    uri: RefCell<Option<String>>,
    recent_title: gtk::Label,
    recent_list: gtk::ListBox,
    sidebar: RefCell<Option<Sidebar>>,
    /// The sidebar's pages: Thumbnails, Contents if the document has an outline, and Markup.
    sidebar_stack: RefCell<Option<adw::ViewStack>>,
    /// The list of the document's markup in the sidebar.
    markup_list: RefCell<Option<MarkupList>>,
    last_page: Cell<usize>,
    /// The last message shown as a toast, for tests.
    last_toast: RefCell<String>,
    search_bar: gtk::SearchBar,
    /// The bar with the markup tools and colors.
    markup_bar: gtk::Revealer,
    /// The controls for how text boxes are set, on the markup bar: all of them, and each.
    text_controls: gtk::Box,
    text_font: gtk::DropDown,
    text_bold: gtk::ToggleButton,
    text_italic: gtk::ToggleButton,
    text_size: gtk::SpinButton,
    /// The size and font new text boxes get.
    text_style: Cell<(f64, TextFont)>,
    /// The controls are being set to show a style, not by the user.
    showing_text_style: Cell<bool>,
    /// The dot on the color button of the markup bar: the color in use.
    color_swatch: gtk::Box,
    /// Lies over the page view, for typing text onto the page.
    page_overlay: gtk::Overlay,
    /// The list of saved signatures, under its button on the markup bar.
    signature_menu: gtk::Popover,
    /// The signature that the next click on a page places, if one was chosen.
    placing: RefCell<Option<Signature>>,
    /// The text field over the page, while a text box is typed into.
    typing: RefCell<Option<Typing>>,
    /// The editor for an annotation's text, while it is open.
    text_editor: RefCell<Option<gtk::Popover>>,
    /// The color of each tool of the markup bar (in the order of `TOOLS`), by name (see
    /// `markup_colors`).
    tool_colors: Cell<[&'static str; TOOLS.len()]>,
    search_entry: gtk::SearchEntry,
    /// "3 of 17", "No matches" and the like.
    search_status: gtk::Label,
    /// Undo and Redo in the main menu, named after the edit.
    undo_section: gio::Menu,
    /// A save is running in the background.
    saving: Cell<bool>,
    /// The user chose to close despite unsaved changes (or saved them).
    close_confirmed: Cell<bool>,
    /// The "Save changes?" dialog while it's open (for tests).
    alert: RefCell<Option<adw::AlertDialog>>,
    /// The file as it was when read or last saved, to tell changes by other programs from our own.
    known: RefCell<Option<Fingerprint>>,
    /// Watches the file for changes by other programs.
    monitor: RefCell<Option<gio::FileMonitor>>,
    /// A look at the file is scheduled (changes come in bursts).
    check_pending: Cell<bool>,
    /// The password the file was opened with, for reloading it.
    password: RefCell<Option<String>>,
    /// Says that the file changed on disk while there are edits, or is gone.
    banner: adw::Banner,
    /// How many print jobs finished (for tests).
    printed: Cell<u32>,
}

impl Window {
    pub fn new(app: &adw::Application) -> Self {
        let canvas = Canvas::new();
        canvas.add_controller(delete_keys());
        let scrolled = gtk::ScrolledWindow::builder()
            .child(&canvas)
            .hexpand(true)
            .vexpand(true)
            .build();

        // Over the page view: where text is typed onto the page (see `edit_text_box`). Above it,
        // the markup bar, added below.
        let page_overlay = gtk::Overlay::new();
        page_overlay.set_child(Some(&scrolled));
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.append(&page_overlay);
        let split = adw::OverlaySplitView::builder()
            .content(&content)
            .show_sidebar(true)
            .min_sidebar_width(170.0)
            .max_sidebar_width(260.0)
            .build();

        let empty = adw::StatusPage::builder()
            .icon_name("x-office-document-symbolic")
            .title(gettext("Open a PDF"))
            .description(gettext("Drop a file here or choose one"))
            .build();
        let open_button = gtk::Button::builder()
            .label(gettext("Open…"))
            .action_name("win.open")
            .halign(gtk::Align::Center)
            .css_classes(["pill", "suggested-action"])
            .build();
        let recent_list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["boxed-list"])
            .build();
        let recent_title = gtk::Label::builder()
            .label(gettext("Recent"))
            .xalign(0.0)
            .css_classes(["heading"])
            .build();
        let start = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .halign(gtk::Align::Center)
            .width_request(360)
            .build();
        start.append(&open_button);
        start.append(&recent_title);
        start.append(&recent_list);
        empty.set_child(Some(&start));

        let stack = gtk::Stack::new();
        stack.add_named(&empty, Some("empty"));
        stack.add_named(&split, Some("document"));
        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&stack));

        // Header bar.
        let title = adw::WindowTitle::new(crate::APP_NAME, "");
        let header = adw::HeaderBar::builder().title_widget(&title).build();
        let sidebar_toggle = gtk::ToggleButton::builder()
            .icon_name("sidebar-show-symbolic")
            .tooltip_text(gettext("Thumbnails (F9)"))
            .build();
        split
            .bind_property("show-sidebar", &sidebar_toggle, "active")
            .bidirectional()
            .sync_create()
            .build();
        header.pack_start(&sidebar_toggle);
        header.pack_start(
            &gtk::Button::builder()
                .icon_name("document-open-symbolic")
                .tooltip_text(gettext_f(
                    "Open ({shortcut})",
                    &[("shortcut", &primary("O"))],
                ))
                .action_name("win.open")
                .build(),
        );

        // The main menu: saving, printing, editing and About. Also on the start page, where only
        // About is enabled.
        let undo_section = gio::Menu::new();
        let main_menu = gio::Menu::new();
        main_menu.append_section(None, &{
            let m = gio::Menu::new();
            m.append(Some(&gettext("Save")), Some("win.save"));
            m.append(Some(&gettext("Save As…")), Some("win.save-as"));
            m.append(
                Some(&gettext("Export Selected Pages…")),
                Some("win.export-pages"),
            );
            m.append(Some(&gettext("Print…")), Some("win.print"));
            m
        });
        main_menu.append_section(None, &undo_section);
        main_menu.append_section(None, &{
            let m = gio::Menu::new();
            m.append(Some(&gettext("Rotate Left")), Some("win.rotate-left"));
            m.append(Some(&gettext("Rotate Right")), Some("win.rotate-right"));
            let delete =
                gio::MenuItem::new(Some(&gettext("Delete Pages")), Some("win.delete-pages"));
            // Shown only: the keys work through `delete_keys`.
            delete.set_attribute_value("accel", Some(&"Delete".to_variant()));
            m.append_item(&delete);
            m
        });
        main_menu.append_section(None, &{
            let m = gio::Menu::new();
            m.append(
                Some(&gettext("Insert Pages from File…")),
                Some("win.insert-file"),
            );
            m
        });
        main_menu.append_section(None, &{
            let m = gio::Menu::new();
            m.append(
                Some(&gettext_f("About {app}", &[("app", &crate::APP_NAME)])),
                Some("app.about"),
            );
            m
        });
        let main_button = gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .tooltip_text(gettext("Main Menu"))
            .menu_model(&main_menu)
            .primary(true)
            .build();
        header.pack_end(&main_button);

        let view_menu = gio::Menu::new();
        let layout_section = gio::Menu::new();
        for (label, target) in [
            (gettext("Single Page"), "single"),
            (gettext("Continuous"), "continuous"),
            (gettext("Two Pages"), "two-page"),
            (gettext("Two Pages, Continuous"), "two-page-continuous"),
        ] {
            layout_section.append(Some(&label), Some(&format!("win.layout::{target}")));
        }
        view_menu.append_section(None, &layout_section);
        view_menu.append_section(None, &{
            let m = gio::Menu::new();
            m.append(
                Some(&gettext("Show Cover Page Alone")),
                Some("win.cover-alone"),
            );
            m
        });
        let layout_button = gtk::MenuButton::builder()
            .icon_name("view-dual-symbolic")
            .tooltip_text(gettext("Page Layout"))
            .menu_model(&view_menu)
            .build();
        header.pack_end(&layout_button);

        let zoom_menu = gio::Menu::new();
        zoom_menu.append_section(None, &{
            let m = gio::Menu::new();
            m.append(Some(&gettext("Fit Width")), Some("win.fit-width"));
            m.append(Some(&gettext("Fit Page")), Some("win.fit-page"));
            m.append(Some(&gettext("Actual Size")), Some("win.zoom-actual"));
            m
        });
        zoom_menu.append_section(None, &{
            let m = gio::Menu::new();
            m.append(Some(&gettext("Zoom In")), Some("win.zoom-in"));
            m.append(Some(&gettext("Zoom Out")), Some("win.zoom-out"));
            m
        });
        let zoom_button = gtk::MenuButton::builder()
            .label("100%")
            .tooltip_text(gettext("Zoom"))
            .menu_model(&zoom_menu)
            .build();
        header.pack_end(&zoom_button);

        let page_entry = gtk::Entry::builder()
            .width_chars(4)
            .max_width_chars(6)
            .xalign(1.0)
            .input_purpose(gtk::InputPurpose::Digits)
            .tooltip_text(gettext_f(
                "Go to Page ({shortcut})",
                &[("shortcut", &primary("L"))],
            ))
            .build();
        let page_total = gtk::Label::new(None);
        page_total.add_css_class("dim-label");
        let page_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        page_box.append(&page_entry);
        page_box.append(&page_total);
        header.pack_end(&page_box);

        // Search: a bar under the header bar.
        let search_entry = gtk::SearchEntry::builder()
            .placeholder_text(gettext("Search"))
            .hexpand(true)
            .build();
        let search_status = gtk::Label::builder()
            .css_classes(["dim-label", "numeric"])
            .width_chars(10)
            .xalign(1.0)
            .build();
        let step_buttons = gtk::Box::builder().css_classes(["linked"]).build();
        step_buttons.append(
            &gtk::Button::builder()
                .icon_name("go-up-symbolic")
                .tooltip_text(gettext("Previous Match (Shift+Enter)"))
                .action_name("win.find-previous")
                .build(),
        );
        step_buttons.append(
            &gtk::Button::builder()
                .icon_name("go-down-symbolic")
                .tooltip_text(gettext("Next Match (Enter)"))
                .action_name("win.find-next")
                .build(),
        );
        let search_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        search_row.append(&search_entry);
        search_row.append(&search_status);
        search_row.append(&step_buttons);
        let search_bar = gtk::SearchBar::builder()
            .child(
                &adw::Clamp::builder()
                    .maximum_size(560)
                    .child(&search_row)
                    .build(),
            )
            .build();
        // Not `connect_entry`: then the bar focuses the entry again once it has slid in, which
        // selects what was typed meanwhile, and the next key replaces it. The find action focuses
        // it right away.
        let search_toggle = gtk::ToggleButton::builder()
            .icon_name("system-search-symbolic")
            .tooltip_text(gettext_f(
                "Search ({shortcut})",
                &[("shortcut", &primary("F"))],
            ))
            .build();
        search_bar
            .bind_property("search-mode-enabled", &search_toggle, "active")
            .bidirectional()
            .sync_create()
            .build();
        header.pack_end(&search_toggle);

        // The markup bar, under the header while the pencil is pressed in: what to mark text with,
        // and in which color. A tool that is pressed in marks every selection as it is made. The
        // colors also recolor the annotation that is picked.
        let tools = gtk::Box::builder().css_classes(["linked"]).build();
        for (label, tool, shortcut) in [
            (gettext("Highlight"), "highlight", "H"),
            (gettext("Underline"), "underline", "U"),
            (gettext("Strikethrough"), "strike-out", "X"),
        ] {
            let kind = tool_named(tool).unwrap_or(HIGHLIGHTER);
            tools.append(
                &gtk::ToggleButton::builder()
                    .child(&tool_icon(kind))
                    .tooltip_text(gettext_f(
                        "{tool}: marks the selected text, and what is selected from now on ({shortcut} marks once)",
                        &[
                            ("tool", &label),
                            ("shortcut", &primary(&format!("Shift+{shortcut}"))),
                        ],
                    ))
                    .action_name("win.markup-tool")
                    .action_target(&tool.to_variant())
                    .build(),
            );
        }
        // The drawing tools, as pictures of what they draw: words would make the bar too wide.
        let draw_tools = gtk::Box::builder().css_classes(["linked"]).build();
        for (tool, tip) in [
            ("draw", gettext("Draw freehand")),
            ("rectangle", gettext("Draw rectangles")),
            ("ellipse", gettext("Draw ellipses")),
            ("line", gettext("Draw lines")),
            ("text", gettext("Add text: click where it goes")),
            ("note", gettext("Add notes: click where one goes")),
        ] {
            let kind = tool_named(tool).unwrap_or(HIGHLIGHTER);
            draw_tools.append(
                &gtk::ToggleButton::builder()
                    .child(&tool_icon(kind))
                    .tooltip_text(tip)
                    .action_name("win.markup-tool")
                    .action_target(&tool.to_variant())
                    .build(),
            );
        }
        // The colors, under a button that shows the one in use.
        let colors = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        let color_menu = gtk::Popover::builder().child(&colors).build();
        for (name, label, _) in markup_colors() {
            let swatch = gtk::Box::builder()
                .css_classes(["markup-swatch", &format!("markup-{name}")])
                .valign(gtk::Align::Center)
                .halign(gtk::Align::Center)
                .build();
            let choose = gtk::ToggleButton::builder()
                .child(&swatch)
                .tooltip_text(label)
                .css_classes(["flat", "circular"])
                .action_name("win.markup-color")
                .action_target(&name.to_variant())
                .build();
            let menu = color_menu.downgrade();
            choose.connect_clicked(move |_| {
                if let Some(menu) = menu.upgrade() {
                    menu.popdown();
                }
            });
            colors.append(&choose);
        }
        let color_swatch = gtk::Box::builder()
            .css_classes(["markup-swatch", "markup-yellow"])
            .valign(gtk::Align::Center)
            .halign(gtk::Align::Center)
            .build();
        let color_button = gtk::MenuButton::builder()
            .child(&color_swatch)
            .tooltip_text(gettext("Color"))
            .popover(&color_menu)
            .build();
        let markup_row = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(10)
            .halign(gtk::Align::Center)
            .css_classes(["toolbar"])
            .build();
        markup_row.append(&tools);
        markup_row.append(&draw_tools);
        // Signatures: the saved ones to choose from, and making a new one.
        let signature_menu = gtk::Popover::new();
        let signature_button = gtk::MenuButton::builder()
            .child(&signature_icon())
            .tooltip_text(gettext("Signature"))
            .popover(&signature_menu)
            .build();
        markup_row.append(&signature_button);
        markup_row.append(&color_button);
        // How text boxes are set: shown with the text tool, and while a text box is picked or typed
        // into.
        let font_names = gtk::StringList::new(&[
            &gettext("Sans-serif"),
            &gettext("Serif"),
            &gettext("Monospace"),
        ]);
        // Each name in the kind of font it names.
        let in_its_font = gtk::SignalListItemFactory::new();
        in_its_font.connect_setup(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().expect("ListItem");
            item.set_child(Some(&gtk::Label::builder().xalign(0.0).build()));
        });
        in_its_font.connect_bind(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().expect("ListItem");
            let (Some(label), Some(name)) = (
                item.child().and_downcast::<gtk::Label>(),
                item.item().and_downcast::<gtk::StringObject>(),
            ) else {
                return;
            };
            label.set_label(&name.string());
            let class = ["font-sans", "font-serif", "font-mono"]
                .get(item.position() as usize)
                .copied()
                .unwrap_or("font-sans");
            label.set_css_classes(&[class]);
        });
        let text_font = gtk::DropDown::builder()
            .model(&font_names)
            .factory(&in_its_font)
            .tooltip_text(gettext("Font"))
            .build();
        let text_bold = gtk::ToggleButton::builder()
            .icon_name("format-text-bold-symbolic")
            .tooltip_text(gettext("Bold"))
            .build();
        let text_italic = gtk::ToggleButton::builder()
            .icon_name("format-text-italic-symbolic")
            .tooltip_text(gettext("Italic"))
            .build();
        let text_weight = gtk::Box::builder().css_classes(["linked"]).build();
        text_weight.append(&text_bold);
        text_weight.append(&text_italic);
        let text_size = gtk::SpinButton::with_range(6.0, 96.0, 1.0);
        text_size.set_value(12.0);
        // No wider than two digits need: the bar is full as it is.
        text_size.set_width_chars(2);
        text_size.set_max_width_chars(2);
        text_size.set_tooltip_text(Some(&gettext("Text size in points")));
        let text_controls = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        text_controls.append(&text_font);
        text_controls.append(&text_weight);
        text_controls.append(&text_size);
        text_controls.set_visible(false);
        markup_row.append(&text_controls);
        // Over the page view only: the sidebar keeps its full height.
        let markup_strip = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["markup-bar"])
            .build();
        // Never wider than the page view: with everything on it, the bar is wider than a small
        // window has room for, and it mustn't push the sidebar or the page about as its controls
        // come and go. What doesn't fit is scrolled to.
        let markup_scroller = gtk::ScrolledWindow::builder()
            .child(&markup_row)
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .build();
        markup_strip.append(&markup_scroller);
        let markup_bar = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::SlideDown)
            .child(&markup_strip)
            .build();
        content.prepend(&markup_bar);
        let markup_toggle = gtk::ToggleButton::builder()
            .icon_name("document-edit-symbolic")
            .tooltip_text(gettext("Markup"))
            .build();
        markup_bar
            .bind_property("reveal-child", &markup_toggle, "active")
            .bidirectional()
            .sync_create()
            .build();
        header.pack_end(&markup_toggle);
        load_css();

        let banner = adw::Banner::builder().action_name("win.reload").build();
        let view = adw::ToolbarView::new();
        view.add_top_bar(&header);
        view.add_top_bar(&search_bar);
        view.add_top_bar(&banner);
        view.set_content(Some(&toasts));

        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title(crate::APP_NAME)
            .default_width(1000)
            .default_height(800)
            .content(&view)
            .build();

        let this = Self(Rc::new(Inner {
            window,
            toasts,
            stack,
            split,
            title,
            canvas,
            page_entry,
            page_total,
            document_widgets: vec![
                sidebar_toggle.upcast(),
                search_toggle.upcast(),
                markup_toggle.upcast(),
                layout_button.upcast(),
                zoom_button.clone().upcast(),
                page_box.upcast(),
            ],
            zoom_button,
            document: RefCell::default(),
            uri: RefCell::default(),
            recent_title,
            recent_list,
            sidebar: RefCell::default(),
            sidebar_stack: RefCell::default(),
            markup_list: RefCell::default(),
            last_page: Cell::new(usize::MAX),
            last_toast: RefCell::default(),
            search_bar,
            markup_bar,
            text_controls,
            text_font,
            text_bold,
            text_italic,
            text_size,
            text_style: Cell::new((12.0, TextFont::default())),
            showing_text_style: Cell::new(false),
            color_swatch,
            page_overlay,
            signature_menu,
            placing: RefCell::default(),
            typing: RefCell::default(),
            text_editor: RefCell::default(),
            tool_colors: Cell::new([
                "yellow", "red", "red", "red", "red", "red", "red", "black", "yellow",
            ]),
            search_entry,
            search_status,
            undo_section,
            saving: Cell::new(false),
            close_confirmed: Cell::new(false),
            alert: RefCell::default(),
            known: RefCell::default(),
            monitor: RefCell::default(),
            check_pending: Cell::new(false),
            password: RefCell::default(),
            banner,
            printed: Cell::new(0),
        }));
        this.setup_actions();
        this.setup_page_entry();
        this.setup_search();
        this.setup_drop();
        if debug::enabled() {
            this.setup_debug_actions();
            DEBUG_WINDOWS.with_borrow_mut(|w| w.push(Rc::downgrade(&this.0)));
        }
        let weak = Rc::downgrade(&this.0);
        this.0.canvas.connect_view_changed(move || {
            if let Some(inner) = weak.upgrade() {
                Window(inner).view_changed();
            }
        });
        let weak = Rc::downgrade(&this.0);
        this.0.canvas.connect_picked(move || {
            if let Some(inner) = weak.upgrade() {
                let picked = inner
                    .document
                    .borrow()
                    .as_ref()
                    .and_then(|d| d.borrow().picked);
                if let Some(list) = inner.markup_list.borrow().as_ref() {
                    list.select(picked);
                }
                Window(inner).show_markup_color();
            }
        });
        this.fill_signature_menu();
        let weak = Rc::downgrade(&this.0);
        let chosen = move || {
            if let Some(inner) = weak.upgrade() {
                Window(inner).text_style_chosen();
            }
        };
        let f = chosen.clone();
        this.0.text_font.connect_selected_notify(move |_| f());
        let f = chosen.clone();
        this.0.text_bold.connect_toggled(move |_| f());
        let f = chosen.clone();
        this.0.text_italic.connect_toggled(move |_| f());
        this.0.text_size.connect_value_changed(move |_| chosen());
        // The text field for typing on the page goes where its text box is, as wide as the box and
        // as high as its text needs. (Left to itself, the overlay gives a text view no height at
        // all.)
        let weak = Rc::downgrade(&this.0);
        this.0
            .page_overlay
            .connect_get_child_position(move |_, widget| {
                let inner = weak.upgrade()?;
                let typing = inner.typing.borrow();
                let typing = typing
                    .as_ref()
                    .filter(|t| t.view.upcast_ref::<gtk::Widget>() == widget)?;
                let (x, y, width) = typing.area.get();
                let (_, measured, _, _) = widget.measure(gtk::Orientation::Vertical, width);
                // At least as high as the box will be for this text: the field is slow to say so
                // itself after its font has changed.
                let buffer = typing.view.buffer();
                let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
                let (size, font) = typing.style.get();
                let inside = (typing.place.width - 2.0 * specto_annot::TEXT_MARGIN).max(size);
                let lines = specto_annot::wrap(&text, size, inside, font).len();
                let zoom = f64::from(width) / typing.place.width.max(1.0);
                let needed = (specto_annot::box_height(lines, size, font) * zoom).ceil() as i32;
                let height = measured.max(needed);
                Some(gdk::Rectangle::new(x, y, width, height.max(1)))
            });
        let weak = Rc::downgrade(&this.0);
        this.0.canvas.connect_place(move |x, y| {
            if let Some(inner) = weak.upgrade() {
                Window(inner).place(x, y);
            }
        });
        let weak = Rc::downgrade(&this.0);
        this.0.canvas.connect_edit_text(move || {
            if let Some(inner) = weak.upgrade() {
                Window(inner).edit_text(None);
            }
        });
        let weak = Rc::downgrade(&this.0);
        this.0.canvas.connect_drawn(move || {
            if let Some(inner) = weak.upgrade() {
                Window(inner).annotate(|_| true);
            }
        });
        let weak = Rc::downgrade(&this.0);
        this.0.canvas.connect_selected(move || {
            if let Some(inner) = weak.upgrade() {
                Window(inner).selected();
            }
        });
        // Closing the bar lets go of its tool: nothing gets marked by a bar that isn't there.
        let weak = Rc::downgrade(&this.0);
        this.0.markup_bar.connect_reveal_child_notify(move |bar| {
            if !bar.reveals_child()
                && let Some(inner) = weak.upgrade()
            {
                Window(inner).set_markup_tool(None);
            }
        });
        let weak = Rc::downgrade(&this.0);
        this.0.canvas.connect_open_uri(move |uri| {
            if let Some(inner) = weak.upgrade() {
                Window(inner).open_uri(uri);
            }
        });
        this.set_document_actions_enabled(false);

        // Look at the file again when coming back to the window: file monitors miss changes on some
        // file systems, and in the Flatpak sandbox (the document portal) they miss changes made
        // outside it.
        let weak = Rc::downgrade(&this.0);
        this.0.window.connect_is_active_notify(move |window| {
            if let Some(inner) = weak.upgrade()
                && window.is_active()
            {
                Window(inner).schedule_check();
            }
        });

        // Ask about unsaved changes, and remember where the user was in this file.
        let weak = Rc::downgrade(&this.0);
        this.0.window.connect_close_request(move |_| {
            let Some(inner) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            let w = Window(inner);
            if w.is_modified() && !w.0.close_confirmed.get() {
                glib::spawn_future_local(async move { w.confirm_close().await });
                return glib::Propagation::Stop;
            }
            w.save_state();
            glib::Propagation::Proceed
        });

        this.refresh_recent();
        let weak = Rc::downgrade(&this.0);
        let handler = gtk::RecentManager::default().connect_changed(move |_| {
            if let Some(inner) = weak.upgrade() {
                Window(inner).refresh_recent();
            }
        });
        let handler = RefCell::new(Some(handler));
        this.0.window.connect_destroy(move |_| {
            if let Some(h) = handler.take() {
                gtk::RecentManager::default().disconnect(h);
            }
        });

        // Callbacks only hold weak references, so the GTK window owns the state and keeps it alive
        // until it is destroyed.
        let keep_alive = RefCell::new(Some(this.clone()));
        this.0.window.connect_destroy(move |_| {
            keep_alive.take();
        });
        this
    }

    pub fn present(&self) {
        self.0.window.present();
    }

    fn has_document(&self) -> bool {
        self.0.document.borrow().is_some()
    }

    /// Opens `file` here, or in a new window if this one already shows a document.
    pub fn open(&self, file: gio::File) {
        if self.has_document() {
            let app = self
                .0
                .window
                .application()
                .and_downcast::<adw::Application>();
            if let Some(app) = app {
                let w = Window::new(&app);
                w.present();
                w.open(file);
            }
            return;
        }
        let this = self.clone();
        glib::spawn_future_local(async move { this.load(file).await });
    }

    async fn load(&self, file: gio::File) {
        let name = display_name(&file).await;
        // Before reading: a change made meanwhile then shows as one.
        let known = fingerprint(&file).await;
        let data = match read_pdf(&file).await {
            Ok(data) => data,
            Err(e) => return self.toast(&couldnt_read(&name, &e)),
        };
        let Some((document, tiles, password)) = self.open_document(&name, data, None).await else {
            return;
        };
        self.set_document(Rc::new(RefCell::new(document)), tiles);
        let uri = file.uri().to_string();
        self.restore_state(&uri);
        gtk::RecentManager::default().add_item(&uri);
        self.0.uri.replace(Some(uri));
        self.0.password.replace(password);
        self.watch(&file, known);
    }

    /// Opens `data` as a document, asking for a password if it needs one (`password` is tried
    /// first).
    ///
    /// Returns the document, its tile notifications and the password that worked, or `None` if it
    /// couldn't be opened (with a message) or the password was canceled.
    async fn open_document(
        &self,
        name: &str,
        data: PdfData,
        mut password: Option<String>,
    ) -> Option<(Document, async_channel::Receiver<()>, Option<String>)> {
        let (tx, rx) = async_channel::unbounded::<()>();
        let mut asked = false;
        loop {
            let tx = tx.clone();
            let wake = move || {
                let _ = tx.try_send(());
            };
            match Document::open(name.to_owned(), data.clone(), password.as_deref(), wake) {
                Ok(d) => return Some((d, rx, password)),
                Err(OpenError::PasswordRequired) => {
                    password = Some(self.ask_password(name, asked).await?);
                    asked = true;
                }
                Err(e) => {
                    self.toast(&gettext_f(
                        "Couldn't open “{name}”: {error}",
                        &[("name", &name), ("error", &e)],
                    ));
                    return None;
                }
            }
        }
    }

    // --- the file on disk ---------------------------------------------------

    /// Starts watching `file`, as it was when read (`known`), for changes by other programs.
    fn watch(&self, file: &gio::File, known: Option<Fingerprint>) {
        let inner = &self.0;
        inner.known.replace(known);
        inner.banner.set_revealed(false);
        let monitor = file
            .monitor_file(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE)
            .ok();
        if let Some(m) = &monitor {
            let weak = Rc::downgrade(&self.0);
            m.connect_changed(move |_, _, _, _| {
                if let Some(inner) = weak.upgrade() {
                    Window(inner).schedule_check();
                }
            });
        }
        if let Some(old) = inner.monitor.replace(monitor) {
            old.cancel();
        }
    }

    /// Looks at the file shortly: a save by another program comes as several events.
    fn schedule_check(&self) {
        if self.0.check_pending.replace(true) {
            return;
        }
        let weak = Rc::downgrade(&self.0);
        glib::timeout_add_local_once(std::time::Duration::from_millis(300), move || {
            if let Some(inner) = weak.upgrade() {
                let w = Window(inner);
                glib::spawn_future_local(async move { w.check_disk().await });
            }
        });
    }

    /// Compares the file with what was read or saved.
    ///
    /// Unchanged, nothing happens. Changed, it's reloaded, or with edits a banner offers to. Gone,
    /// a banner says so.
    async fn check_disk(&self) {
        let inner = &self.0;
        inner.check_pending.set(false);
        if inner.saving.get() {
            // Our own save: looked at once it's done.
            return self.schedule_check();
        }
        let (Some(uri), Some(name)) = (
            inner.uri.borrow().clone(),
            inner
                .document
                .borrow()
                .as_ref()
                .map(|d| d.borrow().name.clone()),
        ) else {
            return;
        };
        let now = fingerprint(&gio::File::for_uri(&uri)).await;
        if now == *inner.known.borrow() {
            inner.banner.set_revealed(false);
            return;
        }
        let banner = &inner.banner;
        match now {
            None => {
                banner.set_title(&gettext_f(
                    "“{name}” was moved or deleted. Saving writes it again.",
                    &[("name", &name)],
                ));
                banner.set_button_label(None);
                banner.set_revealed(true);
            }
            Some(_) if !self.is_modified() => self.reload().await,
            Some(_) => {
                banner.set_title(&gettext_f(
                    "“{name}” was changed by another program. Reloading discards your edits.",
                    &[("name", &name)],
                ));
                banner.set_button_label(Some(&gettext("_Reload")));
                banner.set_revealed(true);
            }
        }
    }

    /// Whether the file changed on disk since it was read or saved (and is still there).
    async fn changed_on_disk(&self) -> bool {
        let Some(uri) = self.0.uri.borrow().clone() else {
            return false;
        };
        let now = fingerprint(&gio::File::for_uri(&uri)).await;
        now.is_some() && now != *self.0.known.borrow()
    }

    /// Reads the file again and shows it in place of the document, edits discarded, at the same
    /// place.
    async fn reload(&self) {
        let inner = &self.0;
        let (Some(uri), Some(doc)) = (inner.uri.borrow().clone(), inner.document.borrow().clone())
        else {
            return;
        };
        let file = gio::File::for_uri(&uri);
        let name = doc.borrow().name.clone();
        let known = fingerprint(&file).await;
        let data = match read_pdf(&file).await {
            Ok(data) => data,
            Err(e) => return self.toast(&couldnt_read(&name, &e)),
        };
        let password = inner.password.borrow().clone();
        let Some((document, tiles, password)) = self.open_document(&name, data, password).await
        else {
            return;
        };
        // The window may have moved on meanwhile.
        if !inner
            .document
            .borrow()
            .as_ref()
            .is_some_and(|d| Rc::ptr_eq(d, &doc))
        {
            return;
        }
        let view = doc.borrow().viewer.view_state();
        let sidebar_page = inner
            .sidebar_stack
            .borrow()
            .as_ref()
            .and_then(|s| s.visible_child_name());
        drop(doc);
        self.set_document(Rc::new(RefCell::new(document)), tiles);
        inner.canvas.update_viewer(|v| v.restore_view_state(&view));
        if let Some(page) = sidebar_page
            && inner.sidebar_stack.borrow().is_some()
        {
            gio::prelude::ActionGroupExt::activate_action(
                &inner.window,
                "sidebar-page",
                Some(&page.to_variant()),
            );
        }
        inner.password.replace(password);
        inner.known.replace(known);
        inner.banner.set_revealed(false);
        self.update_edit_state();
        if inner.search_bar.is_search_mode() {
            self.run_search();
        } else {
            self.update_search_status();
        }
    }

    /// Asks whether to save over a file another program changed. Returns whether to go ahead.
    async fn confirm_overwrite(&self, name: &str) -> bool {
        let inner = &self.0;
        let alert = adw::AlertDialog::new(
            Some(&gettext("File Changed on Disk")),
            Some(&gettext_f(
                "“{name}” was changed by another program after it was opened. Saving replaces those changes with this document.",
                &[("name", &name)],
            )),
        );
        alert.add_responses(&[
            ("cancel", &gettext("_Cancel")),
            ("save", &gettext("_Save Anyway")),
        ]);
        alert.set_response_appearance("save", adw::ResponseAppearance::Destructive);
        alert.set_default_response(Some("cancel"));
        alert.set_close_response("cancel");
        inner.alert.replace(Some(alert.clone()));
        let response = alert.choose_future(Some(&inner.window)).await;
        inner.alert.replace(None);
        response == "save"
    }

    /// Applies the view state saved for `uri`, if any.
    fn restore_state(&self, uri: &str) {
        let Some(saved) = state::load(uri) else {
            return;
        };
        let inner = &self.0;
        inner
            .canvas
            .update_viewer(|v| v.restore_view_state(&saved.view));
        if inner.sidebar_stack.borrow().is_some() {
            gio::prelude::ActionGroupExt::activate_action(
                &inner.window,
                "sidebar-page",
                Some(&saved.sidebar_page.to_variant()),
            );
        }
        inner.split.set_show_sidebar(saved.sidebar);
    }

    fn save_state(&self) {
        let inner = &self.0;
        let (Some(uri), Some(doc)) = (inner.uri.borrow().clone(), inner.document.borrow().clone())
        else {
            return;
        };
        let view = doc.borrow().viewer.view_state();
        let sidebar_page = inner
            .sidebar_stack
            .borrow()
            .as_ref()
            .and_then(|s| s.visible_child_name())
            .map_or_else(|| "thumbnails".to_owned(), |n| n.to_string());
        state::save(
            &uri,
            &state::Saved {
                view,
                sidebar: inner.split.shows_sidebar(),
                sidebar_page,
            },
        );
    }

    /// Lists recently opened PDFs on the start page.
    fn refresh_recent(&self) {
        let inner = &self.0;
        while let Some(row) = inner.recent_list.first_child() {
            inner.recent_list.remove(&row);
        }
        let mut items: Vec<gtk::RecentInfo> = gtk::RecentManager::default()
            .items()
            .into_iter()
            .filter(|i| i.mime_type() == "application/pdf" && i.exists())
            .collect();
        items.sort_by_key(|i| std::cmp::Reverse(i.modified().to_unix()));
        for info in items.into_iter().take(8) {
            let file = gio::File::for_uri(&info.uri());
            let subtitle = folder(&file)
                .and_then(|p| p.path())
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            let row = adw::ActionRow::builder()
                .title(glib::markup_escape_text(&info.display_name()))
                .subtitle(glib::markup_escape_text(&subtitle))
                .activatable(true)
                .build();
            row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
            let weak = Rc::downgrade(&self.0);
            row.connect_activated(move |_| {
                if let Some(inner) = weak.upgrade() {
                    Window(inner).open(file.clone());
                }
            });
            inner.recent_list.append(&row);
        }
        let any = inner.recent_list.first_child().is_some();
        inner.recent_list.set_visible(any);
        inner.recent_title.set_visible(any);
    }

    async fn ask_password(&self, name: &str, retry: bool) -> Option<String> {
        let body = if retry {
            gettext_f(
                "The password was incorrect. Enter the password for “{name}”.",
                &[("name", &name)],
            )
        } else {
            gettext_f("“{name}” is protected with a password.", &[("name", &name)])
        };
        let dialog = adw::AlertDialog::new(Some(&gettext("Password Required")), Some(&body));
        dialog.add_responses(&[
            ("cancel", &gettext("_Cancel")),
            ("unlock", &gettext("_Unlock")),
        ]);
        dialog.set_response_appearance("unlock", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("unlock"));
        dialog.set_close_response("cancel");
        let entry = gtk::PasswordEntry::builder()
            .show_peek_icon(true)
            .activates_default(true)
            .build();
        dialog.set_extra_child(Some(&entry));
        let response = dialog.choose_future(Some(&self.0.window)).await;
        (response == "unlock").then(|| entry.text().to_string())
    }

    fn set_document(&self, document: SharedDocument, tiles: async_channel::Receiver<()>) {
        let inner = &self.0;
        let name = document.borrow().name.clone();
        inner.window.set_title(Some(&name));
        inner.title.set_title(&name);

        let weak = Rc::downgrade(&self.0);
        let weak_move = Rc::downgrade(&self.0);
        let weak_files = Rc::downgrade(&self.0);
        let weak_pages = Rc::downgrade(&self.0);
        let sidebar = Sidebar::new(
            document.clone(),
            move |page| {
                if let Some(inner) = weak.upgrade() {
                    inner.canvas.update_viewer(|v| v.go_to_page(page));
                }
            },
            move |pages, to| {
                if let Some(inner) = weak_move.upgrade() {
                    Window(inner).move_pages(&pages, to);
                }
            },
            move |files, to| {
                if let Some(inner) = weak_files.upgrade() {
                    let w = Window(inner);
                    glib::spawn_future_local(async move { w.insert_files(files, Some(to)).await });
                }
            },
            move |bundle, to| {
                if let Some(inner) = weak_pages.upgrade() {
                    Window(inner).insert_bundle(&bundle, to);
                }
            },
        );
        sidebar.widget.add_controller(delete_keys());
        let stack = adw::ViewStack::new();
        stack.add_titled_with_icon(
            &sidebar.widget,
            Some("thumbnails"),
            &gettext("Thumbnails"),
            "view-grid-symbolic",
        );
        let outline = document.borrow().outline();
        if !outline.is_empty() {
            let weak = Rc::downgrade(&self.0);
            let labels = Rc::downgrade(&document);
            // Outline targets are pages of the file.
            let page_label = move |page: usize| {
                labels
                    .upgrade()
                    .and_then(|d| {
                        let d = d.borrow();
                        Some(d.page_label(d.position_of(MAIN, page)?))
                    })
                    .unwrap_or_default()
            };
            let outline = OutlineView::new(&outline, page_label, move |target| {
                if let Some(inner) = weak.upgrade() {
                    inner.canvas.follow(MAIN, target);
                }
            });
            stack.add_titled_with_icon(
                &outline.widget,
                Some("outline"),
                &gettext("Contents"),
                "view-list-symbolic",
            );
        }
        // Every annotation of the document. Filled when it is looked at: it reads the annotations
        // of every page.
        let weak = Rc::downgrade(&self.0);
        let markup = MarkupList::new(move |(position, id)| {
            if let Some(inner) = weak.upgrade() {
                inner.canvas.pick_and_show(position, id);
            }
        });
        stack.add_titled_with_icon(
            &markup.widget,
            Some("markup"),
            &gettext("Markup"),
            "document-edit-symbolic",
        );
        inner.markup_list.replace(Some(markup));
        let weak = Rc::downgrade(&self.0);
        stack.connect_visible_child_name_notify(move |_| {
            if let Some(inner) = weak.upgrade() {
                Window(inner).refresh_markup_list();
            }
        });
        let switcher = adw::ViewSwitcher::builder()
            .stack(&stack)
            .policy(adw::ViewSwitcherPolicy::Narrow)
            .margin_start(6)
            .margin_end(6)
            .margin_top(6)
            .margin_bottom(6)
            .build();
        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.append(&switcher);
        column.append(&stack);
        stack.set_vexpand(true);
        inner.split.set_sidebar(Some(&column));
        inner.sidebar_stack.replace(Some(stack));
        inner.sidebar.replace(Some(sidebar));
        inner.document.replace(Some(document.clone()));
        inner.canvas.set_document(Some(document.clone()));
        inner.stack.set_visible_child_name("document");
        self.set_document_actions_enabled(true);
        inner.canvas.grab_focus();
        self.view_changed();

        // Finished tiles go into the cache, then a redraw. Not holding on to the document: once
        // it's replaced (reloaded) and dropped, its channel closes.
        let weak = Rc::downgrade(&self.0);
        let weak_doc = Rc::downgrade(&document);
        drop(document);
        glib::spawn_future_local(async move {
            while tiles.recv().await.is_ok() {
                let (Some(inner), Some(document)) = (weak.upgrade(), weak_doc.upgrade()) else {
                    break;
                };
                {
                    let mut d = document.borrow_mut();
                    if !d.receive_tiles() {
                        continue;
                    }
                    d.flush_requests();
                }
                inner.canvas.queue_draw();
                if let Some(s) = inner.sidebar.borrow().as_ref() {
                    s.refresh();
                }
            }
        });
    }

    /// Updates header bar and sidebar after the view changed.
    fn view_changed(&self) {
        self.place_typing();
        let inner = &self.0;
        let Some((page, zoom, mode, cover)) = inner.canvas.with_viewer(|v| {
            (
                v.current_page(),
                v.zoom(),
                v.layout_mode(),
                v.settings().cover_alone,
            )
        }) else {
            return;
        };
        inner
            .zoom_button
            .set_label(&format!("{:.0}%", zoom * 100.0));
        if inner.last_page.replace(page) != page {
            if !inner.page_entry.has_focus() {
                self.show_page_number(page);
            }
            if let Some(s) = inner.sidebar.borrow().as_ref() {
                s.follow(page);
            }
        }
        let set_state = |name: &str, state: glib::Variant| {
            if let Some(a) = inner
                .window
                .lookup_action(name)
                .and_downcast::<gio::SimpleAction>()
            {
                a.set_state(&state);
            }
        };
        set_state("layout", mode.name().to_variant());
        set_state("cover-alone", cover.to_variant());
    }

    /// Opens a link from the document.
    ///
    /// Only web and mail links: a PDF shouldn't be able to launch local files or other URI
    /// handlers.
    fn open_uri(&self, uri: &str) {
        let scheme = uri.split(':').next().unwrap_or("").to_ascii_lowercase();
        if !matches!(scheme.as_str(), "http" | "https" | "mailto") {
            return self.toast(&gettext_f(
                "Links of this kind aren't opened: {address}",
                &[("address", &uri)],
            ));
        }
        let launcher = gtk::UriLauncher::new(uri);
        let this = self.clone();
        glib::spawn_future_local(async move {
            if let Err(e) = launcher.launch_future(Some(&this.0.window)).await {
                this.toast(&gettext_f(
                    "Couldn't open link: {error}",
                    &[("error", &e.message())],
                ));
            }
        });
    }

    fn toast(&self, message: &str) {
        self.0.last_toast.replace(message.to_owned());
        self.0.toasts.add_toast(adw::Toast::new(message));
    }

    // --- search -------------------------------------------------------

    fn setup_search(&self) {
        let inner = &self.0;
        let weak = Rc::downgrade(&self.0);
        inner.search_entry.connect_search_changed(move |_| {
            if let Some(inner) = weak.upgrade() {
                Window(inner).run_search();
            }
        });
        // Enter: next match. Ctrl+G and Shift+Ctrl+G come with the entry.
        let weak = Rc::downgrade(&self.0);
        inner.search_entry.connect_activate(move |_| {
            if let Some(inner) = weak.upgrade() {
                Window(inner).search_step(true);
            }
        });
        let weak = Rc::downgrade(&self.0);
        inner.search_entry.connect_next_match(move |_| {
            if let Some(inner) = weak.upgrade() {
                Window(inner).search_step(true);
            }
        });
        let weak = Rc::downgrade(&self.0);
        inner.search_entry.connect_previous_match(move |_| {
            if let Some(inner) = weak.upgrade() {
                Window(inner).search_step(false);
            }
        });
        // Escape closes the bar.
        let weak = Rc::downgrade(&self.0);
        inner.search_entry.connect_stop_search(move |_| {
            if let Some(inner) = weak.upgrade() {
                inner.search_bar.set_search_mode(false);
            }
        });
        // Shift+Enter: previous match.
        let keys = gtk::EventControllerKey::new();
        let weak = Rc::downgrade(&self.0);
        keys.connect_key_pressed(move |_, key, _, state| {
            let enter = matches!(key, gdk::Key::Return | gdk::Key::KP_Enter);
            if enter && state.contains(gdk::ModifierType::SHIFT_MASK) {
                if let Some(inner) = weak.upgrade() {
                    Window(inner).search_step(false);
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        inner.search_entry.add_controller(keys);
        // Closing the bar (Escape, the button) ends the search. Opening it again searches for
        // what's still in the entry.
        let weak = Rc::downgrade(&self.0);
        inner
            .search_bar
            .connect_search_mode_enabled_notify(move |bar| {
                let Some(inner) = weak.upgrade() else { return };
                let w = Window(inner);
                if bar.is_search_mode() {
                    w.run_search();
                } else {
                    if let Some(doc) = w.0.document.borrow().as_ref() {
                        doc.borrow_mut().end_search();
                    }
                    w.update_search_status();
                    w.0.canvas.queue_draw();
                    w.0.canvas.grab_focus();
                }
            });
    }

    /// Searches for the entry's text, goes to the first match from the current page on, and indexes
    /// the rest of the document if needed.
    fn run_search(&self) {
        let Some(doc) = self.0.document.borrow().clone() else {
            return;
        };
        let text = self.0.search_entry.text();
        let (found, indexer) = {
            let mut d = doc.borrow_mut();
            let found = d.search(&text);
            let from = d.viewer.current_page();
            let indexer = (!text.trim().is_empty())
                .then(|| d.start_indexing(from))
                .flatten();
            (found, indexer)
        };
        if found {
            self.reveal_match();
        }
        if let Some(rx) = indexer {
            self.receive_index(rx);
        }
        self.update_search_status();
        self.0.canvas.queue_draw();
    }

    /// Indexes pages the current search hasn't seen yet (inserted ones), if there is a search.
    fn index_for_search(&self) {
        let Some(doc) = self.0.document.borrow().clone() else {
            return;
        };
        let rx = {
            let mut d = doc.borrow_mut();
            if d.search_status().is_none() {
                return;
            }
            let from = d.viewer.current_page();
            d.start_indexing(from)
        };
        if let Some(rx) = rx {
            self.receive_index(rx);
        }
    }

    /// Takes in the indexer's pages as they come, updating the search.
    ///
    /// The pages are for the document shown now: if it's replaced (reloaded) meanwhile, the rest
    /// are dropped.
    fn receive_index(&self, rx: async_channel::Receiver<Indexed>) {
        let weak = Rc::downgrade(&self.0);
        let Some(for_doc) = self.0.document.borrow().as_ref().map(Rc::downgrade) else {
            return;
        };
        glib::spawn_future_local(async move {
            while let Ok(page) = rx.recv().await {
                let Some(inner) = weak.upgrade() else { break };
                let w = Window(inner);
                let Some(doc) = w.0.document.borrow().clone() else {
                    break;
                };
                if !std::ptr::eq(Rc::as_ptr(&doc), for_doc.as_ptr()) {
                    break;
                }
                let before = doc.borrow().search_status();
                let new_current = doc.borrow_mut().indexed(page);
                if new_current {
                    w.reveal_match();
                } else if doc.borrow().search_status() != before {
                    w.update_search_status();
                    w.0.canvas.queue_draw();
                }
            }
        });
    }

    fn search_step(&self, forward: bool) {
        let Some(doc) = self.0.document.borrow().clone() else {
            return;
        };
        if !self.0.search_bar.is_search_mode() {
            self.0.search_bar.set_search_mode(true);
            return;
        }
        if doc.borrow_mut().search_step(forward) {
            self.reveal_match();
        } else {
            self.0.window.error_bell();
        }
    }

    /// Scrolls the current match into view and updates the status.
    fn reveal_match(&self) {
        let Some(doc) = self.0.document.borrow().clone() else {
            return;
        };
        doc.borrow_mut().reveal_match();
        self.0.canvas.update_viewer(|_| {});
        self.update_search_status();
    }

    fn update_search_status(&self) {
        let inner = &self.0;
        let status = inner
            .document
            .borrow()
            .as_ref()
            .and_then(|d| d.borrow().search_status());
        let (text, none_found) = match status {
            None => (String::new(), false),
            Some(s) => {
                let done = s.pages_indexed == s.pages;
                match (s.matches, s.current) {
                    (0, _) if done => (gettext("No matches"), true),
                    (0, _) => (gettext("Searching…"), false),
                    (n, Some(c)) => {
                        let args: &[(&str, &dyn std::fmt::Display)] =
                            &[("current", &(c + 1)), ("total", &n)];
                        let text = if done {
                            gettext_f("{current} of {total}", args)
                        } else {
                            // Translators: the search is still running, so there may be
                            // more matches than {total}.
                            gettext_f("{current} of {total}…", args)
                        };
                        (text, false)
                    }
                    (n, None) => (format!("{n}{}", if done { "" } else { "…" }), false),
                }
            }
        };
        inner.search_status.set_label(&text);
        if none_found {
            inner.search_entry.add_css_class("error");
        } else {
            inner.search_entry.remove_css_class("error");
        }
    }

    /// Activates `action` on the focused text field, if one has the focus.
    fn forward_to_entry(&self, action: &str) -> bool {
        let Some(focus) = gtk::prelude::GtkWindowExt::focus(&self.0.window) else {
            return false;
        };
        // The text field for typing on the page: its own keys do these, if they get to it.
        if let Some(view) = focus.downcast_ref::<gtk::TextView>() {
            let buffer = view.buffer();
            match action {
                "selection.select-all" => {
                    buffer.select_range(&buffer.start_iter(), &buffer.end_iter());
                }
                "clipboard.copy" => buffer.copy_clipboard(&view.clipboard()),
                "text.undo" => buffer.undo(),
                "text.redo" => buffer.redo(),
                _ => return false,
            }
            return true;
        }
        if focus.downcast_ref::<gtk::Text>().is_none() {
            return false;
        }
        focus.activate_action(action, None).is_ok()
    }

    /// The test hooks described in `debug.rs`.
    fn setup_debug_actions(&self) {
        let string = String::static_variant_type();
        let action = |name: &str, f: fn(&Window, &str)| {
            let weak = Rc::downgrade(&self.0);
            gio::ActionEntry::builder(name)
                .parameter_type(Some(&string))
                .activate(move |_: &adw::ApplicationWindow, _, param| {
                    if let (Some(inner), Some(arg)) =
                        (weak.upgrade(), param.and_then(|p| p.get::<String>()))
                    {
                        f(&Window(inner), &arg);
                    }
                })
                .build()
        };
        self.0.window.add_action_entries([
            action("debug-state", |w, path| {
                if let Err(e) = std::fs::write(path, w.debug_state()) {
                    eprintln!("debug-state: {path}: {e}");
                }
            }),
            action("debug-screenshot", |w, path| {
                debug::screenshot(w.0.window.upcast_ref(), path)
            }),
            action("debug-maximize", |w, _| w.0.window.maximize()),
            // Save As without the file dialog.
            action("debug-save-as", |w, path| {
                let (w, file) = (w.clone(), gio::File::for_path(path));
                glib::spawn_future_local(async move {
                    w.save_as_file(file).await;
                });
            }),
            // Drops thumbnails without dragging: "0,1@450" drops pages 0 and 1 at window height
            // 450.
            action("debug-drop-thumbnails", |w, arg| {
                let Some((pages, y)) = arg.split_once('@') else {
                    return;
                };
                let pages = pages.split(',').filter_map(|p| p.parse().ok()).collect();
                if let (Some(s), Ok(y)) = (w.0.sidebar.borrow().as_ref(), y.parse()) {
                    s.drop_at(pages, &w.0.window, y);
                }
            }),
            // Drops files on the sidebar without dragging: "a.pdf|b.pdf@450" drops two files at
            // window height 450.
            action("debug-drop-files", |w, arg| {
                let Some((paths, y)) = arg.rsplit_once('@') else {
                    return;
                };
                let files = paths.split('|').map(gio::File::for_path).collect();
                if let (Some(s), Ok(y)) = (w.0.sidebar.borrow().as_ref(), y.parse()) {
                    s.drop_files_at(files, &w.0.window, y);
                }
            }),
            // Drops pages of another window without dragging: "1:0,2@450" drops pages 0 and 2 of
            // window 1 at window height 450.
            action("debug-drop-pages-from", |w, arg| {
                let parse = || {
                    let (window, rest) = arg.split_once(':')?;
                    let (pages, y) = rest.rsplit_once('@')?;
                    let pages: Vec<usize> =
                        pages.split(',').filter_map(|p| p.parse().ok()).collect();
                    Some((window.parse::<u32>().ok()?, pages, y.parse::<f64>().ok()?))
                };
                let Some((n, pages, y)) = parse() else {
                    return;
                };
                let from = DEBUG_WINDOWS.with_borrow(|ws| {
                    ws.iter()
                        .filter_map(std::rc::Weak::upgrade)
                        .find(|i| i.window.id() == n)
                });
                let Some(doc) = from.and_then(|i| i.document.borrow().clone()) else {
                    return;
                };
                let bundle = doc.borrow().bundle(&pages);
                if let Some(s) = w.0.sidebar.borrow().as_ref() {
                    s.drop_bundle_at(bundle, &w.0.window, y);
                }
            }),
            // Prints to a PDF file without the dialog: "out.pdf", or "out.pdf|selection" for the
            // pages selected in the sidebar.
            action("debug-print-to-file", |w, arg| {
                let (path, selection) = match arg.strip_suffix("|selection") {
                    Some(p) => (p, true),
                    None => (arg, false),
                };
                w.print(Some((path.into(), selection)));
            }),
            // Export Selected Pages without the file dialog.
            action("debug-export", |w, path| {
                let (w, file) = (w.clone(), gio::File::for_path(path));
                glib::spawn_future_local(async move {
                    let pages = w.pages_to_export();
                    w.export_to(&pages, file).await;
                });
            }),
            // Insert Pages from File without the file dialog.
            action("debug-insert", |w, path| {
                let (w, file) = (w.clone(), gio::File::for_path(path));
                glib::spawn_future_local(async move {
                    w.insert_file(file).await;
                });
            }),
            // Chooses the saved signature of that number, from 0, to be placed, as its row under
            // the signature button does.
            action("debug-place-signature", |w, number| {
                let saved = signatures::saved();
                if let Some((_, signature)) =
                    number.parse::<usize>().ok().and_then(|i| saved.get(i))
                {
                    w.place_signature(signature.clone());
                }
            }),
            // Sets the text controls of the markup bar, as by hand: words like "serif bold italic
            // 18".
            action("debug-text-style", |w, words| {
                for word in words.split_ascii_whitespace() {
                    match word {
                        "sans" => w.0.text_font.set_selected(0),
                        "serif" => w.0.text_font.set_selected(1),
                        "mono" => w.0.text_font.set_selected(2),
                        "bold" => w.0.text_bold.set_active(true),
                        "italic" => w.0.text_italic.set_active(true),
                        "regular" => {
                            w.0.text_bold.set_active(false);
                            w.0.text_italic.set_active(false);
                        }
                        size => {
                            if let Ok(size) = size.parse::<f64>() {
                                w.0.text_size.set_value(size);
                            }
                        }
                    }
                }
            }),
            // Shows the dialog for a new signature.
            action("debug-new-signature", |w, _| w.new_signature()),
            // Activates a row of the markup list, by its number from 0.
            action("debug-markup-row", |w, row| {
                let at = row.parse::<usize>().ok().and_then(|i| {
                    let list = w.0.markup_list.borrow();
                    list.as_ref()?.entries().get(i).map(|e| e.at)
                });
                if let Some((position, id)) = at {
                    w.0.canvas.pick_and_show(position, id);
                }
            }),
            // Answers the open "Save changes?" dialog.
            action("debug-alert-response", |w, response| {
                if let Some(alert) = w.0.alert.borrow().clone() {
                    alert.emit_by_name::<()>("response", &[&response]);
                    alert.force_close();
                }
            }),
            action("debug-go-to", |w, text| {
                let entry = &w.0.page_entry;
                entry.grab_focus();
                entry.set_text(text);
                entry.emit_activate();
            }),
        ]);
    }

    /// The window's state as JSON, for tests.
    fn debug_state(&self) -> String {
        let inner = &self.0;
        let mut json = debug::Json::default();
        json.str(
            "view",
            &inner.stack.visible_child_name().unwrap_or_default(),
        )
        .str("title", &inner.window.title().unwrap_or_default())
        .str("toast", &inner.last_toast.borrow())
        // The title of the dialog over the window, "About" for instance.
        .str(
            "dialog",
            &inner
                .window
                .visible_dialog()
                .map(|d| d.title().to_string())
                .unwrap_or_default(),
        )
        .bool("sidebar", inner.split.shows_sidebar())
        .str(
            "sidebar_page",
            &inner
                .sidebar_stack
                .borrow()
                .as_ref()
                .and_then(|s| s.visible_child_name())
                .unwrap_or_default(),
        )
        .raw("recent", &{
            // [title, subtitle] per row of the start page's recent files.
            let mut rows = Vec::new();
            let mut child = inner.recent_list.first_child();
            while let Some(c) = child {
                if let Some(row) = c.downcast_ref::<adw::ActionRow>() {
                    rows.push(format!(
                        "[{}, {}]",
                        debug::quote(&row.title()),
                        debug::quote(&row.subtitle().unwrap_or_default())
                    ));
                }
                child = c.next_sibling();
            }
            format!("[{}]", rows.join(", "))
        })
        .str("page_entry", &inner.page_entry.text())
        .str("page_total", &inner.page_total.label())
        .str("zoom_label", &inner.zoom_button.label().unwrap_or_default())
        .num("scale", inner.canvas.device_scale())
        .bool("complete", inner.canvas.is_complete())
        .bool("maximized", inner.window.is_maximized())
        .str("cursor", inner.canvas.cursor_name().unwrap_or(""))
        .str(
            "focus",
            &gtk::prelude::GtkWindowExt::focus(&inner.window)
                .map_or_else(String::new, |w| w.type_().name().to_owned()),
        )
        .str(
            "selection",
            &inner.canvas.selected_text().unwrap_or_default(),
        )
        .bool("markup_bar", inner.markup_bar.reveals_child())
        .num("signatures", signatures::saved().len() as f64)
        // The text controls: whether they show, and what they are set to, such as "serif bold 18".
        .bool("text_controls", inner.text_controls.is_visible())
        .str("text_style", &{
            let (size, font) = self.text_controls_style();
            let mut words = vec![
                match font.family {
                    FontFamily::Sans => "sans",
                    FontFamily::Serif => "serif",
                    FontFamily::Mono => "mono",
                }
                .to_owned(),
            ];
            if font.bold {
                words.push("bold".to_owned());
            }
            if font.italic {
                words.push("italic".to_owned());
            }
            words.push(format!("{size}"));
            words.join(" ")
        })
        .bool("placing_signature", inner.placing.borrow().is_some())
        .str(
            "markup_tool",
            &inner
                .window
                .action_state("markup-tool")
                .and_then(|s| s.get::<String>())
                .unwrap_or_default(),
        )
        .str(
            "markup_color",
            &inner
                .window
                .action_state("markup-color")
                .and_then(|s| s.get::<String>())
                .unwrap_or_default(),
        )
        .bool("search_bar", inner.search_bar.is_search_mode())
        // Mapped once the bar has started sliding in. Keys before that are dropped (GTK doesn't
        // deliver them to unmapped widgets).
        .bool("search_ready", inner.search_entry.is_mapped())
        .str("search_query", &inner.search_entry.text())
        .str("search_status", &inner.search_status.label());
        if let Some(s) = inner
            .document
            .borrow()
            .as_ref()
            .and_then(|d| d.borrow().search_status())
        {
            json.num("matches", s.matches as f64)
                .num("match", s.current.map_or(-1.0, |c| c as f64))
                .num("indexed", s.pages_indexed as f64);
        }
        if let Some(m) = inner
            .document
            .borrow()
            .as_ref()
            .and_then(|d| d.borrow().current_match())
        {
            json.num("match_page", m.page as f64);
        }
        // Where the canvas is in the window, and for the pages around the current one, the affine
        // map from PDF points to window coordinates: [page, a, b, c, d, e, f] with x' = a x + c y +
        // e, y' = b x + d y + f.
        let origin = inner
            .canvas
            .compute_point(&inner.window, &gtk::graphene::Point::new(0.0, 0.0))
            .map_or((0.0, 0.0), |p| (p.x() as f64, p.y() as f64));
        let (px, py) = inner.canvas.pointer();
        json.raw("canvas", &format!("[{}, {}]", origin.0, origin.1))
            .raw("pointer", &format!("[{px}, {py}]"));
        // Thumbnails on screen, [page, x, y, width, height] in the window, and the pages selected
        // in the sidebar.
        if let Some(s) = inner.sidebar.borrow().as_ref() {
            let shown: Vec<String> = s
                .visible(&inner.window)
                .iter()
                .map(|(p, r)| format!("[{p}, {}, {}, {}, {}]", r.x(), r.y(), r.width(), r.height()))
                .collect();
            let selected: Vec<String> = s.selected().iter().map(usize::to_string).collect();
            json.raw("thumbnails", &format!("[{}]", shown.join(", ")))
                .raw("thumbnails_selected", &format!("[{}]", selected.join(", ")));
        }
        let maps = inner.canvas.with_viewer(|v| {
            let current = v.current_page();
            (current.saturating_sub(2)..(current + 4).min(v.page_count()))
                .filter_map(|i| {
                    let at = |x, y| v.pdf_to_view(i, specto_geom::PdfPoint::new(x, y));
                    let (o, ex, ey) = (at(0.0, 0.0)?, at(1.0, 0.0)?, at(0.0, 1.0)?);
                    Some(format!(
                        "[{i}, {}, {}, {}, {}, {}, {}]",
                        ex.x - o.x,
                        ex.y - o.y,
                        ey.x - o.x,
                        ey.y - o.y,
                        o.x + origin.0,
                        o.y + origin.1
                    ))
                })
                .collect::<Vec<_>>()
                .join(", ")
        });
        json.raw("pages", &format!("[{}]", maps.unwrap_or_default()));
        let width = inner.window.width();
        json.num("width", width as f64)
            .num("height", inner.window.height() as f64);
        if let Some(doc) = inner.document.borrow().as_ref() {
            let d = doc.borrow();
            let pages: Vec<String> = d
                .edit()
                .pages()
                .iter()
                .map(|p| format!("[{}, {}, {}]", p.page, p.rotation.degrees(), p.source.0))
                .collect();
            // [page of the file, rotation in degrees, file] per position.
            json.raw("edit_pages", &format!("[{}]", pages.join(", ")))
                .bool("modified", d.edit().is_modified())
                .num("cache_tiles", d.cache.len() as f64)
                .num("cache_bytes", d.cache.bytes() as f64)
                .bool("saving", inner.saving.get())
                .str(
                    "alert",
                    &inner
                        .alert
                        .borrow()
                        .as_ref()
                        .map(|a| a.heading().unwrap_or_default().to_string())
                        .unwrap_or_default(),
                )
                .str("subtitle", &inner.title.subtitle())
                .num("printed", f64::from(inner.printed.get()))
                .str(
                    "banner",
                    &if inner.banner.is_revealed() {
                        inner.banner.title().to_string()
                    } else {
                        String::new()
                    },
                )
                .str(
                    "undo",
                    &d.edit().undo_edit().map(edit_name).unwrap_or_default(),
                )
                .str(
                    "redo",
                    &d.edit().redo_edit().map(edit_name).unwrap_or_default(),
                );
        }
        // The annotations of the current page (the file's and new ones), and whether one is picked.
        if let Some(doc) = inner.document.borrow().clone() {
            let mut d = doc.borrow_mut();
            let page = d.viewer.current_page();
            let count = d.annotations(page).len();
            json.num("annotations", count as f64)
                .bool("annotation_picked", d.picked.is_some())
                .str("annotation_text", &d.picked_text().unwrap_or_default())
                .bool(
                    "text_editor",
                    inner.text_editor.borrow().is_some() || inner.typing.borrow().is_some(),
                );
            // The text field for typing on the page: where the overlay is asked to put it (left,
            // top, width) and the size it has.
            if let Some(t) = inner.typing.borrow().as_ref() {
                let (x, y, w) = t.area.get();
                json.raw(
                    "typing",
                    &format!("[{x}, {y}, {w}, {}, {}]", t.view.width(), t.view.height()),
                );
            }
            // The rows of the markup list: [page, kind, text] each.
            let rows: Vec<String> = inner
                .markup_list
                .borrow()
                .as_ref()
                .map(MarkupList::entries)
                .unwrap_or_default()
                .iter()
                .map(|e| {
                    format!(
                        "[{}, {}, {}]",
                        debug::quote(&e.page),
                        debug::quote(&e.kind),
                        debug::quote(&e.text)
                    )
                })
                .collect();
            json.raw("markup_list", &format!("[{}]", rows.join(", ")));
        }
        inner.canvas.with_viewer(|v| {
            json.num("page", v.current_page() as f64)
                .num("page_count", v.page_count() as f64)
                .str("layout", v.layout_mode().name())
                .bool("cover_alone", v.settings().cover_alone)
                .str("zoom_mode", v.zoom_mode().name())
                .num("zoom", v.zoom());
        });
        json.finish()
    }

    /// The pages an edit applies to: those selected in the sidebar if there are several, else the
    /// current page (both pages of a spread).
    fn pages_to_edit(&self) -> Vec<usize> {
        let selected = self
            .0
            .sidebar
            .borrow()
            .as_ref()
            .map(Sidebar::selected)
            .unwrap_or_default();
        if selected.len() > 1 {
            return selected;
        }
        self.0
            .canvas
            .with_viewer(|v| v.current_spread().to_vec())
            .unwrap_or_default()
    }

    /// Deletes the pages selected in the sidebar if there are several, else the current page.
    ///
    /// The last page can't be deleted. Marks the selected text: a highlight, or a line under or
    /// through it, in that tool's color. Returns whether there was text to mark.
    fn mark(&self, style: MarkupStyle) -> bool {
        let color = color_named(self.0.tool_colors.get()[tool_index(Tool::Mark(style))]);
        self.annotate(|d| d.mark_selection(style, color))
    }

    /// The Highlight, Underline and Strikethrough actions: marks once.
    fn mark_once(&self, style: MarkupStyle) {
        if !self.mark(style) {
            self.toast(&gettext("Select the text to mark first"));
        }
    }

    /// The tool that is pressed in on the markup bar, if any.
    fn markup_tool(&self) -> Option<Tool> {
        let state = self.0.window.action_state("markup-tool")?;
        tool_named(state.str()?)
    }

    /// Presses a tool of the markup bar in (or none), and shows its color.
    fn set_markup_tool(&self, tool: Option<Tool>) {
        let name = tool.map_or("none", tool_name);
        self.0
            .window
            .change_action_state("markup-tool", &name.to_variant());
        self.show_markup_color();
        self.set_draw_tool();
    }

    /// Tells the page view what drags do: draw, with a drawing tool pressed in (in its color), else
    /// select text.
    fn set_draw_tool(&self) {
        let draw = match self.markup_tool() {
            Some(tool @ Tool::Draw(kind)) => {
                let color = color_named(self.0.tool_colors.get()[tool_index(tool)]);
                let width = if kind == DraftKind::Ink { 2.0 } else { 1.5 };
                Some((kind, color, width))
            }
            _ => None,
        };
        self.0.canvas.set_draw_tool(draw);
        let placing = self.0.placing.borrow().is_some();
        self.0
            .canvas
            .set_place_tool(placing || matches!(self.markup_tool(), Some(Tool::Text | Tool::Note)));
    }

    /// Fills the list of saved signatures under its button: each to be chosen or deleted, and a
    /// button to make a new one.
    fn fill_signature_menu(&self) {
        let inner = &self.0;
        let rows = gtk::Box::new(gtk::Orientation::Vertical, 6);
        for (name, signature) in signatures::saved() {
            let choose = gtk::Button::builder()
                .child(&signatures::preview(&signature))
                .tooltip_text(gettext("Place this signature: click where it goes"))
                .css_classes(["flat"])
                .build();
            let weak = Rc::downgrade(inner);
            choose.connect_clicked(move |_| {
                if let Some(inner) = weak.upgrade() {
                    inner.signature_menu.popdown();
                    Window(inner).place_signature(signature.clone());
                }
            });
            let delete = gtk::Button::builder()
                .icon_name("user-trash-symbolic")
                .tooltip_text(gettext("Delete this signature"))
                .valign(gtk::Align::Center)
                .css_classes(["flat"])
                .build();
            let weak = Rc::downgrade(inner);
            delete.connect_clicked(move |_| {
                signatures::delete(&name);
                if let Some(inner) = weak.upgrade() {
                    Window(inner).fill_signature_menu();
                }
            });
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            row.append(&choose);
            row.append(&delete);
            rows.append(&row);
        }
        let new = gtk::Button::with_label(&gettext("New Signature…"));
        let weak = Rc::downgrade(inner);
        new.connect_clicked(move |_| {
            if let Some(inner) = weak.upgrade() {
                inner.signature_menu.popdown();
                Window(inner).new_signature();
            }
        });
        rows.append(&new);
        inner.signature_menu.set_child(Some(&rows));
    }

    /// Shows the dialog for making a signature. Once it is saved, it is the one being placed.
    fn new_signature(&self) {
        let weak = Rc::downgrade(&self.0);
        signatures::new_signature(&self.0.window, move |_, signature| {
            if let Some(inner) = weak.upgrade() {
                let w = Window(inner);
                w.fill_signature_menu();
                w.place_signature(signature);
            }
        });
    }

    /// The next click on a page places `signature`. Lets go of the markup bar's tool.
    fn place_signature(&self, signature: Signature) {
        self.0.canvas.unpick_annotation();
        self.0.placing.replace(Some(signature));
        self.set_markup_tool(None);
        self.toast(&gettext("Click where the signature goes"));
    }

    /// The page was clicked at (`x`, `y`) with a placing tool: a note goes there and asks for its
    /// text, or text is asked for and becomes a text box there.
    fn place(&self, x: f64, y: f64) {
        let p = ViewPoint::new(x, y);
        // A signature chosen to be placed: once, here.
        if let Some(signature) = self.0.placing.take() {
            let (w, h) = signature.size();
            // 150 points wide, less for a tall one.
            let width = SIGNATURE_WIDTH.min(SIGNATURE_WIDTH * 0.6 * w / h);
            self.edit(|d| {
                let placed = match signature {
                    Signature::Drawn { strokes, size } => d.add_drawing(
                        p,
                        width,
                        &strokes,
                        size,
                        color_named("black"),
                        signatures::PEN * width / size.0,
                    ),
                    Signature::Photo(image) => d.add_stamp(p, width, image),
                };
                placed.then(Edited::default)
            });
            self.set_draw_tool();
            return;
        }
        match self.markup_tool() {
            Some(tool @ Tool::Note) => {
                let color = color_named(self.0.tool_colors.get()[tool_index(tool)]);
                if self.annotate(|d| d.add_note(p, color)) {
                    self.edit_text(None);
                }
            }
            Some(Tool::Text) => self.edit_text(Some(p)),
            _ => {}
        }
    }

    /// Shows a small editor for text: that of the picked annotation (what a text box shows, the
    /// text of a note, a comment on anything else), or, with `new_at`, of a text box to be made
    /// there. Closing it (a click elsewhere, Escape) keeps the text.
    fn edit_text(&self, new_at: Option<ViewPoint>) {
        let inner = &self.0;
        let Some(doc) = inner.document.borrow().clone() else {
            return;
        };
        // A text box is typed into where it is on the page. (Not on pages shown turned, where the
        // text would have to be typed sideways: those get the small editor below, like notes and
        // comments.)
        let on_page = match new_at {
            Some(p) => {
                let color = color_named(inner.tool_colors.get()[tool_index(Tool::Text)]);
                let style = inner.text_style.get();
                let place = doc.borrow().new_text_box_place(p, style, color);
                place.map(|place| (place, String::new(), true))
            }
            None => {
                let editing = doc.borrow_mut().begin_text_edit();
                editing.map(|(place, text)| (place, text, false))
            }
        };
        if let Some((place, text, new)) = on_page {
            return self.type_on_page(place, &text, new);
        }
        let (text, area) = match new_at {
            Some(p) => (
                String::new(),
                gdk::Rectangle::new(p.x as i32, p.y as i32, 1, 1),
            ),
            None => {
                let Some(text) = doc.borrow_mut().picked_text() else {
                    return;
                };
                let Some(area) = inner.canvas.picked_area() else {
                    return;
                };
                (text, area)
            }
        };
        let view = gtk::TextView::builder()
            .wrap_mode(gtk::WrapMode::WordChar)
            .top_margin(6)
            .bottom_margin(6)
            .left_margin(6)
            .right_margin(6)
            .build();
        view.buffer().set_text(&text);
        let scroller = gtk::ScrolledWindow::builder()
            .child(&view)
            .min_content_width(260)
            .min_content_height(110)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        let popover = gtk::Popover::builder().child(&scroller).build();
        popover.set_parent(&inner.canvas);
        popover.set_pointing_to(Some(&area));
        inner.text_editor.replace(Some(popover.clone()));
        let weak = Rc::downgrade(inner);
        let buffer = view.buffer();
        popover.connect_closed(move |popover| {
            let text = buffer
                .text(&buffer.start_iter(), &buffer.end_iter(), false)
                .to_string();
            // Later: the popover is still closing, and is taken off the page view once it has.
            let (weak, popover) = (weak.clone(), popover.clone());
            glib::idle_add_local_once(move || {
                popover.unparent();
                if let Some(inner) = weak.upgrade() {
                    inner.text_editor.replace(None);
                    Window(inner).set_text(new_at, &text);
                }
            });
        });
        popover.popup();
        view.grab_focus();
    }

    /// Puts a text field over the page where a text box is (or is to be), in the box's font, size
    /// and color, so that what is typed appears where it will stand. Escape, or a click elsewhere,
    /// keeps the text.
    fn type_on_page(&self, place: TextBoxPlace, text: &str, new: bool) {
        let inner = &self.0;
        self.finish_typing();
        let view = gtk::TextView::builder()
            .wrap_mode(gtk::WrapMode::WordChar)
            .accepts_tab(false)
            // Filling what the overlay gives it, which is the text box's place: aligned to a side,
            // it would be as narrow as a letter.
            .halign(gtk::Align::Fill)
            .valign(gtk::Align::Fill)
            .css_classes(["page-text"])
            .build();
        view.buffer().set_text(text);
        inner.page_overlay.add_overlay(&view);
        let keys = gtk::EventControllerKey::new();
        let weak = Rc::downgrade(inner);
        keys.connect_key_pressed(move |_, key, _, _| {
            if key != gdk::Key::Escape {
                return glib::Propagation::Proceed;
            }
            if let Some(inner) = weak.upgrade() {
                Window(inner).finish_typing();
            }
            glib::Propagation::Stop
        });
        view.add_controller(keys);
        // A click elsewhere takes the keyboard away: the text is kept. (Later: the focus is still
        // on its way.)
        let focus = gtk::EventControllerFocus::new();
        let weak = Rc::downgrade(inner);
        focus.connect_leave(move |_| {
            let weak = weak.clone();
            glib::idle_add_local_once(move || {
                let Some(inner) = weak.upgrade() else { return };
                // Not for the controls that set the text's size and font: those act on what is
                // being typed.
                let on_controls = gtk::prelude::GtkWindowExt::focus(&inner.window)
                    .is_some_and(|f| f.is_ancestor(&inner.text_controls));
                if !on_controls {
                    Window(inner).finish_typing();
                }
            });
        });
        view.add_controller(focus);
        inner.typing.replace(Some(Typing {
            view: view.clone(),
            style: Cell::new((place.size, place.font)),
            place,
            new,
            area: Cell::new((0, 0, 1)),
        }));
        self.show_text_style();
        self.place_typing();
        // The redraw first: the page without the box, if it had it.
        inner.canvas.queue_draw();
        view.grab_focus();
        let end = view.buffer().end_iter();
        view.buffer().place_cursor(&end);
    }

    /// Puts the text field where its text box is on screen now, at the size the text has there:
    /// when it is shown, and when the view moves.
    fn place_typing(&self) {
        let inner = &self.0;
        let typing = inner.typing.borrow();
        let (Some(typing), Some(doc)) = (typing.as_ref(), inner.document.borrow().clone()) else {
            return;
        };
        let Some((corner, zoom)) = doc.borrow().text_box_corner(&typing.place) else {
            return typing.view.set_visible(false);
        };
        let place = &typing.place;
        let (size, font) = typing.style.get();
        let view = &typing.view;
        view.set_visible(true);
        // Where the overlay puts it (see `Window::new`): its corner, and its width. Its height is
        // what its text needs at that width.
        typing.area.set((
            corner.x.round() as i32,
            corner.y.round() as i32,
            (place.width * zoom).round().max(1.0) as i32,
        ));
        view.queue_resize();
        let margin = (specto_annot::TEXT_MARGIN * zoom).round() as i32;
        view.set_left_margin(margin);
        view.set_right_margin(margin);
        // A line of the field is higher than the letters, half of the rest above them: less of a
        // margin on top makes up for it, so that the text stands where the box will have it.
        let above = (specto_annot::TEXT_MARGIN - 0.12 * size) * zoom;
        view.set_top_margin(above.max(0.0).round() as i32);
        view.set_bottom_margin(margin);
        let c = place.color;
        let byte = |v: f32| (v * 255.0).round() as u8;
        let rgb = format!("rgb({}, {}, {})", byte(c.r), byte(c.g), byte(c.b));
        // The box's font, or what stands in for it with the same widths, so that lines break where
        // they will in the box.
        let css = format!(
            ".page-text, .page-text text {{ background: none; color: {rgb}; caret-color: {rgb}; \
             font-family: {}; font-weight: {}; font-style: {}; \
             font-size: {:.2}px; line-height: 1.2; }}\n\
             .page-text {{ outline: 1px solid @accent_bg_color; outline-offset: 2px; border-radius: 2px; }}\n",
            font_stack(font.family),
            if font.bold { "bold" } else { "normal" },
            if font.italic { "italic" } else { "normal" },
            size * zoom
        );
        TYPING_CSS.with(|provider| {
            provider.load_from_string(&css);
            if let Some(display) = gdk::Display::default() {
                gtk::style_context_add_provider_for_display(
                    &display,
                    provider,
                    gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
                );
            }
        });
    }

    /// Ends typing on the page, if there is any: what was typed is the text box's text now (a new
    /// box, if there is text for one).
    fn finish_typing(&self) {
        let inner = &self.0;
        let Some(typing) = inner.typing.take() else {
            return;
        };
        let buffer = typing.view.buffer();
        let text = buffer
            .text(&buffer.start_iter(), &buffer.end_iter(), false)
            .to_string();
        inner.page_overlay.remove_overlay(&typing.view);
        // In the size and font it has now, which may have been changed while typing.
        let style = typing.style.get();
        let place = TextBoxPlace {
            size: style.0,
            font: style.1,
            ..typing.place.clone()
        };
        let mut failed = None;
        self.edit(|d| {
            if typing.new {
                d.add_text_box_at(&place, &text);
            } else if let Err(e) = d.end_text_edit(Some((&text, style))) {
                failed = Some(e);
            }
            // Always: the page is drawn with the box again.
            Some(Edited::default())
        });
        if let Some(e) = failed {
            self.toast(&gettext_f(
                "Couldn't change the annotation: {error}",
                &[("error", &save_error(e))],
            ));
        }
        self.show_markup_color();
        inner.canvas.grab_focus();
    }

    /// The text editor was closed with `text` in it: for the picked annotation, or for a new text
    /// box at `new_at`.
    fn set_text(&self, new_at: Option<ViewPoint>, text: &str) {
        let mut failed = None;
        let tool = Tool::Text;
        let color = color_named(self.0.tool_colors.get()[tool_index(tool)]);
        let style = self.0.text_style.get();
        self.edit(|d| {
            let changed = match new_at {
                Some(p) => d.add_text_box(p, text, style, color),
                None => d.set_picked_text(text).unwrap_or_else(|e| {
                    failed = Some(e);
                    false
                }),
            };
            changed.then(Edited::default)
        });
        if let Some(e) = failed {
            self.toast(&gettext_f(
                "Couldn't change the annotation: {error}",
                &[("error", &save_error(e))],
            ));
        }
        self.show_markup_color();
    }

    /// The size and font the text controls of the markup bar are set to.
    fn text_controls_style(&self) -> (f64, TextFont) {
        let inner = &self.0;
        let family = match inner.text_font.selected() {
            1 => FontFamily::Serif,
            2 => FontFamily::Mono,
            _ => FontFamily::Sans,
        };
        (
            inner.text_size.value(),
            TextFont {
                family,
                bold: inner.text_bold.is_active(),
                italic: inner.text_italic.is_active(),
            },
        )
    }

    /// Shows the text controls while they have something to act on (the text tool, a text box
    /// picked or typed into), set to that thing's size and font.
    fn show_text_style(&self) {
        let inner = &self.0;
        let typing = inner.typing.borrow().as_ref().map(|t| t.style.get());
        let picked = inner
            .document
            .borrow()
            .as_ref()
            .and_then(|d| d.borrow_mut().picked_text_style());
        let tool = self.markup_tool() == Some(Tool::Text);
        inner
            .text_controls
            .set_visible(tool || typing.is_some() || picked.is_some());
        let (size, font) = typing.or(picked).unwrap_or(inner.text_style.get());
        inner.showing_text_style.set(true);
        inner.text_font.set_selected(match font.family {
            FontFamily::Sans => 0,
            FontFamily::Serif => 1,
            FontFamily::Mono => 2,
        });
        inner.text_bold.set_active(font.bold);
        inner.text_italic.set_active(font.italic);
        inner.text_size.set_value(size);
        inner.showing_text_style.set(false);
    }

    /// A text control of the markup bar was changed: what is typed, or the picked text box, is set
    /// that way, and so are text boxes from now on.
    fn text_style_chosen(&self) {
        let inner = &self.0;
        if inner.showing_text_style.get() {
            return;
        }
        let style = self.text_controls_style();
        inner.text_style.set(style);
        let typing = inner.typing.borrow().as_ref().map(|t| {
            t.style.set(style);
            t.view.clone()
        });
        if let Some(view) = typing {
            self.place_typing();
            // Back to typing, unless a size is being typed in.
            let in_size = gtk::prelude::GtkWindowExt::focus(&inner.window)
                .is_some_and(|f| f.is_ancestor(&inner.text_size));
            if !in_size {
                view.grab_focus();
            }
            return;
        }
        let mut failed = None;
        self.edit(|d| match d.restyle_picked_text_box(style) {
            Ok(changed) => changed.then(Edited::default),
            Err(e) => {
                failed = Some(e);
                None
            }
        });
        if let Some(e) = failed {
            self.toast(&gettext_f(
                "Couldn't change the annotation: {error}",
                &[("error", &save_error(e))],
            ));
        }
    }

    /// Shows, on the markup bar, the color of the picked annotation if it is one of the bar's, else
    /// that of the tool pressed in (of the highlighter if none is).
    fn show_markup_color(&self) {
        let inner = &self.0;
        let picked = inner
            .document
            .borrow()
            .as_ref()
            .and_then(|d| d.borrow_mut().picked_color())
            .map(|c| {
                markup_colors()
                    .into_iter()
                    .find(|(_, _, known)| *known == c)
                    .map_or("other", |(name, _, _)| name)
            });
        let tool = self.markup_tool().unwrap_or(HIGHLIGHTER);
        let name = picked.unwrap_or(inner.tool_colors.get()[tool_index(tool)]);
        inner
            .window
            .change_action_state("markup-color", &name.to_variant());
        inner
            .color_swatch
            .set_css_classes(&["markup-swatch", &format!("markup-{name}")]);
        self.show_text_style();
    }

    /// A color of the markup bar was chosen: the picked annotation gets it, if one is picked.
    ///
    /// Otherwise it is what the tool pressed in (the highlighter if none is) marks in from now on.
    fn choose_markup_color(&self, name: &'static str) {
        let inner = &self.0;
        let picked = inner
            .document
            .borrow()
            .as_ref()
            .is_some_and(|d| d.borrow().picked.is_some());
        if picked {
            let mut failed = None;
            self.edit(|d| match d.recolor_picked_annotation(color_named(name)) {
                Ok(changed) => changed.then(Edited::default),
                Err(e) => {
                    failed = Some(e);
                    None
                }
            });
            if let Some(e) = failed {
                self.toast(&gettext_f(
                    "Couldn't change the annotation: {error}",
                    &[("error", &save_error(e))],
                ));
            }
        } else {
            let tool = self.markup_tool().unwrap_or(HIGHLIGHTER);
            let mut colors = inner.tool_colors.get();
            colors[tool_index(tool)] = name;
            inner.tool_colors.set(colors);
            self.set_draw_tool();
        }
        self.show_markup_color();
    }

    /// Text was selected with the pointer: a tool that is pressed in marks it.
    fn selected(&self) {
        if let Some(Tool::Mark(style)) = self.markup_tool() {
            self.mark(style);
        }
    }

    /// Applies an edit to the annotations, then redraws and updates the menus if it changed
    /// anything, which it returns.
    ///
    /// The pages stay as they are, so there is less to do than after [`edit`](Self::edit).
    fn annotate(&self, f: impl FnOnce(&mut Document) -> bool) -> bool {
        let inner = &self.0;
        let Some(doc) = inner.document.borrow().clone() else {
            return false;
        };
        let changed = f(&mut doc.borrow_mut());
        if changed {
            inner.canvas.queue_draw();
            if let Some(s) = inner.sidebar.borrow().as_ref() {
                s.refresh();
            }
            self.update_edit_state();
        }
        changed
    }

    /// Delete and Backspace: the annotation that is picked, else pages.
    fn delete_pages(&self) {
        let mut failed = None;
        let mut removed = false;
        // As after an edit to pages: the thumbnail of the page changes too.
        self.edit(|d| {
            match d.remove_picked_annotation() {
                Ok(r) => removed = r,
                Err(e) => failed = Some(e),
            }
            removed.then(Edited::default)
        });
        if let Some(e) = failed {
            return self.toast(&gettext_f(
                "Couldn't delete the annotation: {error}",
                &[("error", &save_error(e))],
            ));
        }
        if removed {
            return;
        }
        let selected = self
            .0
            .sidebar
            .borrow()
            .as_ref()
            .map(Sidebar::selected)
            .unwrap_or_default();
        let pages = if selected.len() > 1 {
            selected
        } else {
            match self.0.canvas.with_viewer(|v| v.current_page()) {
                Some(p) => vec![p],
                None => return,
            }
        };
        let mut deleted = false;
        self.edit(|d| {
            deleted = d.delete(&pages);
            deleted.then(Edited::default)
        });
        if !deleted {
            self.0.window.error_bell();
        }
    }

    /// Inserts pages of another document before position `to`.
    ///
    /// They're selected, and the view goes to the first of them.
    fn insert_bundle(&self, bundle: &PageBundle, to: usize) {
        let mut error = None;
        self.edit(|d| match d.insert_bundle(to, bundle) {
            Ok(range) => {
                let range = range?;
                Some(Edited {
                    go_to: Some(range.start),
                    select: range.collect(),
                })
            }
            Err(e) => {
                error = Some(e);
                None
            }
        });
        match error {
            Some(e) => self.toast(&gettext_f(
                "Couldn't copy the pages: {error}",
                &[("error", &e)],
            )),
            None => self.index_for_search(),
        }
    }

    /// Moves the pages at `pages` before position `to`.
    ///
    /// They stay selected, and the view stays on the page it showed.
    fn move_pages(&self, pages: &[usize], to: usize) {
        let Some(current) = self.0.canvas.with_viewer(|v| v.current_page()) else {
            return;
        };
        self.edit(|d| {
            let moved = d.move_pages(pages, to)?;
            Some(Edited {
                go_to: Some(position_after_move(pages, moved.start, current)),
                select: moved.collect(),
            })
        });
    }

    /// Whether the sidebar has the keyboard focus.
    fn sidebar_has_focus(&self) -> bool {
        let focus = gtk::prelude::GtkWindowExt::focus(&self.0.window);
        let sidebar = self.0.sidebar.borrow();
        matches!((focus, sidebar.as_ref()), (Some(f), Some(s)) if s.contains(&f))
    }

    /// Turns the pages to edit (see [`pages_to_edit`]) by `by`, unless a text field has the focus:
    /// there the shortcut (Ctrl+Left/Right) moves by a word, as it would without this action.
    ///
    /// [`pages_to_edit`]: Self::pages_to_edit
    fn rotate(&self, by: Rotation) {
        // In a text field, the keys move the cursor by words instead.
        let focus = gtk::prelude::GtkWindowExt::focus(&self.0.window);
        if let Some(text) = focus.filter(|f| f.is::<gtk::Text>() || f.is::<gtk::TextView>()) {
            let count = if by == Rotation::Cw90 { 1 } else { -1 };
            text.emit_by_name::<()>("move-cursor", &[&gtk::MovementStep::Words, &count, &false]);
            return;
        }
        let pages = self.pages_to_edit();
        // The pages stay selected, so they can be turned again.
        self.edit(|d| {
            d.rotate(&pages, by).then(|| Edited {
                select: pages.clone(),
                go_to: None,
            })
        });
    }

    /// Shows the print dialog for the document as edited (or prints to a PDF file without it: see
    /// `print::print`).
    fn print(&self, export: Option<(std::path::PathBuf, bool)>) {
        let inner = &self.0;
        let Some(doc) = inner.document.borrow().clone() else {
            return;
        };
        let job = {
            let d = doc.borrow();
            print::Job {
                pages: d.bundle(&(0..d.edit().page_count()).collect::<Vec<_>>()),
                name: d.name.clone(),
                current: d.viewer.current_page(),
                selected: inner
                    .sidebar
                    .borrow()
                    .as_ref()
                    .map(Sidebar::selected)
                    .unwrap_or_default(),
            }
        };
        let weak = Rc::downgrade(&self.0);
        print::print(job, &inner.window, export, move |outcome| {
            let Some(inner) = weak.upgrade() else {
                return;
            };
            let w = Window(inner);
            match outcome {
                print::Outcome::Done => w.0.printed.set(w.0.printed.get() + 1),
                print::Outcome::Cancelled => {}
                print::Outcome::Failed(e) => {
                    w.toast(&gettext_f("Couldn't print: {error}", &[("error", &e)]))
                }
            }
        });
    }

    /// Asks for a PDF and inserts its pages.
    async fn choose_insert_file(&self) {
        let dialog = pdf_dialog(&gettext("Insert Pages from File"));
        if let Some(folder) = self
            .0
            .uri
            .borrow()
            .as_ref()
            .and_then(|u| folder(&gio::File::for_uri(u)))
        {
            dialog.set_initial_folder(Some(&folder));
        }
        if let Ok(file) = dialog.open_future(Some(&self.0.window)).await {
            self.insert_file(file).await;
        }
    }

    /// Inserts all pages of `file` after the current page (or spread) and goes to the first of
    /// them.
    async fn insert_file(&self, file: gio::File) {
        self.insert_files(vec![file], None).await;
    }

    /// Inserts all pages of `files`, in order and as one edit, before position `at` (by default
    /// after the current page or spread), and goes to the first of them.
    ///
    /// A file that can't be opened is left out, with a message.
    async fn insert_files(&self, files: Vec<gio::File>, at: Option<usize>) {
        let Some(doc) = self.0.document.borrow().clone() else {
            return;
        };
        let mut sources = Vec::new();
        for file in files {
            if let Some(source) = self.open_source(&doc, &file).await {
                sources.push(source);
            }
        }
        // The window may have moved on to another document meanwhile.
        if sources.is_empty()
            || !self
                .0
                .document
                .borrow()
                .as_ref()
                .is_some_and(|d| Rc::ptr_eq(d, &doc))
        {
            return;
        }
        let at = at.unwrap_or_else(|| {
            self.0
                .canvas
                .with_viewer(|v| v.current_spread().last().map_or(0, |&p| p + 1))
                .unwrap_or(0)
        });
        // The new pages are selected, ready to be moved or turned.
        self.edit(|d| {
            let inserted = d.insert(at, &sources)?;
            Some(Edited {
                go_to: Some(inserted.start),
                select: inserted.collect(),
            })
        });
        self.index_for_search();
    }

    /// Reads and opens `file` as a source of `doc`, asking for its password if it has one.
    ///
    /// `None` if it couldn't be opened (with a message) or the password was canceled.
    async fn open_source(&self, doc: &SharedDocument, file: &gio::File) -> Option<SourceId> {
        let name = display_name(file).await;
        let data = match read_pdf(file).await {
            Ok(data) => data,
            Err(e) => {
                self.toast(&couldnt_read(&name, &e));
                return None;
            }
        };
        let mut password: Option<String> = None;
        loop {
            let opened = doc
                .borrow_mut()
                .add_source(data.clone(), password.as_deref());
            match opened {
                Ok((source, _)) => return Some(source),
                Err(OpenError::PasswordRequired) => {
                    password = Some(self.ask_password(&name, password.is_some()).await?);
                }
                Err(e) => {
                    self.toast(&gettext_f(
                        "Couldn't open “{name}”: {error}",
                        &[("name", &name), ("error", &e)],
                    ));
                    return None;
                }
            }
        }
    }

    /// The pages "Export Selected Pages" writes: those selected in the sidebar, else the current
    /// page.
    fn pages_to_export(&self) -> Vec<usize> {
        let selected = self
            .0
            .sidebar
            .borrow()
            .as_ref()
            .map(Sidebar::selected)
            .unwrap_or_default();
        if !selected.is_empty() {
            return selected;
        }
        self.0
            .canvas
            .with_viewer(|v| vec![v.current_page()])
            .unwrap_or_default()
    }

    /// Asks where to save the selected pages, and saves them there as a new PDF.
    async fn export_pages(&self) {
        let inner = &self.0;
        let Some(name) = inner
            .document
            .borrow()
            .as_ref()
            .map(|d| d.borrow().name.clone())
        else {
            return;
        };
        let pages = self.pages_to_export();
        let dialog = gtk::FileDialog::builder()
            .title(gettext("Export Selected Pages"))
            .initial_name(export_name(&name, &pages))
            .build();
        if let Some(folder) = inner
            .uri
            .borrow()
            .as_ref()
            .and_then(|u| folder(&gio::File::for_uri(u)))
        {
            dialog.set_initial_folder(Some(&folder));
        }
        if let Ok(file) = dialog.save_future(Some(&inner.window)).await {
            self.export_to(&pages, file).await;
        }
    }

    /// Saves the pages at `pages` as a new PDF in `file`. The document and its file stay as they
    /// are.
    async fn export_to(&self, pages: &[usize], file: gio::File) {
        let Some(doc) = self.0.document.borrow().clone() else {
            return;
        };
        let job = {
            let d = doc.borrow();
            let refs = pages
                .iter()
                .filter_map(|&i| d.edit().pages().get(i).copied())
                .collect();
            d.save_pages(refs)
        };
        let n = pages.len();
        match write_job(job, &file).await {
            Ok(()) => self.toast(&ngettext_f(
                "Exported {n} page to “{name}”",
                "Exported {n} pages to “{name}”",
                n,
                &[("name", &file_name(&file))],
            )),
            Err(e) => self.toast(&couldnt_save(&file, &e)),
        }
    }

    fn is_modified(&self) -> bool {
        self.0
            .document
            .borrow()
            .as_ref()
            .is_some_and(|d| d.borrow().edit().is_modified())
    }

    /// Saves the document to its file, if it has changes. Returns whether it's saved now.
    async fn save(&self) -> bool {
        let Some(uri) = self.0.uri.borrow().clone() else {
            return false;
        };
        let file = gio::File::for_uri(&uri);
        // Unchanged, unless the file is gone: then it's written again.
        if !self.is_modified() && fingerprint(&file).await.is_some() {
            return true;
        }
        if self.changed_on_disk().await && !self.confirm_overwrite(&file_name(&file)).await {
            return false;
        }
        self.save_to(file).await
    }

    /// Asks where to save a copy, saves it there, and from then on shows that file.
    async fn save_as(&self) {
        let inner = &self.0;
        let Some(name) = inner
            .document
            .borrow()
            .as_ref()
            .map(|d| d.borrow().name.clone())
        else {
            return;
        };
        let dialog = gtk::FileDialog::builder()
            .title(gettext("Save As"))
            .initial_name(&name)
            .build();
        if let Some(folder) = inner
            .uri
            .borrow()
            .as_ref()
            .and_then(|u| folder(&gio::File::for_uri(u)))
        {
            dialog.set_initial_folder(Some(&folder));
        }
        if let Ok(file) = dialog.save_future(Some(&inner.window)).await {
            self.save_as_file(file).await;
        }
    }

    /// Saves to `file` and makes it the document's file.
    async fn save_as_file(&self, file: gio::File) -> bool {
        if !self.save_to(file.clone()).await {
            return false;
        }
        let inner = &self.0;
        let name = file.basename().map_or_else(
            || file.uri().to_string(),
            |b| b.to_string_lossy().into_owned(),
        );
        if let Some(doc) = inner.document.borrow().as_ref() {
            doc.borrow_mut().name = name.clone();
        }
        inner.window.set_title(Some(&name));
        inner.title.set_title(&name);
        let uri = file.uri().to_string();
        inner.uri.replace(Some(uri.clone()));
        gtk::RecentManager::default().add_item(&uri);
        self.save_state();
        let known = fingerprint(&file).await;
        self.watch(&file, known);
        true
    }

    /// Writes the document to `file`.
    ///
    /// Local files are written next to the old one and renamed into place (see
    /// `specto_write::replace_file`), others through GIO. Returns whether it worked, and says why
    /// if not.
    async fn save_to(&self, file: gio::File) -> bool {
        let inner = &self.0;
        let Some(doc) = inner.document.borrow().clone() else {
            return false;
        };
        if inner.saving.replace(true) {
            return false;
        }
        let job = doc.borrow().save_job();
        let version = job.version;
        let result = write_job(job, &file).await;
        if result.is_ok() {
            // What's on disk now is ours.
            inner.known.replace(fingerprint(&file).await);
            inner.banner.set_revealed(false);
        }
        inner.saving.set(false);
        match result {
            Ok(()) => {
                doc.borrow_mut().mark_saved(version);
                self.update_edit_state();
                true
            }
            Err(e) => {
                self.toast(&couldnt_save(&file, &e));
                false
            }
        }
    }

    /// Asks whether to save changes before closing, then closes (or not).
    async fn confirm_close(&self) {
        let inner = &self.0;
        if inner.alert.borrow().is_some() {
            return;
        }
        let name = inner
            .document
            .borrow()
            .as_ref()
            .map(|d| d.borrow().name.clone())
            .unwrap_or_default();
        let alert = adw::AlertDialog::new(
            Some(&gettext("Save Changes?")),
            Some(&gettext_f(
                "“{name}” has unsaved changes. Changes that aren't saved will be lost.",
                &[("name", &name)],
            )),
        );
        alert.add_responses(&[
            ("cancel", &gettext("_Cancel")),
            ("discard", &gettext("_Discard")),
            ("save", &gettext("_Save")),
        ]);
        alert.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
        alert.set_response_appearance("save", adw::ResponseAppearance::Suggested);
        alert.set_default_response(Some("save"));
        alert.set_close_response("cancel");
        inner.alert.replace(Some(alert.clone()));
        let response = alert.choose_future(Some(&inner.window)).await;
        inner.alert.replace(None);
        let close = match response.as_str() {
            "discard" => true,
            "save" => self.save().await,
            _ => false,
        };
        if close {
            inner.close_confirmed.set(true);
            inner.window.close();
        }
    }

    /// Applies an edit to the document, then updates the view, the sidebar and the menus if it
    /// changed anything. `f` returns `None` if nothing changed.
    fn edit(&self, f: impl FnOnce(&mut Document) -> Option<Edited>) {
        let inner = &self.0;
        let Some(doc) = inner.document.borrow().clone() else {
            return;
        };
        let Some(edited) = f(&mut doc.borrow_mut()) else {
            return;
        };
        if let Some(s) = inner.sidebar.borrow().as_ref() {
            s.pages_changed(&edited.select);
        }
        inner.last_page.set(usize::MAX);
        // Scrollbars, redraw, page number. In one update with going to a page, so the sidebar
        // doesn't follow a page in between.
        inner.canvas.update_viewer(|v| {
            if let Some(page) = edited.go_to {
                v.go_to_page(page);
            }
        });
        self.update_edit_state();
        self.update_search_status();
    }

    /// Fills the list of markup in the sidebar from the document, if it is what the sidebar shows:
    /// after an edit, and when it is turned to.
    fn refresh_markup_list(&self) {
        let inner = &self.0;
        let shown = inner
            .sidebar_stack
            .borrow()
            .as_ref()
            .and_then(|s| s.visible_child_name())
            .is_some_and(|name| name == "markup");
        let (Some(doc), true) = (inner.document.borrow().clone(), shown) else {
            return;
        };
        let mut d = doc.borrow_mut();
        let entries = d
            .all_annotations()
            .into_iter()
            .map(|(position, id, a)| Entry::new((position, id), &a, d.page_label(position)))
            .collect();
        let picked = d.picked;
        drop(d);
        if let Some(list) = inner.markup_list.borrow().as_ref() {
            list.set(entries, picked);
        }
    }

    /// Updates what depends on the edits: Undo and Redo, the title, the page count.
    fn update_edit_state(&self) {
        self.refresh_markup_list();
        let inner = &self.0;
        let Some(doc) = inner.document.borrow().clone() else {
            return;
        };
        let d = doc.borrow();
        let (undo, redo) = (d.edit().undo_edit(), d.edit().redo_edit());
        for (name, edit) in [("undo", undo), ("redo", redo)] {
            if let Some(a) = inner
                .window
                .lookup_action(name)
                .and_downcast::<gio::SimpleAction>()
            {
                a.set_enabled(edit.is_some());
            }
        }
        let section = &inner.undo_section;
        section.remove_all();
        section.append(
            Some(&undo.map_or_else(|| gettext("Undo"), undo_label)),
            Some("win.undo"),
        );
        section.append(
            Some(&redo.map_or_else(|| gettext("Redo"), redo_label)),
            Some("win.redo"),
        );

        let pages = d.viewer.page_count();
        let subtitle = if d.edit().is_modified() {
            ngettext_f("{n} page — Edited", "{n} pages — Edited", pages, &[])
        } else {
            ngettext_f("{n} page", "{n} pages", pages, &[])
        };
        inner.title.set_subtitle(&subtitle);
        // With labels, `show_page_number` gives the position instead.
        if d.labels().is_none() {
            // Translators: follows the box with the page number: "3 of 120".
            let total = gettext_f("of {pages}", &[("pages", &pages)]);
            inner.page_total.set_label(&total);
        }
    }

    fn set_document_actions_enabled(&self, enabled: bool) {
        for w in &self.0.document_widgets {
            w.set_visible(enabled);
        }
        for name in DOCUMENT_ACTIONS {
            if let Some(a) = self
                .0
                .window
                .lookup_action(name)
                .and_downcast::<gio::SimpleAction>()
            {
                a.set_enabled(enabled);
            }
        }
        if enabled {
            self.update_edit_state();
        } else {
            for name in ["undo", "redo"] {
                if let Some(a) = self
                    .0
                    .window
                    .lookup_action(name)
                    .and_downcast::<gio::SimpleAction>()
                {
                    a.set_enabled(false);
                }
            }
        }
    }

    fn setup_actions(&self) {
        let window = &self.0.window;
        let weak = Rc::downgrade(&self.0);
        let with = move |f: fn(&Window)| {
            let weak = weak.clone();
            move |_: &adw::ApplicationWindow, _: &gio::SimpleAction, _: Option<&glib::Variant>| {
                if let Some(inner) = weak.upgrade() {
                    f(&Window(inner));
                }
            }
        };

        window.add_action_entries([
            gio::ActionEntry::builder("open")
                .activate(with(|w| w.choose_file()))
                .build(),
            gio::ActionEntry::builder("zoom-in")
                .activate(with(|w| {
                    w.0.canvas.update_viewer(|v| v.zoom_by(1.25, centre(v)))
                }))
                .build(),
            gio::ActionEntry::builder("zoom-out")
                .activate(with(|w| {
                    w.0.canvas.update_viewer(|v| v.zoom_by(0.8, centre(v)))
                }))
                .build(),
            gio::ActionEntry::builder("zoom-actual")
                .activate(with(|w| {
                    w.0.canvas.update_viewer(|v| v.set_zoom(1.0, centre(v)))
                }))
                .build(),
            gio::ActionEntry::builder("fit-width")
                .activate(with(|w| {
                    w.0.canvas
                        .update_viewer(|v| v.set_zoom_mode(ZoomMode::FitWidth))
                }))
                .build(),
            gio::ActionEntry::builder("fit-page")
                .activate(with(|w| {
                    w.0.canvas
                        .update_viewer(|v| v.set_zoom_mode(ZoomMode::FitPage))
                }))
                .build(),
            gio::ActionEntry::builder("go-to-page")
                .activate(with(|w| {
                    w.0.page_entry.grab_focus();
                }))
                .build(),
            // Copy and Select All act on the page entry when it has the focus (application
            // shortcuts would otherwise take its Ctrl+C), and on the document's text otherwise.
            gio::ActionEntry::builder("copy")
                .activate(with(|w| {
                    if !w.forward_to_entry("clipboard.copy") {
                        w.0.canvas.copy();
                    }
                }))
                .build(),
            gio::ActionEntry::builder("select-all")
                .activate(with(|w| {
                    if w.forward_to_entry("selection.select-all") {
                        return;
                    }
                    match w.0.sidebar.borrow().as_ref() {
                        Some(s) if w.sidebar_has_focus() => s.select_all(),
                        _ => w.0.canvas.select_all(),
                    }
                }))
                .build(),
            gio::ActionEntry::builder("find")
                .activate(with(|w| {
                    let inner = &w.0;
                    inner.search_bar.set_search_mode(true);
                    inner.search_entry.grab_focus();
                    inner.search_entry.select_region(0, -1);
                }))
                .build(),
            gio::ActionEntry::builder("find-next")
                .activate(with(|w| w.search_step(true)))
                .build(),
            gio::ActionEntry::builder("find-previous")
                .activate(with(|w| w.search_step(false)))
                .build(),
            gio::ActionEntry::builder("save")
                .activate(with(|w| {
                    let w = w.clone();
                    glib::spawn_future_local(async move {
                        w.save().await;
                    });
                }))
                .build(),
            gio::ActionEntry::builder("save-as")
                .activate(with(|w| {
                    let w = w.clone();
                    glib::spawn_future_local(async move { w.save_as().await });
                }))
                .build(),
            gio::ActionEntry::builder("insert-file")
                .activate(with(|w| {
                    let w = w.clone();
                    glib::spawn_future_local(async move { w.choose_insert_file().await });
                }))
                .build(),
            gio::ActionEntry::builder("print")
                .activate(with(|w| w.print(None)))
                .build(),
            gio::ActionEntry::builder("reload")
                .activate(with(|w| {
                    let w = w.clone();
                    glib::spawn_future_local(async move { w.reload().await });
                }))
                .build(),
            gio::ActionEntry::builder("export-pages")
                .activate(with(|w| {
                    let w = w.clone();
                    glib::spawn_future_local(async move { w.export_pages().await });
                }))
                .build(),
            gio::ActionEntry::builder("delete-pages")
                .activate(with(|w| w.delete_pages()))
                .build(),
            gio::ActionEntry::builder("highlight")
                .activate(with(|w| w.mark_once(MarkupStyle::Highlight)))
                .build(),
            gio::ActionEntry::builder("underline")
                .activate(with(|w| w.mark_once(MarkupStyle::Underline)))
                .build(),
            gio::ActionEntry::builder("strike-out")
                .activate(with(|w| w.mark_once(MarkupStyle::StrikeOut)))
                .build(),
            gio::ActionEntry::builder("rotate-left")
                .activate(with(|w| w.rotate(Rotation::Cw270)))
                .build(),
            gio::ActionEntry::builder("rotate-right")
                .activate(with(|w| w.rotate(Rotation::Cw90)))
                .build(),
            gio::ActionEntry::builder("undo")
                .activate(with(|w| {
                    if !w.forward_to_entry("text.undo") {
                        w.edit(|d| d.undo().map(|_| Edited::default()));
                    }
                }))
                .build(),
            gio::ActionEntry::builder("redo")
                .activate(with(|w| {
                    if !w.forward_to_entry("text.redo") {
                        w.edit(|d| d.redo().map(|_| Edited::default()));
                    }
                }))
                .build(),
            gio::ActionEntry::builder("next-page")
                .activate(with(|w| w.0.canvas.update_viewer(Viewer::next_page)))
                .build(),
            gio::ActionEntry::builder("previous-page")
                .activate(with(|w| w.0.canvas.update_viewer(Viewer::previous_page)))
                .build(),
            gio::ActionEntry::builder("first-page")
                .activate(with(|w| w.0.canvas.update_viewer(|v| v.go_to_page(0))))
                .build(),
            gio::ActionEntry::builder("last-page")
                .activate(with(|w| {
                    w.0.canvas
                        .update_viewer(|v| v.go_to_page(v.page_count().saturating_sub(1)))
                }))
                .build(),
        ]);

        let weak = Rc::downgrade(&self.0);
        window.add_action_entries([gio::ActionEntry::builder("layout")
            .parameter_type(Some(&String::static_variant_type()))
            .state("continuous".to_variant())
            .activate(move |_, action, param| {
                let (Some(inner), Some(name)) =
                    (weak.upgrade(), param.and_then(|p| p.get::<String>()))
                else {
                    return;
                };
                let Some(mode) = LayoutMode::from_name(&name) else {
                    return;
                };
                action.set_state(&name.to_variant());
                inner.canvas.update_viewer(|v| v.set_layout_mode(mode));
            })
            .build()]);
        // The markup bar: the tool pressed in ("none" for none, and activating the one that is in
        // lets go of it) and the color shown ("other" for a picked annotation in a color the bar
        // doesn't have).
        let weak = Rc::downgrade(&self.0);
        window.add_action_entries([gio::ActionEntry::builder("markup-tool")
            .parameter_type(Some(&String::static_variant_type()))
            .state("none".to_variant())
            .activate(move |_, action, param| {
                let (Some(inner), Some(name)) =
                    (weak.upgrade(), param.and_then(|p| p.get::<String>()))
                else {
                    return;
                };
                let w = Window(inner);
                let current = action.state().and_then(|s| s.get::<String>());
                let tool = tool_named(&name).filter(|_| current.as_deref() != Some(&name));
                // Taking up a tool is to make something new: what was picked is let go of, and the
                // bar shows the tool's color.
                if tool.is_some() {
                    w.0.canvas.unpick_annotation();
                    w.0.placing.replace(None);
                }
                w.set_markup_tool(tool);
                // What is selected already is marked at once.
                if let Some(Tool::Mark(style)) = tool {
                    w.mark(style);
                }
            })
            .change_state(|_, action, state| {
                if let Some(state) = state {
                    action.set_state(state);
                }
            })
            .build()]);
        let weak = Rc::downgrade(&self.0);
        window.add_action_entries([gio::ActionEntry::builder("markup-color")
            .parameter_type(Some(&String::static_variant_type()))
            .state("yellow".to_variant())
            .activate(move |_, _, param| {
                let (Some(inner), Some(name)) =
                    (weak.upgrade(), param.and_then(|p| p.get::<String>()))
                else {
                    return;
                };
                let known = markup_colors().into_iter().find(|(n, _, _)| *n == name);
                if let Some((name, _, _)) = known {
                    Window(inner).choose_markup_color(name);
                }
            })
            .change_state(|_, action, state| {
                if let Some(state) = state {
                    action.set_state(state);
                }
            })
            .build()]);
        let weak = Rc::downgrade(&self.0);
        window.add_action_entries([gio::ActionEntry::builder("cover-alone")
            .state(false.to_variant())
            .activate(move |_, action, _| {
                let Some(inner) = weak.upgrade() else { return };
                let on = !action
                    .state()
                    .and_then(|s| s.get::<bool>())
                    .unwrap_or(false);
                action.set_state(&on.to_variant());
                inner.canvas.update_viewer(|v| v.set_cover_alone(on));
            })
            .build()]);

        // Which sidebar page to show: "thumbnails", "outline" or "markup".
        let weak = Rc::downgrade(&self.0);
        window.add_action_entries([gio::ActionEntry::builder("sidebar-page")
            .parameter_type(Some(&String::static_variant_type()))
            .state("thumbnails".to_variant())
            .activate(move |_, action, param| {
                let (Some(inner), Some(page)) =
                    (weak.upgrade(), param.and_then(|p| p.get::<String>()))
                else {
                    return;
                };
                if let Some(stack) = inner.sidebar_stack.borrow().as_ref()
                    && stack.child_by_name(&page).is_some()
                {
                    stack.set_visible_child_name(&page);
                    inner.split.set_show_sidebar(true);
                    action.set_state(&page.to_variant());
                }
            })
            .build()]);

        window.add_action(&gio::PropertyAction::new(
            "show-sidebar",
            &self.0.split,
            "show-sidebar",
        ));
        window.add_action(&gio::PropertyAction::new(
            "show-markup",
            &self.0.markup_bar,
            "reveal-child",
        ));
    }

    /// Puts page `page`'s label (or number) into the page entry, plus "(3 of 120)" when the
    /// document has labels.
    fn show_page_number(&self, page: usize) {
        let inner = &self.0;
        let Some(doc) = inner.document.borrow().clone() else {
            return;
        };
        let d = doc.borrow();
        inner.page_entry.set_text(&d.page_label(page));
        if d.labels().is_some() {
            let total = gettext_f(
                // Translators: follows the box with a printed page number:
                // "iv (4 of 120)".
                "({position} of {pages})",
                &[("position", &(page + 1)), ("pages", &d.viewer.page_count())],
            );
            inner.page_total.set_label(&total);
        }
    }

    fn setup_page_entry(&self) {
        let entry = &self.0.page_entry;
        let weak = Rc::downgrade(&self.0);
        entry.connect_activate(move |e| {
            let Some(inner) = weak.upgrade() else { return };
            let target = inner
                .document
                .borrow()
                .as_ref()
                .and_then(|d| d.borrow().find_page(&e.text()));
            match target {
                Some(page) => {
                    inner.canvas.update_viewer(|v| v.go_to_page(page));
                    inner.canvas.grab_focus();
                }
                None => {
                    e.error_bell();
                    let current = inner.last_page.get();
                    Window(inner).show_page_number(current);
                }
            }
        });
        // Restore the current page number when focus leaves without Enter.
        let focus = gtk::EventControllerFocus::new();
        let weak = Rc::downgrade(&self.0);
        focus.connect_leave(move |_| {
            if let Some(inner) = weak.upgrade() {
                let current = inner.last_page.get();
                if current != usize::MAX {
                    Window(inner).show_page_number(current);
                }
            }
        });
        entry.add_controller(focus);
    }

    fn setup_drop(&self) {
        let target = gtk::DropTarget::new(gio::File::static_type(), gdk::DragAction::COPY);
        let weak = Rc::downgrade(&self.0);
        target.connect_drop(move |_, value, _, _| {
            let (Some(inner), Ok(file)) = (weak.upgrade(), value.get::<gio::File>()) else {
                return false;
            };
            Window(inner).open(file);
            true
        });
        self.0
            .window
            .content()
            .expect("content")
            .add_controller(target);
    }

    fn choose_file(&self) {
        let dialog = pdf_dialog(&gettext("Open PDF"));
        let this = self.clone();
        glib::spawn_future_local(async move {
            if let Ok(file) = dialog.open_future(Some(&this.0.window)).await {
                this.open(file);
            }
        });
    }
}

/// How a shortcut with the primary modifier is written in tooltips: "⌘O" on macOS, "Ctrl+O"
/// elsewhere (shortcuts use GTK's `<Primary>`, which maps the same way).
fn primary(key: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("⌘{key}")
    } else {
        // Translators: a keyboard shortcut as tooltips show it, the
        // Control key with another.
        gettext_f("Ctrl+{key}", &[("key", &key)])
    }
}

fn couldnt_read(name: &str, error: &dyn std::fmt::Display) -> String {
    gettext_f(
        "Couldn't read “{name}”: {error}",
        &[("name", &name), ("error", error)],
    )
}

fn couldnt_save(file: &gio::File, error: &dyn std::fmt::Display) -> String {
    gettext_f(
        "Couldn't save “{name}”: {error}",
        &[("name", &file_name(file)), ("error", error)],
    )
}

fn centre(v: &Viewer) -> ViewPoint {
    ViewPoint::new(v.viewport().width / 2.0, v.viewport().height / 2.0)
}

thread_local! {
    /// The windows, with the test hooks only: for those that reach into another window.
    static DEBUG_WINDOWS: RefCell<Vec<std::rc::Weak<Inner>>> = const { RefCell::new(Vec::new()) };
}

/// The colors of the markup bar: a name (for the action's state and the style class of the swatch),
/// what a tooltip calls it, and the color.
///
/// Light ones for highlighting first, then a strong red, which is what lines under and through text
/// start with, as in Preview.
fn markup_colors() -> [(&'static str, String, Color); 7] {
    [
        ("yellow", gettext("Yellow"), Color::new(1.0, 0.92, 0.23)),
        ("green", gettext("Green"), Color::new(0.56, 0.94, 0.64)),
        ("blue", gettext("Blue"), Color::new(0.6, 0.76, 0.95)),
        ("pink", gettext("Pink"), Color::new(0.97, 0.66, 0.82)),
        ("purple", gettext("Purple"), Color::new(0.86, 0.54, 0.87)),
        ("red", gettext("Red"), Color::new(0.88, 0.11, 0.14)),
        ("black", gettext("Black"), Color::new(0.14, 0.14, 0.14)),
    ]
}

/// The color of the markup bar called `name` (yellow if there is none).
fn color_named(name: &str) -> Color {
    let colors = markup_colors();
    colors
        .iter()
        .find(|(n, _, _)| *n == name)
        .map_or(colors[0].2, |(_, _, c)| *c)
}

/// A picture of what a drawing tool draws, for its button: there are no stock icons for these.
///
/// Drawn in the button's text color, so it follows the light and the dark style.
fn tool_icon(kind: Tool) -> gtk::DrawingArea {
    let icon = gtk::DrawingArea::builder()
        .content_width(16)
        .content_height(16)
        .valign(gtk::Align::Center)
        .halign(gtk::Align::Center)
        .build();
    icon.set_draw_func(move |area, cr, width, height| {
        let color = area.color();
        cr.set_source_rgba(
            color.red().into(),
            color.green().into(),
            color.blue().into(),
            color.alpha().into(),
        );
        cr.set_line_width(1.5);
        cr.set_line_cap(gtk::cairo::LineCap::Round);
        cr.set_line_join(gtk::cairo::LineJoin::Round);
        // In a square of 16, whatever size the button gives.
        cr.scale(f64::from(width) / 16.0, f64::from(height) / 16.0);
        match kind {
            // A stroke as of a pen: a wave.
            Tool::Draw(DraftKind::Ink) => {
                cr.move_to(1.5, 11.0);
                cr.curve_to(4.0, 1.0, 7.0, 1.0, 8.0, 8.0);
                cr.curve_to(9.0, 15.0, 12.0, 15.0, 14.5, 5.0);
            }
            Tool::Draw(DraftKind::Square) => cr.rectangle(1.75, 3.25, 12.5, 9.5),
            Tool::Draw(DraftKind::Circle) => {
                let _ = cr.save();
                cr.translate(8.0, 8.0);
                cr.scale(6.25, 4.75);
                cr.arc(0.0, 0.0, 1.0, 0.0, std::f64::consts::TAU);
                let _ = cr.restore();
            }
            Tool::Draw(DraftKind::Line) => {
                cr.move_to(2.5, 13.5);
                cr.line_to(13.5, 2.5);
            }
            // A capital T.
            Tool::Text => {
                cr.move_to(3.0, 3.0);
                cr.line_to(13.0, 3.0);
                cr.move_to(8.0, 3.0);
                cr.line_to(8.0, 13.5);
            }
            // A slip of paper with writing on it, like the mark on the page.
            Tool::Note => {
                cr.rectangle(2.25, 2.25, 11.5, 11.5);
                cr.move_to(5.0, 6.0);
                cr.line_to(11.0, 6.0);
                cr.move_to(5.0, 9.5);
                cr.line_to(9.0, 9.5);
            }
            // The text tools: a letter, with what the tool does to it.
            Tool::Mark(style) => {
                let letter = |cr: &gtk::cairo::Context, top: f64, bottom: f64| {
                    let bar = bottom - (bottom - top) * 0.36;
                    cr.move_to(4.0, bottom);
                    cr.line_to(8.0, top);
                    cr.line_to(12.0, bottom);
                    cr.move_to(5.4, bar);
                    cr.line_to(10.6, bar);
                };
                match style {
                    // A band of the marker behind it.
                    MarkupStyle::Highlight => {
                        let _ = cr.save();
                        cr.set_source_rgba(
                            color.red().into(),
                            color.green().into(),
                            color.blue().into(),
                            f64::from(color.alpha()) * 0.3,
                        );
                        cr.rectangle(1.0, 5.5, 14.0, 8.5);
                        let _ = cr.fill();
                        let _ = cr.restore();
                        letter(cr, 2.5, 12.5);
                    }
                    MarkupStyle::Underline | MarkupStyle::Squiggly => {
                        letter(cr, 1.5, 11.0);
                        cr.move_to(2.5, 14.5);
                        cr.line_to(13.5, 14.5);
                    }
                    MarkupStyle::StrikeOut => {
                        letter(cr, 2.5, 13.5);
                        cr.move_to(1.5, 7.5);
                        cr.line_to(14.5, 7.5);
                    }
                }
            }
        }
        let _ = cr.stroke();
    });
    icon
}

/// The picture on the signature button: a scribble over a line.
fn signature_icon() -> gtk::DrawingArea {
    let icon = gtk::DrawingArea::builder()
        .content_width(18)
        .content_height(16)
        .valign(gtk::Align::Center)
        .halign(gtk::Align::Center)
        .build();
    icon.set_draw_func(|area, cr, width, height| {
        let color = area.color();
        cr.set_source_rgba(
            color.red().into(),
            color.green().into(),
            color.blue().into(),
            color.alpha().into(),
        );
        cr.set_line_cap(gtk::cairo::LineCap::Round);
        cr.set_line_join(gtk::cairo::LineJoin::Round);
        cr.scale(f64::from(width) / 18.0, f64::from(height) / 16.0);
        cr.set_line_width(1.4);
        cr.move_to(2.0, 10.0);
        cr.curve_to(4.0, 0.0, 7.0, 0.0, 6.0, 9.0);
        cr.curve_to(5.5, 13.0, 9.0, 3.0, 10.5, 7.0);
        cr.curve_to(11.5, 10.0, 13.0, 9.0, 16.0, 6.0);
        let _ = cr.stroke();
        cr.set_line_width(1.0);
        cr.move_to(1.5, 14.0);
        cr.line_to(16.5, 14.0);
        let _ = cr.stroke();
    });
    icon
}

/// A tool of the markup bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tool {
    /// Marks text as it is selected.
    Mark(MarkupStyle),
    /// Draws where the pointer is dragged.
    Draw(DraftKind),
    /// Puts a text box where the page is clicked, once its text is typed.
    Text,
    /// Puts a note where the page is clicked.
    Note,
}

/// Typing into a text box on the page.
struct Typing {
    /// The text field over the page.
    view: gtk::TextView,
    /// Where the text box is.
    place: TextBoxPlace,
    /// The size and font of what is typed, which the markup bar can change meanwhile.
    style: Cell<(f64, TextFont)>,
    /// Whether the box is yet to be made, of what is typed.
    new: bool,
    /// Where the field is over the page view: its left and top edge, and its width.
    area: Cell<(i32, i32, i32)>,
}

thread_local! {
    /// The style of the text field for typing on the page, which follows the box's text and the
    /// zoom.
    static TYPING_CSS: gtk::CssProvider = gtk::CssProvider::new();
}

/// How wide a signature is when it is placed, in points.
const SIGNATURE_WIDTH: f64 = 150.0;

/// The fonts to ask the system for, for a family: the font PDF programs set it in, then others with
/// the same widths, then anything of the kind.
fn font_stack(family: FontFamily) -> &'static str {
    match family {
        FontFamily::Sans => {
            "Helvetica, \"Nimbus Sans\", Arimo, \"Liberation Sans\", Arial, sans-serif"
        }
        FontFamily::Serif => {
            "Times, \"Nimbus Roman\", Tinos, \"Liberation Serif\", \"Times New Roman\", serif"
        }
        FontFamily::Mono => {
            "Courier, \"Nimbus Mono PS\", Cousine, \"Liberation Mono\", \"Courier New\", monospace"
        }
    }
}

/// The tool whose color the bar shows while none is pressed in.
const HIGHLIGHTER: Tool = Tool::Mark(MarkupStyle::Highlight);

/// The markup bar's tools, as the `markup-tool` action names them.
const TOOLS: [(&str, Tool); 9] = [
    ("highlight", Tool::Mark(MarkupStyle::Highlight)),
    ("underline", Tool::Mark(MarkupStyle::Underline)),
    ("strike-out", Tool::Mark(MarkupStyle::StrikeOut)),
    ("draw", Tool::Draw(DraftKind::Ink)),
    ("rectangle", Tool::Draw(DraftKind::Square)),
    ("ellipse", Tool::Draw(DraftKind::Circle)),
    ("line", Tool::Draw(DraftKind::Line)),
    ("text", Tool::Text),
    ("note", Tool::Note),
];

/// Where a tool is in `TOOLS` (and its color in `Inner::tool_colors`).
fn tool_index(tool: Tool) -> usize {
    TOOLS.iter().position(|(_, t)| *t == tool).unwrap_or(0)
}

fn tool_name(tool: Tool) -> &'static str {
    TOOLS[tool_index(tool)].0
}

fn tool_named(name: &str) -> Option<Tool> {
    TOOLS.iter().find(|(n, _)| *n == name).map(|(_, t)| *t)
}

/// The app's own styles: the color swatches of the markup bar. Loaded once, however many windows
/// there are.
fn load_css() {
    static LOADED: std::sync::Once = std::sync::Once::new();
    LOADED.call_once(|| {
        let Some(display) = gdk::Display::default() else {
            return;
        };
        let mut css = String::from(
            ".markup-swatch { min-width: 18px; min-height: 18px; border-radius: 9999px; \
             box-shadow: inset 0 0 0 1px alpha(currentColor, 0.25); }\n\
             .markup-bar { background-color: @headerbar_bg_color; \
             box-shadow: inset 0 -1px alpha(currentColor, 0.12); }\n",
        );
        for (class, family) in [
            ("font-sans", FontFamily::Sans),
            ("font-serif", FontFamily::Serif),
            ("font-mono", FontFamily::Mono),
        ] {
            css.push_str(&format!(
                ".{class} {{ font-family: {}; }}\n",
                font_stack(family)
            ));
        }
        css.push_str("");
        for (name, _, c) in markup_colors() {
            let byte = |v: f32| (v * 255.0).round() as u8;
            css.push_str(&format!(
                ".markup-{name} {{ background-color: rgb({}, {}, {}); }}\n",
                byte(c.r),
                byte(c.g),
                byte(c.b)
            ));
        }
        let provider = gtk::CssProvider::new();
        provider.load_from_string(&css);
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    });
}

/// What an edit changed, for [`Window::edit`].
#[derive(Default)]
struct Edited {
    /// Pages to select in the sidebar (none: the current page).
    select: Vec<usize>,
    /// The page to show.
    go_to: Option<usize>,
}

/// Where the page at `page` is after moving those at `moved` so that they start at `start`.
fn position_after_move(moved: &[usize], start: usize, page: usize) -> usize {
    let mut moved = moved.to_vec();
    moved.sort_unstable();
    moved.dedup();
    if let Some(i) = moved.iter().position(|&m| m == page) {
        return start + i;
    }
    let rest = page - moved.iter().filter(|&&m| m < page).count();
    if rest >= start {
        rest + moved.len()
    } else {
        rest
    }
}

/// Delete and Backspace delete pages, on the canvas and in the sidebar.
///
/// Not shortcuts for the whole window, which would take the keys from text fields.
fn delete_keys() -> gtk::ShortcutController {
    let keys = gtk::ShortcutController::new();
    keys.add_shortcut(gtk::Shortcut::new(
        gtk::ShortcutTrigger::parse_string("Delete|KP_Delete|BackSpace"),
        Some(gtk::NamedAction::new("win.delete-pages")),
    ));
    keys
}

/// A file dialog for choosing a PDF.
fn pdf_dialog(title: &str) -> gtk::FileDialog {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some(&gettext("PDF documents")));
    filter.add_mime_type("application/pdf");
    filter.add_suffix("pdf");
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    gtk::FileDialog::builder()
        .title(title)
        .filters(&filters)
        .default_filter(&filter)
        .build()
}

/// The name of `file` to show ("Report.pdf").
async fn display_name(file: &gio::File) -> String {
    file.query_info_future(
        gio::FILE_ATTRIBUTE_STANDARD_DISPLAY_NAME,
        gio::FileQueryInfoFlags::NONE,
        glib::Priority::DEFAULT,
    )
    .await
    .map(|i| i.display_name().to_string())
    .unwrap_or_else(|_| file.parse_name().to_string())
}

/// Reads `file`, mapping it if it's large and local (see [`PdfData`]).
///
/// Files without a local path (remote locations) are read through GIO.
async fn read_pdf(file: &gio::File) -> Result<PdfData, String> {
    if let Some(path) = file.path() {
        return gio::spawn_blocking(move || PdfData::open(path))
            .await
            .map_err(|_| gettext("reading failed"))?
            .map_err(|e| {
                // "No such file or directory (os error 2)", without the code.
                let text = e.to_string();
                text.split(" (os error").next().unwrap_or(&text).to_owned()
            });
    }
    let (bytes, _) = file
        .load_contents_future()
        .await
        .map_err(|e| e.message().to_owned())?;
    Ok(bytes.to_vec().into())
}

/// The Undo menu item for an edit, such as "Undo Rotate Page" or "Undo Delete 3 Pages".
///
/// Whole sentences per edit, so each can be translated as one.
fn undo_label(edit: Edit) -> String {
    match edit {
        Edit::Rotate { pages } => {
            ngettext_f("Undo Rotate Page", "Undo Rotate {n} Pages", pages, &[])
        }
        Edit::Delete { pages } => {
            ngettext_f("Undo Delete Page", "Undo Delete {n} Pages", pages, &[])
        }
        Edit::Move { pages } => ngettext_f("Undo Move Page", "Undo Move {n} Pages", pages, &[]),
        Edit::Insert { pages } => {
            ngettext_f("Undo Insert Page", "Undo Insert {n} Pages", pages, &[])
        }
        Edit::AddAnnotation(AnnotType::Highlight) => gettext("Undo Highlight"),
        Edit::AddAnnotation(AnnotType::Underline) => gettext("Undo Underline"),
        Edit::AddAnnotation(AnnotType::StrikeOut) => gettext("Undo Strikethrough"),
        Edit::AddAnnotation(AnnotType::Ink) => gettext("Undo Drawing"),
        Edit::AddAnnotation(AnnotType::Square) => gettext("Undo Rectangle"),
        Edit::AddAnnotation(AnnotType::Circle) => gettext("Undo Ellipse"),
        Edit::AddAnnotation(AnnotType::Line) => gettext("Undo Line"),
        Edit::AddAnnotation(AnnotType::Note) => gettext("Undo Note"),
        Edit::AddAnnotation(AnnotType::FreeText) => gettext("Undo Text Box"),
        Edit::AddAnnotation(_) => gettext("Undo Add Annotation"),
        Edit::RemoveAnnotation(_) => gettext("Undo Delete Annotation"),
        Edit::ChangeAnnotation(_) => gettext("Undo Change Annotation"),
    }
}

/// The Redo menu item for an edit, like [`undo_label`].
fn redo_label(edit: Edit) -> String {
    match edit {
        Edit::Rotate { pages } => {
            ngettext_f("Redo Rotate Page", "Redo Rotate {n} Pages", pages, &[])
        }
        Edit::Delete { pages } => {
            ngettext_f("Redo Delete Page", "Redo Delete {n} Pages", pages, &[])
        }
        Edit::Move { pages } => ngettext_f("Redo Move Page", "Redo Move {n} Pages", pages, &[]),
        Edit::Insert { pages } => {
            ngettext_f("Redo Insert Page", "Redo Insert {n} Pages", pages, &[])
        }
        Edit::AddAnnotation(AnnotType::Highlight) => gettext("Redo Highlight"),
        Edit::AddAnnotation(AnnotType::Underline) => gettext("Redo Underline"),
        Edit::AddAnnotation(AnnotType::StrikeOut) => gettext("Redo Strikethrough"),
        Edit::AddAnnotation(AnnotType::Ink) => gettext("Redo Drawing"),
        Edit::AddAnnotation(AnnotType::Square) => gettext("Redo Rectangle"),
        Edit::AddAnnotation(AnnotType::Circle) => gettext("Redo Ellipse"),
        Edit::AddAnnotation(AnnotType::Line) => gettext("Redo Line"),
        Edit::AddAnnotation(AnnotType::Note) => gettext("Redo Note"),
        Edit::AddAnnotation(AnnotType::FreeText) => gettext("Redo Text Box"),
        Edit::AddAnnotation(_) => gettext("Redo Add Annotation"),
        Edit::RemoveAnnotation(_) => gettext("Redo Delete Annotation"),
        Edit::ChangeAnnotation(_) => gettext("Redo Change Annotation"),
    }
}

/// An edit's name in the tests' `debug-state` (such as "Rotate Page" or "Delete 3 Pages"): always
/// English, and not shown to anyone.
fn edit_name(edit: Edit) -> String {
    let (verb, n) = match edit {
        Edit::Rotate { pages } => ("Rotate", pages),
        Edit::Delete { pages } => ("Delete", pages),
        Edit::Move { pages } => ("Move", pages),
        Edit::Insert { pages } => ("Insert", pages),
        Edit::AddAnnotation(t) => return format!("Add {t:?}"),
        Edit::RemoveAnnotation(t) => return format!("Remove {t:?}"),
        Edit::ChangeAnnotation(t) => return format!("Change {t:?}"),
    };
    if n == 1 {
        format!("{verb} Page")
    } else {
        format!("{verb} {n} Pages")
    }
}

/// What tells whether a file changed: its entity tag (for local files, the modification time to the
/// microsecond) and size.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Fingerprint {
    etag: String,
    size: i64,
}

/// `file`'s fingerprint, or `None` if it isn't there.
async fn fingerprint(file: &gio::File) -> Option<Fingerprint> {
    let info = file
        .query_info_future(
            "etag::value,standard::size",
            gio::FileQueryInfoFlags::NONE,
            glib::Priority::DEFAULT,
        )
        .await
        .ok()?;
    Some(Fingerprint {
        etag: info.etag().map(|e| e.to_string()).unwrap_or_default(),
        size: info.size(),
    })
}

/// Writes `job` to `file` in the background: through a temporary file renamed into place if it's
/// local, else through GIO.
async fn write_job(job: SaveJob, file: &gio::File) -> Result<(), String> {
    match file.path() {
        Some(path) => gio::spawn_blocking(move || job.save(&path).map_err(save_error))
            .await
            .unwrap_or_else(|_| Err(gettext("saving failed"))),
        None => {
            let bytes = gio::spawn_blocking(move || job.write().map_err(save_error))
                .await
                .unwrap_or_else(|_| Err(gettext("saving failed")))?;
            file.replace_contents_future(bytes, None, false, gio::FileCreateFlags::NONE)
                .await
                .map(|_| ())
                .map_err(|(_, e)| e.message().to_owned())
        }
    }
}

/// The folder `file` is in, as people know it.
///
/// In the Flatpak sandbox, files come through the document portal, at a path under `/run/user`. The
/// portal says where they really are.
fn folder(file: &gio::File) -> Option<gio::File> {
    const HOST_PATH: &str = "xattr::document-portal.host-path";
    let host = file
        .path()
        .filter(|p| p.starts_with("/run/user") || p.starts_with("/run/flatpak"))
        .and_then(|_| {
            file.query_info(
                HOST_PATH,
                gio::FileQueryInfoFlags::NONE,
                gio::Cancellable::NONE,
            )
            .ok()
        })
        .and_then(|info| info.attribute_string(HOST_PATH))
        .map(|p| gio::File::for_path(unescape_xattr(&p)));
    host.unwrap_or_else(|| file.clone()).parent()
}

/// Undoes GIO's escaping of extended attributes, which writes bytes that aren't printable ASCII,
/// and backslashes, as `\xNN`.
fn unescape_xattr(text: &str) -> std::path::PathBuf {
    use std::os::unix::ffi::OsStringExt;
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i..i + 4)
            .filter(|b| b.starts_with(b"\\x"))
            .and_then(|b| std::str::from_utf8(&b[2..]).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match hex {
            Some(b) => {
                out.push(b);
                i += 4;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    std::ffi::OsString::from_vec(out).into()
}

/// The last part of `file`'s name, for messages.
fn file_name(file: &gio::File) -> String {
    file.basename()
        .map_or_else(String::new, |b| b.to_string_lossy().into_owned())
}

/// The name Export Selected Pages suggests: "Report (pages 2-4, 7).pdf" for positions 1, 2, 3 and 6
/// of "Report.pdf".
fn export_name(name: &str, pages: &[usize]) -> String {
    let stem = name
        .strip_suffix(".pdf")
        .or_else(|| name.strip_suffix(".PDF"))
        .unwrap_or(name);
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for &p in pages {
        match ranges.last_mut() {
            Some((_, end)) if *end + 1 == p => *end = p,
            _ => ranges.push((p, p)),
        }
    }
    let what = match ranges.as_slice() {
        [(a, b)] if a == b => gettext_f("page {page}", &[("page", &(a + 1))]),
        r if r.len() <= 3 => {
            let parts: Vec<String> = r
                .iter()
                .map(|&(a, b)| {
                    if a == b {
                        (a + 1).to_string()
                    } else {
                        format!("{}-{}", a + 1, b + 1)
                    }
                })
                .collect();
            gettext_f("pages {pages}", &[("pages", &parts.join(", "))])
        }
        _ => ngettext_f("{n} page", "{n} pages", pages.len(), &[]),
    };
    format!("{stem} ({what}).pdf")
}

/// Why saving failed, for a message.
fn save_error(e: specto_write::WriteError) -> String {
    use specto_write::WriteError as E;
    match e {
        E::Io(e) => {
            // "Permission denied (os error 13)", without the code.
            let text = e.to_string();
            text.split(" (os error").next().unwrap_or(&text).to_owned()
        }
        E::PasswordRequired => gettext("the document's password is needed"),
        _ => gettext_f(
            "{app} can't save this document correctly yet",
            &[("app", &crate::APP_NAME)],
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{export_name, position_after_move, unescape_xattr};

    #[test]
    fn xattr_paths() {
        assert_eq!(
            unescape_xattr(r"/home/a/R\xc3\xa9sum\xc3\xa9s/x\x5cy.pdf"),
            std::path::Path::new("/home/a/Résumés/x\\y.pdf")
        );
        assert_eq!(unescape_xattr(r"/a/\xzz"), std::path::Path::new(r"/a/\xzz"));
    }

    #[test]
    fn export_names() {
        assert_eq!(export_name("Report.pdf", &[4]), "Report (page 5).pdf");
        assert_eq!(
            export_name("Report.pdf", &[1, 2, 3, 6]),
            "Report (pages 2-4, 7).pdf"
        );
        assert_eq!(export_name("a.PDF", &[0, 2, 4, 6]), "a (4 pages).pdf");
        assert_eq!(export_name("notes", &[0, 1]), "notes (pages 1-2).pdf");
    }

    /// Checks `position_after_move` against moving the pages of a list.
    #[test]
    fn pages_keep_their_place_after_a_move() {
        for (moved, to) in [(vec![0], 3), (vec![4, 1], 0), (vec![2, 3], 6), (vec![5], 2)] {
            let mut doc = specto_doc::EditDoc::from_source(specto_doc::SourceId(0), 6);
            let range = doc.move_pages(&moved, to).expect("moved");
            for page in 0..6u32 {
                let now = doc.pages().iter().position(|p| p.page == page).unwrap();
                assert_eq!(
                    position_after_move(&moved, range.start, page as usize),
                    now,
                    "page {page}, moving {moved:?} to {to}"
                );
            }
        }
    }
}
