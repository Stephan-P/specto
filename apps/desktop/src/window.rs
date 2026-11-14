//! A document window: header bar, thumbnail sidebar and page view.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use pdfrs_geom::ViewPoint;
use pdfrs_render::OpenError;
use pdfrs_view::{LayoutMode, Viewer, ZoomMode};

use crate::canvas::Canvas;
use crate::debug;
use crate::document::{Document, SharedDocument};
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
}

impl Window {
    pub fn new(app: &adw::Application) -> Self {
        let canvas = Canvas::new();
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

        let view = adw::ToolbarView::new();
        view.add_top_bar(&header);
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
                sidebar_toggle.upcast(),
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
        }));
        this.setup_actions();
        this.setup_page_entry();
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

        // Remember where the user was in this file.
        let weak = Rc::downgrade(&this.0);
        this.0.window.connect_close_request(move |_| {
            if let Some(inner) = weak.upgrade() {
                Window(inner).save_state();
            }
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
        let name = file
            .query_info_future(
                gio::FILE_ATTRIBUTE_STANDARD_DISPLAY_NAME,
                gio::FileQueryInfoFlags::NONE,
                glib::Priority::DEFAULT,
            )
            .await
            .map(|i| i.display_name().to_string())
            .unwrap_or_else(|_| file.parse_name().to_string());
        let data = match file.load_contents_future().await {
            Ok((bytes, _)) => bytes.to_vec(),
            Err(e) => return self.toast(&format!("Couldn't read “{name}”: {}", e.message())),
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
        let (name, pages) = {
            let d = document.borrow();
            (d.name.clone(), d.viewer.page_count())
        };
        inner.window.set_title(Some(&name));
        inner.title.set_title(&name);
        inner.title.set_subtitle(&format!(
            "{pages} page{}",
            if pages == 1 { "" } else { "s" }
        ));
        // With labels the entry shows "iv", so also give the plain position.
        if document.borrow().labels.is_none() {
            inner.page_total.set_label(&format!("of {pages}"));
        }

        let weak = Rc::downgrade(&self.0);
        let sidebar = Sidebar::new(document.clone(), move |page| {
            if let Some(inner) = weak.upgrade() {
                inner.canvas.update_viewer(|v| v.go_to_page(page));
            }
        });
        let outline = document.borrow().source.outline();
        if outline.is_empty() {
            inner.split.set_sidebar(Some(&sidebar.widget));
        } else {
            let weak = Rc::downgrade(&self.0);
            let labels = Rc::downgrade(&document);
            let page_label = move |page: usize| {
                labels
                    .upgrade()
                    .map(|d| d.borrow().page_label(page))
                    .unwrap_or_default()
            };
            let outline = OutlineView::new(&outline, page_label, move |target| {
                if let Some(inner) = weak.upgrade() {
                    inner.canvas.follow(target);
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
        .bool("complete", inner.canvas.is_complete());
        let width = inner.window.width();
        json.num("width", width as f64)
            .num("height", inner.window.height() as f64);
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
        let filter = gtk::FileFilter::new();
        filter.set_name(Some("PDF documents"));
        filter.add_mime_type("application/pdf");
        filter.add_suffix("pdf");
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let dialog = gtk::FileDialog::builder()
            .title("Open PDF")
            .filters(&filters)
            .default_filter(&filter)
            .build();
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
