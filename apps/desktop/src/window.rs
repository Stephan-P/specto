//! A document window: header bar, thumbnail sidebar and page view.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use pdfrs_doc::{Edit, Rotation, SourceId};
use pdfrs_geom::ViewPoint;
use pdfrs_render::{OpenError, PdfData};
use pdfrs_view::{LayoutMode, Viewer, ZoomMode};

use crate::canvas::Canvas;
use crate::debug;
use crate::document::{Document, Indexed, MAIN, SaveJob, SharedDocument};
use crate::outline::OutlineView;
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
    "save",
    "save-as",
    "insert-file",
    "delete-pages",
    "export-pages",
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
    /// Thumbnails / Contents, if the document has an outline.
    sidebar_stack: RefCell<Option<adw::ViewStack>>,
    last_page: Cell<usize>,
    /// The last message shown as a toast, for tests.
    last_toast: RefCell<String>,
    search_bar: gtk::SearchBar,
    search_entry: gtk::SearchEntry,
    /// "3 of 17", "No matches", …
    search_status: gtk::Label,
    /// Undo and Redo in the main menu, named after the edit.
    undo_section: gio::Menu,
    /// A save is running in the background.
    saving: Cell<bool>,
    /// The user chose to close despite unsaved changes (or saved them).
    close_confirmed: Cell<bool>,
    /// The "Save changes?" dialog while it's open (for tests).
    alert: RefCell<Option<adw::AlertDialog>>,
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

        let split = adw::OverlaySplitView::builder()
            .content(&scrolled)
            .show_sidebar(true)
            .min_sidebar_width(170.0)
            .max_sidebar_width(260.0)
            .build();

        let empty = adw::StatusPage::builder()
            .icon_name("x-office-document-symbolic")
            .title("Open a PDF")
            .description("Drop a file here or choose one")
            .build();
        let open_button = gtk::Button::builder()
            .label("Open…")
            .action_name("win.open")
            .halign(gtk::Align::Center)
            .css_classes(["pill", "suggested-action"])
            .build();
        let recent_list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["boxed-list"])
            .build();
        let recent_title = gtk::Label::builder()
            .label("Recent")
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
        let title = adw::WindowTitle::new("pdfrs", "");
        let header = adw::HeaderBar::builder().title_widget(&title).build();
        let sidebar_toggle = gtk::ToggleButton::builder()
            .icon_name("sidebar-show-symbolic")
            .tooltip_text("Thumbnails (F9)")
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
                .tooltip_text(format!("Open ({}O)", primary_key()))
                .action_name("win.open")
                .build(),
        );

        // The main menu: editing, for now.
        let undo_section = gio::Menu::new();
        let main_menu = gio::Menu::new();
        main_menu.append_section(None, &{
            let m = gio::Menu::new();
            m.append(Some("Save"), Some("win.save"));
            m.append(Some("Save As…"), Some("win.save-as"));
            m.append(Some("Export Selected Pages…"), Some("win.export-pages"));
            m
        });
        main_menu.append_section(None, &undo_section);
        main_menu.append_section(None, &{
            let m = gio::Menu::new();
            m.append(Some("Rotate Left"), Some("win.rotate-left"));
            m.append(Some("Rotate Right"), Some("win.rotate-right"));
            let delete = gio::MenuItem::new(Some("Delete Pages"), Some("win.delete-pages"));
            // Shown only: the keys work through `delete_keys`.
            delete.set_attribute_value("accel", Some(&"Delete".to_variant()));
            m.append_item(&delete);
            m
        });
        main_menu.append_section(None, &{
            let m = gio::Menu::new();
            m.append(Some("Insert Pages from File…"), Some("win.insert-file"));
            m
        });
        let main_button = gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .tooltip_text("Main Menu")
            .menu_model(&main_menu)
            .primary(true)
            .build();
        header.pack_end(&main_button);

        let view_menu = gio::Menu::new();
        let layout_section = gio::Menu::new();
        for (label, target) in [
            ("Single Page", "single"),
            ("Continuous", "continuous"),
            ("Two Pages", "two-page"),
            ("Two Pages, Continuous", "two-page-continuous"),
        ] {
            layout_section.append(Some(label), Some(&format!("win.layout::{target}")));
        }
        view_menu.append_section(None, &layout_section);
        view_menu.append_section(None, &{
            let m = gio::Menu::new();
            m.append(Some("Show Cover Page Alone"), Some("win.cover-alone"));
            m
        });
        let layout_button = gtk::MenuButton::builder()
            .icon_name("view-dual-symbolic")
            .tooltip_text("Page Layout")
            .menu_model(&view_menu)
            .build();
        header.pack_end(&layout_button);

        let zoom_menu = gio::Menu::new();
        zoom_menu.append_section(None, &{
            let m = gio::Menu::new();
            m.append(Some("Fit Width"), Some("win.fit-width"));
            m.append(Some("Fit Page"), Some("win.fit-page"));
            m.append(Some("Actual Size"), Some("win.zoom-actual"));
            m
        });
        zoom_menu.append_section(None, &{
            let m = gio::Menu::new();
            m.append(Some("Zoom In"), Some("win.zoom-in"));
            m.append(Some("Zoom Out"), Some("win.zoom-out"));
            m
        });
        let zoom_button = gtk::MenuButton::builder()
            .label("100%")
            .tooltip_text("Zoom")
            .menu_model(&zoom_menu)
            .build();
        header.pack_end(&zoom_button);

        let page_entry = gtk::Entry::builder()
            .width_chars(4)
            .max_width_chars(6)
            .xalign(1.0)
            .input_purpose(gtk::InputPurpose::Digits)
            .tooltip_text(format!("Go to Page ({}L)", primary_key()))
            .build();
        let page_total = gtk::Label::new(None);
        page_total.add_css_class("dim-label");
        let page_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        page_box.append(&page_entry);
        page_box.append(&page_total);
        header.pack_end(&page_box);

        // Search: a bar under the header bar.
        let search_entry = gtk::SearchEntry::builder()
            .placeholder_text("Search")
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
                .tooltip_text("Previous Match (Shift+Enter)")
                .action_name("win.find-previous")
                .build(),
        );
        step_buttons.append(
            &gtk::Button::builder()
                .icon_name("go-down-symbolic")
                .tooltip_text("Next Match (Enter)")
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
        // Not `connect_entry`: then the bar focuses the entry again once
        // it has slid in, which selects what was typed meanwhile, and the
        // next key replaces it. The find action focuses it right away.
        let search_toggle = gtk::ToggleButton::builder()
            .icon_name("system-search-symbolic")
            .tooltip_text(format!("Search ({}F)", primary_key()))
            .build();
        search_bar
            .bind_property("search-mode-enabled", &search_toggle, "active")
            .bidirectional()
            .sync_create()
            .build();
        header.pack_end(&search_toggle);

        let view = adw::ToolbarView::new();
        view.add_top_bar(&header);
        view.add_top_bar(&search_bar);
        view.set_content(Some(&toasts));

        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("pdfrs")
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
                main_button.upcast(),
                sidebar_toggle.upcast(),
                search_toggle.upcast(),
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
            last_page: Cell::new(usize::MAX),
            last_toast: RefCell::default(),
            search_bar,
            search_entry,
            search_status,
            undo_section,
            saving: Cell::new(false),
            close_confirmed: Cell::new(false),
            alert: RefCell::default(),
        }));
        this.setup_actions();
        this.setup_page_entry();
        this.setup_search();
        this.setup_drop();
        if debug::enabled() {
            this.setup_debug_actions();
        }
        let weak = Rc::downgrade(&this.0);
        this.0.canvas.connect_view_changed(move || {
            if let Some(inner) = weak.upgrade() {
                Window(inner).view_changed();
            }
        });
        let weak = Rc::downgrade(&this.0);
        this.0.canvas.connect_open_uri(move |uri| {
            if let Some(inner) = weak.upgrade() {
                Window(inner).open_uri(uri);
            }
        });
        this.set_document_actions_enabled(false);

        // Ask about unsaved changes; remember where the user was in this file.
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

        // Callbacks only hold weak references, so the GTK window owns the
        // state and keeps it alive until it is destroyed.
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
        let data = match read_pdf(&file).await {
            Ok(data) => data,
            Err(e) => return self.toast(&format!("Couldn't read “{name}”: {e}")),
        };

        let (tx, rx) = async_channel::unbounded::<()>();
        let mut password: Option<String> = None;
        let document = loop {
            let tx = tx.clone();
            let wake = move || {
                let _ = tx.try_send(());
            };
            match Document::open(name.clone(), data.clone(), password.as_deref(), wake) {
                Ok(d) => break d,
                Err(OpenError::PasswordRequired) => {
                    match self.ask_password(&name, password.is_some()).await {
                        Some(p) => password = Some(p),
                        None => return,
                    }
                }
                Err(e) => return self.toast(&format!("Couldn't open “{name}”: {e}")),
            }
        };
        drop(tx);
        self.set_document(Rc::new(RefCell::new(document)), rx);
        let uri = file.uri().to_string();
        self.restore_state(&uri);
        gtk::RecentManager::default().add_item(&uri);
        self.0.uri.replace(Some(uri));
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
            let subtitle = file
                .parent()
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
            format!("The password was incorrect. Enter the password for “{name}”.")
        } else {
            format!("“{name}” is protected with a password.")
        };
        let dialog = adw::AlertDialog::new(Some("Password Required"), Some(&body));
        dialog.add_responses(&[("cancel", "_Cancel"), ("unlock", "_Unlock")]);
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
        );
        sidebar.widget.add_controller(delete_keys());
        let outline = document.borrow().outline();
        if outline.is_empty() {
            inner.split.set_sidebar(Some(&sidebar.widget));
        } else {
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
            let stack = adw::ViewStack::new();
            stack.add_titled_with_icon(
                &sidebar.widget,
                Some("thumbnails"),
                "Thumbnails",
                "view-grid-symbolic",
            );
            stack.add_titled_with_icon(
                &outline.widget,
                Some("outline"),
                "Contents",
                "view-list-symbolic",
            );
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
        }
        inner.sidebar.replace(Some(sidebar));
        inner.document.replace(Some(document.clone()));
        inner.canvas.set_document(Some(document.clone()));
        inner.stack.set_visible_child_name("document");
        self.set_document_actions_enabled(true);
        inner.canvas.grab_focus();
        self.view_changed();

        // Finished tiles → cache → redraw.
        let weak = Rc::downgrade(&self.0);
        glib::spawn_future_local(async move {
            while tiles.recv().await.is_ok() {
                let Some(inner) = weak.upgrade() else { break };
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

    /// Opens a link from the document. Only web and mail links: a PDF
    /// shouldn't be able to launch local files or other URI handlers.
    fn open_uri(&self, uri: &str) {
        let scheme = uri.split(':').next().unwrap_or("").to_ascii_lowercase();
        if !matches!(scheme.as_str(), "http" | "https" | "mailto") {
            return self.toast(&format!("Links of this kind aren't opened: {uri}"));
        }
        let launcher = gtk::UriLauncher::new(uri);
        let this = self.clone();
        glib::spawn_future_local(async move {
            if let Err(e) = launcher.launch_future(Some(&this.0.window)).await {
                this.toast(&format!("Couldn't open link: {}", e.message()));
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
        // Closing the bar (Escape, the button) ends the search; opening it
        // again searches for what's still in the entry.
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

    /// Searches for the entry's text, goes to the first match from the
    /// current page on, and indexes the rest of the document if needed.
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

    /// Indexes pages the current search hasn't seen yet (inserted ones),
    /// if there is a search.
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
    fn receive_index(&self, rx: async_channel::Receiver<Indexed>) {
        let weak = Rc::downgrade(&self.0);
        glib::spawn_future_local(async move {
            while let Ok(page) = rx.recv().await {
                let Some(inner) = weak.upgrade() else { break };
                let w = Window(inner);
                let Some(doc) = w.0.document.borrow().clone() else {
                    break;
                };
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
                    (0, _) if done => ("No matches".to_owned(), true),
                    (0, _) => ("Searching…".to_owned(), false),
                    (n, Some(c)) => (
                        format!("{} of {n}{}", c + 1, if done { "" } else { "…" }),
                        false,
                    ),
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
            // Drops thumbnails without dragging: "0,1@450" drops pages 0
            // and 1 at window height 450.
            action("debug-drop-thumbnails", |w, arg| {
                let Some((pages, y)) = arg.split_once('@') else {
                    return;
                };
                let pages = pages.split(',').filter_map(|p| p.parse().ok()).collect();
                if let (Some(s), Ok(y)) = (w.0.sidebar.borrow().as_ref(), y.parse()) {
                    s.drop_at(pages, &w.0.window, y);
                }
            }),
            // Drops files on the sidebar without dragging: "a.pdf|b.pdf@450"
            // drops two files at window height 450.
            action("debug-drop-files", |w, arg| {
                let Some((paths, y)) = arg.rsplit_once('@') else {
                    return;
                };
                let files = paths.split('|').map(gio::File::for_path).collect();
                if let (Some(s), Ok(y)) = (w.0.sidebar.borrow().as_ref(), y.parse()) {
                    s.drop_files_at(files, &w.0.window, y);
                }
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
        .bool("search_bar", inner.search_bar.is_search_mode())
        // Mapped once the bar has started sliding in; keys before that are
        // dropped (GTK doesn't deliver them to unmapped widgets).
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
        // Where the canvas is in the window, and for the pages around the
        // current one, the affine map from PDF points to window coordinates:
        // [page, a, b, c, d, e, f] with x' = a x + c y + e, y' = b x + d y + f.
        let origin = inner
            .canvas
            .compute_point(&inner.window, &gtk::graphene::Point::new(0.0, 0.0))
            .map_or((0.0, 0.0), |p| (p.x() as f64, p.y() as f64));
        let (px, py) = inner.canvas.pointer();
        json.raw("canvas", &format!("[{}, {}]", origin.0, origin.1))
            .raw("pointer", &format!("[{px}, {py}]"));
        // Thumbnails on screen, [page, x, y, width, height] in the window,
        // and the pages selected in the sidebar.
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
                    let at = |x, y| v.pdf_to_view(i, pdfrs_geom::PdfPoint::new(x, y));
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
                .edit
                .pages()
                .iter()
                .map(|p| format!("[{}, {}, {}]", p.page, p.rotation.degrees(), p.source.0))
                .collect();
            // [page of the file, rotation in degrees, file] per position.
            json.raw("edit_pages", &format!("[{}]", pages.join(", ")))
                .bool("modified", d.edit.is_modified())
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
                .str(
                    "undo",
                    &d.edit.undo_edit().map(edit_name).unwrap_or_default(),
                )
                .str(
                    "redo",
                    &d.edit.redo_edit().map(edit_name).unwrap_or_default(),
                );
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

    /// The pages an edit applies to: those selected in the sidebar if there
    /// are several, else the current page (both pages of a spread).
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

    /// Deletes the pages selected in the sidebar if there are several,
    /// else the current page. The last page can't be deleted.
    fn delete_pages(&self) {
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

    /// Moves the pages at `pages` before position `to`; they stay
    /// selected, and the view stays on the page it showed.
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

    /// Turns the pages to edit (see [`pages_to_edit`]) by `by`, unless a
    /// text field has the focus: there the shortcut (Ctrl+Left/Right) moves
    /// by a word, as it would without this action.
    ///
    /// [`pages_to_edit`]: Self::pages_to_edit
    fn rotate(&self, by: Rotation) {
        if let Some(text) = gtk::prelude::GtkWindowExt::focus(&self.0.window)
            .and_then(|f| f.downcast::<gtk::Text>().ok())
        {
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

    /// Asks for a PDF and inserts its pages.
    async fn choose_insert_file(&self) {
        let dialog = pdf_dialog("Insert Pages from File");
        if let Some(folder) = self
            .0
            .uri
            .borrow()
            .as_ref()
            .and_then(|u| gio::File::for_uri(u).parent())
        {
            dialog.set_initial_folder(Some(&folder));
        }
        if let Ok(file) = dialog.open_future(Some(&self.0.window)).await {
            self.insert_file(file).await;
        }
    }

    /// Inserts all pages of `file` after the current page (or spread) and
    /// goes to the first of them.
    async fn insert_file(&self, file: gio::File) {
        self.insert_files(vec![file], None).await;
    }

    /// Inserts all pages of `files`, in order and as one edit, before
    /// position `at` (by default after the current page or spread), and
    /// goes to the first of them. A file that can't be opened is left out,
    /// with a message.
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

    /// Reads and opens `file` as a source of `doc`, asking for its password
    /// if it has one. `None` if it couldn't be opened (with a message) or
    /// the password was cancelled.
    async fn open_source(&self, doc: &SharedDocument, file: &gio::File) -> Option<SourceId> {
        let name = display_name(file).await;
        let data = match read_pdf(file).await {
            Ok(data) => data,
            Err(e) => {
                self.toast(&format!("Couldn't read “{name}”: {e}"));
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
                    self.toast(&format!("Couldn't open “{name}”: {e}"));
                    return None;
                }
            }
        }
    }

    /// The pages "Export Selected Pages" writes: those selected in the
    /// sidebar, else the current page.
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

    /// Asks where to save the selected pages, and saves them there as a
    /// new PDF.
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
            .title("Export Selected Pages")
            .initial_name(export_name(&name, &pages))
            .build();
        if let Some(folder) = inner
            .uri
            .borrow()
            .as_ref()
            .and_then(|u| gio::File::for_uri(u).parent())
        {
            dialog.set_initial_folder(Some(&folder));
        }
        if let Ok(file) = dialog.save_future(Some(&inner.window)).await {
            self.export_to(&pages, file).await;
        }
    }

    /// Saves the pages at `pages` as a new PDF in `file`. The document and
    /// its file stay as they are.
    async fn export_to(&self, pages: &[usize], file: gio::File) {
        let Some(doc) = self.0.document.borrow().clone() else {
            return;
        };
        let job = {
            let d = doc.borrow();
            let refs = pages
                .iter()
                .filter_map(|&i| d.edit.pages().get(i).copied())
                .collect();
            d.save_pages(refs)
        };
        let n = pages.len();
        match write_job(job, &file).await {
            Ok(()) => self.toast(&format!(
                "Exported {n} page{} to “{}”",
                if n == 1 { "" } else { "s" },
                file_name(&file)
            )),
            Err(e) => self.toast(&format!("Couldn't save “{}”: {e}", file_name(&file))),
        }
    }

    fn is_modified(&self) -> bool {
        self.0
            .document
            .borrow()
            .as_ref()
            .is_some_and(|d| d.borrow().edit.is_modified())
    }

    /// Saves the document to its file, if it has changes. Returns whether
    /// it's saved now.
    async fn save(&self) -> bool {
        if !self.is_modified() {
            return true;
        }
        let Some(uri) = self.0.uri.borrow().clone() else {
            return false;
        };
        self.save_to(gio::File::for_uri(&uri)).await
    }

    /// Asks where to save a copy, saves it there, and from then on shows
    /// that file.
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
            .title("Save As")
            .initial_name(&name)
            .build();
        if let Some(folder) = inner
            .uri
            .borrow()
            .as_ref()
            .and_then(|u| gio::File::for_uri(u).parent())
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
        true
    }

    /// Writes the document to `file`. Local files are written next to the
    /// old one and renamed into place (see `pdfrs_write::replace_file`);
    /// others through GIO. Returns whether it worked; if not, says why.
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
        inner.saving.set(false);
        match result {
            Ok(()) => {
                doc.borrow_mut().edit.mark_saved_version(version);
                self.update_edit_state();
                true
            }
            Err(e) => {
                self.toast(&format!("Couldn't save “{}”: {e}", file_name(&file)));
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
            Some("Save Changes?"),
            Some(&format!(
                "“{name}” has unsaved changes. Changes that aren't saved will be lost."
            )),
        );
        alert.add_responses(&[
            ("cancel", "_Cancel"),
            ("discard", "_Discard"),
            ("save", "_Save"),
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

    /// Applies an edit to the document, then updates the view, the sidebar
    /// and the menus if it changed anything. `f` returns `None` if nothing
    /// changed.
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
        // Scrollbars, redraw, page number; in one update with going to a
        // page, so the sidebar doesn't follow a page in between.
        inner.canvas.update_viewer(|v| {
            if let Some(page) = edited.go_to {
                v.go_to_page(page);
            }
        });
        self.update_edit_state();
        self.update_search_status();
    }

    /// Updates what depends on the edits: Undo and Redo, the title, the
    /// page count.
    fn update_edit_state(&self) {
        let inner = &self.0;
        let Some(doc) = inner.document.borrow().clone() else {
            return;
        };
        let d = doc.borrow();
        let (undo, redo) = (d.edit.undo_edit(), d.edit.redo_edit());
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
            Some(&undo.map_or_else(|| "Undo".to_owned(), |e| format!("Undo {}", edit_name(e)))),
            Some("win.undo"),
        );
        section.append(
            Some(&redo.map_or_else(|| "Redo".to_owned(), |e| format!("Redo {}", edit_name(e)))),
            Some("win.redo"),
        );

        let pages = d.viewer.page_count();
        let mut subtitle = format!("{pages} page{}", if pages == 1 { "" } else { "s" });
        if d.edit.is_modified() {
            subtitle.push_str(" — Edited");
        }
        inner.title.set_subtitle(&subtitle);
        // With labels, `show_page_number` gives the position instead.
        if d.labels.is_none() {
            inner.page_total.set_label(&format!("of {pages}"));
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
            // Copy and Select All act on the page entry when it has the
            // focus (application shortcuts would otherwise take its Ctrl+C),
            // and on the document's text otherwise.
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
            gio::ActionEntry::builder("export-pages")
                .activate(with(|w| {
                    let w = w.clone();
                    glib::spawn_future_local(async move { w.export_pages().await });
                }))
                .build(),
            gio::ActionEntry::builder("delete-pages")
                .activate(with(|w| w.delete_pages()))
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

        // Which sidebar page to show: "thumbnails" or "outline".
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
    }

    /// Puts page `page`'s label (or number) into the page entry, plus
    /// "(3 of 120)" when the document has labels.
    fn show_page_number(&self, page: usize) {
        let inner = &self.0;
        let Some(doc) = inner.document.borrow().clone() else {
            return;
        };
        let d = doc.borrow();
        inner.page_entry.set_text(&d.page_label(page));
        if d.labels.is_some() {
            inner
                .page_total
                .set_label(&format!("({} of {})", page + 1, d.viewer.page_count()));
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
        let dialog = pdf_dialog("Open PDF");
        let this = self.clone();
        glib::spawn_future_local(async move {
            if let Ok(file) = dialog.open_future(Some(&this.0.window)).await {
                this.open(file);
            }
        });
    }
}

/// How the primary shortcut modifier is written in tooltips: Cmd on macOS,
/// Ctrl elsewhere (shortcuts use GTK's `<Primary>`, which maps the same way).
fn primary_key() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌘"
    } else {
        "Ctrl+"
    }
}

fn centre(v: &Viewer) -> ViewPoint {
    ViewPoint::new(v.viewport().width / 2.0, v.viewport().height / 2.0)
}

/// What an edit changed, for [`Window::edit`].
#[derive(Default)]
struct Edited {
    /// Pages to select in the sidebar (none: the current page).
    select: Vec<usize>,
    /// The page to show.
    go_to: Option<usize>,
}

/// Where the page at `page` is after moving those at `moved` so that they
/// start at `start`.
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
/// Not shortcuts for the whole window, which would take the keys from text
/// fields.
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
    filter.set_name(Some("PDF documents"));
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
/// Files without a local path (remote locations) are read through GIO.
async fn read_pdf(file: &gio::File) -> Result<PdfData, String> {
    if let Some(path) = file.path() {
        return gio::spawn_blocking(move || PdfData::open(path))
            .await
            .map_err(|_| "reading failed".to_owned())?
            .map_err(|e| {
                // "No such file or directory (os error 2)" → without the code.
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

/// An edit's name for the Undo and Redo menu items: "Rotate Page",
/// "Delete 3 Pages", …
fn edit_name(edit: Edit) -> String {
    let (verb, n) = match edit {
        Edit::Rotate { pages } => ("Rotate", pages),
        Edit::Delete { pages } => ("Delete", pages),
        Edit::Move { pages } => ("Move", pages),
        Edit::Insert { pages } => ("Insert", pages),
    };
    if n == 1 {
        format!("{verb} Page")
    } else {
        format!("{verb} {n} Pages")
    }
}

/// Writes `job` to `file` in the background: through a temporary file
/// renamed into place if it's local, else through GIO.
async fn write_job(job: SaveJob, file: &gio::File) -> Result<(), String> {
    match file.path() {
        Some(path) => gio::spawn_blocking(move || job.save(&path).map_err(save_error))
            .await
            .unwrap_or_else(|_| Err("saving failed".to_owned())),
        None => {
            let bytes = gio::spawn_blocking(move || job.write().map_err(save_error))
                .await
                .unwrap_or_else(|_| Err("saving failed".to_owned()))?;
            file.replace_contents_future(bytes, None, false, gio::FileCreateFlags::NONE)
                .await
                .map(|_| ())
                .map_err(|(_, e)| e.message().to_owned())
        }
    }
}

/// The last part of `file`'s name, for messages.
fn file_name(file: &gio::File) -> String {
    file.basename()
        .map_or_else(String::new, |b| b.to_string_lossy().into_owned())
}

/// The name Export Selected Pages suggests: "Report (pages 2-4, 7).pdf"
/// for positions 1, 2, 3 and 6 of "Report.pdf".
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
        [(a, b)] if a == b => format!("page {}", a + 1),
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
            format!("pages {}", parts.join(", "))
        }
        _ => format!("{} pages", pages.len()),
    };
    format!("{stem} ({what}).pdf")
}

/// Why saving failed, for a message.
fn save_error(e: pdfrs_write::WriteError) -> String {
    use pdfrs_write::WriteError as E;
    match e {
        E::Io(e) => {
            // "Permission denied (os error 13)" → without the code.
            let text = e.to_string();
            text.split(" (os error").next().unwrap_or(&text).to_owned()
        }
        E::PasswordRequired => "the document's password is needed".to_owned(),
        _ => "pdfrs can't save this document correctly yet".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::{export_name, position_after_move};

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
            let mut doc = pdfrs_doc::EditDoc::from_source(pdfrs_doc::SourceId(0), 6);
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
