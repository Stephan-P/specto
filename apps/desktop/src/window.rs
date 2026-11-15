//! A document window: header bar, thumbnail sidebar and page view.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use specto_annot::{AnnotType, Color, MarkupStyle};
use specto_doc::{Edit, Rotation, SourceId};
use specto_geom::ViewPoint;
use specto_render::{OpenError, PdfData};
use specto_view::{LayoutMode, Viewer, ZoomMode};

use crate::canvas::Canvas;
use crate::debug;
use crate::document::{Document, Indexed, MAIN, PageBundle, SaveJob, SharedDocument};
use crate::i18n::{gettext, gettext_f, ngettext_f};
use crate::outline::OutlineView;
use crate::print;
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
    /// The file as it was when read or last saved, to tell changes by other
    /// programs from our own.
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

        let split = adw::OverlaySplitView::builder()
            .content(&scrolled)
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

        // The main menu: saving, printing, editing and About. Also on the
        // start page, where only About is enabled.
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
            m.append(Some(&gettext("Highlight")), Some("win.highlight"));
            m.append(Some(&gettext("Underline")), Some("win.underline"));
            m.append(Some(&gettext("Strikethrough")), Some("win.strike-out"));
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
        // Not `connect_entry`: then the bar focuses the entry again once
        // it has slid in, which selects what was typed meanwhile, and the
        // next key replaces it. The find action focuses it right away.
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
        this.0.canvas.connect_open_uri(move |uri| {
            if let Some(inner) = weak.upgrade() {
                Window(inner).open_uri(uri);
            }
        });
        this.set_document_actions_enabled(false);

        // Look at the file again when coming back to the window: file
        // monitors miss changes on some file systems, and in the Flatpak
        // sandbox (the document portal) they miss changes made outside it.
        let weak = Rc::downgrade(&this.0);
        this.0.window.connect_is_active_notify(move |window| {
            if let Some(inner) = weak.upgrade()
                && window.is_active()
            {
                Window(inner).schedule_check();
            }
        });

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

    /// Opens `data` as a document, asking for a password if it needs one
    /// (`password` is tried first). Returns the document, its tile
    /// notifications and the password that worked; `None` if it couldn't
    /// be opened (with a message) or the password was cancelled.
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

    /// Starts watching `file`, as it was when read (`known`), for changes
    /// by other programs.
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

    /// Looks at the file shortly: a save by another program comes as
    /// several events.
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

    /// Compares the file with what was read or saved: unchanged, nothing
    /// happens; changed, it's reloaded, or with edits a banner offers to;
    /// gone, a banner says so.
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

    /// Whether the file changed on disk since it was read or saved (and is
    /// still there).
    async fn changed_on_disk(&self) -> bool {
        let Some(uri) = self.0.uri.borrow().clone() else {
            return false;
        };
        let now = fingerprint(&gio::File::for_uri(&uri)).await;
        now.is_some() && now != *self.0.known.borrow()
    }

    /// Reads the file again and shows it in place of the document, edits
    /// discarded, at the same place.
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

    /// Asks whether to save over a file another program changed. Returns
    /// whether to go ahead.
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
        let outline = document.borrow().outline();
        if outline.is_empty() {
            inner.split.set_sidebar(Some(&sidebar.widget));
            inner.sidebar_stack.replace(None);
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
                &gettext("Thumbnails"),
                "view-grid-symbolic",
            );
            stack.add_titled_with_icon(
                &outline.widget,
                Some("outline"),
                &gettext("Contents"),
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

        // Finished tiles → cache → redraw. Not holding on to the document:
        // once it's replaced (reloaded) and dropped, its channel closes.
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
    /// The pages are for the document shown now: if it's replaced (reloaded)
    /// meanwhile, the rest are dropped.
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
            // Drops pages of another window without dragging: "1:0,2@450"
            // drops pages 0 and 2 of window 1 at window height 450.
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
            // Prints to a PDF file without the dialog: "out.pdf", or
            // "out.pdf|selection" for the pages selected in the sidebar.
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
        // The annotations of the current page (the file's and new ones),
        // and whether one is picked.
        if let Some(doc) = inner.document.borrow().clone() {
            let mut d = doc.borrow_mut();
            let page = d.viewer.current_page();
            let count = d.annotations(page).len();
            json.num("annotations", count as f64)
                .bool("annotation_picked", d.picked.is_some());
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
    /// Marks the selected text: a highlight, or a line under or through
    /// it.
    fn mark(&self, style: MarkupStyle) {
        let color = match style {
            MarkupStyle::Highlight => HIGHLIGHT,
            _ => MARKUP_LINE,
        };
        if !self.annotate(|d| d.mark_selection(style, color)) {
            self.toast(&gettext("Select the text to mark first"));
        }
    }

    /// Applies an edit to the annotations, then redraws and updates the
    /// menus if it changed anything, which it returns. The pages stay as
    /// they are, so there is less to do than after [`edit`](Self::edit).
    fn annotate(&self, f: impl FnOnce(&mut Document) -> bool) -> bool {
        let inner = &self.0;
        let Some(doc) = inner.document.borrow().clone() else {
            return false;
        };
        let changed = f(&mut doc.borrow_mut());
        if changed {
            inner.canvas.queue_draw();
            self.update_edit_state();
        }
        changed
    }

    /// Delete and Backspace: the annotation that is picked, else pages.
    fn delete_pages(&self) {
        if self.annotate(|d| d.remove_picked_annotation()) {
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

    /// Inserts pages of another document before position `to`; they're
    /// selected, and the view goes to the first of them.
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

    /// Shows the print dialog for the document as edited (or prints to a
    /// PDF file without it: see `print::print`).
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

    /// Saves the document to its file, if it has changes. Returns whether
    /// it's saved now.
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

    /// Writes the document to `file`. Local files are written next to the
    /// old one and renamed into place (see `specto_write::replace_file`);
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
                .activate(with(|w| w.mark(MarkupStyle::Highlight)))
                .build(),
            gio::ActionEntry::builder("underline")
                .activate(with(|w| w.mark(MarkupStyle::Underline)))
                .build(),
            gio::ActionEntry::builder("strike-out")
                .activate(with(|w| w.mark(MarkupStyle::StrikeOut)))
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

/// How a shortcut with the primary modifier is written in tooltips: "⌘O" on
/// macOS, "Ctrl+O" elsewhere (shortcuts use GTK's `<Primary>`, which maps
/// the same way).
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
    /// The windows, with the test hooks only: for those that reach into
    /// another window.
    static DEBUG_WINDOWS: RefCell<Vec<std::rc::Weak<Inner>>> = const { RefCell::new(Vec::new()) };
}

/// The colours of new markup: a highlighter's yellow, and red for lines
/// under and through text, as Preview has them.
const HIGHLIGHT: Color = Color::new(1.0, 0.92, 0.23);
const MARKUP_LINE: Color = Color::new(0.88, 0.11, 0.14);

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
/// Files without a local path (remote locations) are read through GIO.
async fn read_pdf(file: &gio::File) -> Result<PdfData, String> {
    if let Some(path) = file.path() {
        return gio::spawn_blocking(move || PdfData::open(path))
            .await
            .map_err(|_| gettext("reading failed"))?
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

/// The Undo menu item for an edit: "Undo Rotate Page", "Undo Delete 3
/// Pages", … Whole sentences per edit, so each can be translated as one.
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
        Edit::AddAnnotation(_) => gettext("Redo Add Annotation"),
        Edit::RemoveAnnotation(_) => gettext("Redo Delete Annotation"),
        Edit::ChangeAnnotation(_) => gettext("Redo Change Annotation"),
    }
}

/// An edit's name in the tests' `debug-state` ("Rotate Page", "Delete 3
/// Pages", …): always English, and not shown to anyone.
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

/// What tells whether a file changed: its entity tag (for local files,
/// the modification time to the microsecond) and size.
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

/// Writes `job` to `file` in the background: through a temporary file
/// renamed into place if it's local, else through GIO.
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

/// The folder `file` is in, as people know it. In the Flatpak sandbox,
/// files come through the document portal, at `/run/user/…/doc/…`; the
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

/// Undoes GIO's escaping of extended attributes, which writes bytes that
/// aren't printable ASCII, and backslashes, as `\xNN`.
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
            // "Permission denied (os error 13)" → without the code.
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
