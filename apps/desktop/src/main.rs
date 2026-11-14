//! pdfrs desktop app (GTK4 + libadwaita).
//!
//! `pdfrs-gtk [FILE…]` opens each file in its own window.

mod canvas;
mod document;
mod outline;
mod state;
mod thumbnails;
mod window;

use adw::prelude::*;
use gtk::{gio, glib};

use window::Window;

const APP_ID: &str = "net.xfbs.pdfrs";

fn main() -> glib::ExitCode {
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build();

    app.connect_startup(|app| {
        for (action, accels) in [
            ("win.open", &["<Primary>o"][..]),
            (
                "win.zoom-in",
                &["<Primary>plus", "<Primary>equal", "<Primary>KP_Add"],
            ),
            ("win.zoom-out", &["<Primary>minus", "<Primary>KP_Subtract"]),
            ("win.zoom-actual", &["<Primary>0", "<Primary>KP_0"]),
            ("win.fit-width", &["<Primary>8"]),
            ("win.fit-page", &["<Primary>9"]),
            ("win.go-to-page", &["<Primary>l"]),
            ("win.layout::single", &["<Primary>1"]),
            ("win.layout::continuous", &["<Primary>2"]),
            ("win.layout::two-page", &["<Primary>3"]),
            ("win.layout::two-page-continuous", &["<Primary>4"]),
            ("win.show-sidebar", &["F9"]),
            ("window.close", &["<Primary>w"]),
            ("app.quit", &["<Primary>q"]),
        ] {
            let accels = platform_accels(accels);
            let accels: Vec<&str> = accels.iter().map(String::as_str).collect();
            app.set_accels_for_action(action, &accels);
        }
        // macOS shows this in the system menu bar. Elsewhere the in-window
        // header bar menus are enough (and libadwaita windows don't show
        // a menu bar anyway).
        if cfg!(target_os = "macos") {
            app.set_menubar(Some(&menubar()));
        }
        // Close windows one by one, so each saves its view state.
        let quit = gio::ActionEntry::builder("quit")
            .activate(|app: &adw::Application, _, _| {
                for window in app.windows() {
                    window.close();
                }
            })
            .build();
        app.add_action_entries([quit]);
        if std::env::var_os("PDFRS_DEBUG").is_some() {
            app.add_action_entries([debug_screenshot()]);
        }
    });

    app.connect_activate(|app| Window::new(app).present());
    app.connect_open(|app, files, _hint| {
        for file in files {
            let window = Window::new(app);
            window.present();
            window.open(file.clone());
        }
    });
    app.run()
}

/// `app.screenshot(path)`: saves the active window as PNG. Only registered
/// with `PDFRS_DEBUG=1`; trigger it with
/// `gapplication action net.xfbs.pdfrs screenshot "'/tmp/shot.png'"`.
fn debug_screenshot() -> gio::ActionEntry<adw::Application> {
    gio::ActionEntry::builder("screenshot")
        .parameter_type(Some(&String::static_variant_type()))
        .activate(|app: &adw::Application, _, param| {
            let (Some(window), Some(path)) =
                (app.active_window(), param.and_then(|p| p.get::<String>()))
            else {
                return;
            };
            // At device resolution, so HiDPI sharpness shows in the PNG.
            let scale = window.surface().map_or(1.0, |s| s.scale()) as f32;
            let paintable = gtk::WidgetPaintable::new(Some(&window));
            let snapshot = gtk::Snapshot::new();
            snapshot.scale(scale, scale);
            paintable.snapshot(&snapshot, window.width() as f64, window.height() as f64);
            if let (Some(node), Some(renderer)) = (snapshot.to_node(), window.renderer()) {
                let _ = renderer.render_texture(&node, None).save_to_png(&path);
            }
        })
        .build()
}

/// On macOS, registers each `<Primary>` shortcut with `<Meta>` (Cmd) first:
/// GTK's macOS backend maps `<Primary>` to Ctrl, and menus display the first
/// shortcut of an action, so Cmd must come first to show as "⌘O". The Ctrl
/// version stays as a second, undisplayed shortcut.
fn platform_accels(accels: &[&str]) -> Vec<String> {
    let primary: Vec<String> = accels.iter().map(|a| a.to_string()).collect();
    if !cfg!(target_os = "macos") {
        return primary;
    }
    let mut out: Vec<String> = accels
        .iter()
        .filter_map(|a| a.strip_prefix("<Primary>"))
        .map(|key| format!("<Meta>{key}"))
        .collect();
    out.extend(primary);
    out
}

/// The application menu bar (macOS only): File, View and Go, like Preview.
fn menubar() -> gio::Menu {
    let section = |items: &[(&str, &str)]| {
        let m = gio::Menu::new();
        for (label, action) in items {
            m.append(Some(label), Some(action));
        }
        m
    };
    let submenu = |sections: &[gio::Menu]| {
        let m = gio::Menu::new();
        for s in sections {
            m.append_section(None, s);
        }
        m
    };

    let file = submenu(&[
        section(&[("Open…", "win.open")]),
        section(&[("Close Window", "window.close")]),
        section(&[("Quit pdfrs", "app.quit")]),
    ]);
    let view = submenu(&[
        section(&[
            ("Single Page", "win.layout::single"),
            ("Continuous", "win.layout::continuous"),
            ("Two Pages", "win.layout::two-page"),
            ("Two Pages, Continuous", "win.layout::two-page-continuous"),
        ]),
        section(&[("Show Cover Page Alone", "win.cover-alone")]),
        section(&[
            ("Actual Size", "win.zoom-actual"),
            ("Fit Width", "win.fit-width"),
            ("Fit Page", "win.fit-page"),
            ("Zoom In", "win.zoom-in"),
            ("Zoom Out", "win.zoom-out"),
        ]),
        section(&[
            ("Show Sidebar", "win.show-sidebar"),
            ("Thumbnails", "win.sidebar-page::thumbnails"),
            ("Contents", "win.sidebar-page::outline"),
        ]),
    ]);
    let go = submenu(&[
        section(&[
            ("Previous Page", "win.previous-page"),
            ("Next Page", "win.next-page"),
        ]),
        section(&[
            ("First Page", "win.first-page"),
            ("Last Page", "win.last-page"),
        ]),
        section(&[("Go to Page…", "win.go-to-page")]),
    ]);

    let bar = gio::Menu::new();
    bar.append_submenu(Some("File"), &file);
    bar.append_submenu(Some("View"), &view);
    bar.append_submenu(Some("Go"), &go);
    bar
}
