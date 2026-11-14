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
            ("win.open", &["<Ctrl>o"][..]),
            (
                "win.zoom-in",
                &["<Ctrl>plus", "<Ctrl>equal", "<Ctrl>KP_Add"],
            ),
            ("win.zoom-out", &["<Ctrl>minus", "<Ctrl>KP_Subtract"]),
            ("win.zoom-actual", &["<Ctrl>0", "<Ctrl>KP_0"]),
            ("win.fit-width", &["<Ctrl>8"]),
            ("win.fit-page", &["<Ctrl>9"]),
            ("win.go-to-page", &["<Ctrl>l"]),
            ("win.show-sidebar", &["F9"]),
            ("window.close", &["<Ctrl>w"]),
            ("app.quit", &["<Ctrl>q"]),
        ] {
            app.set_accels_for_action(action, accels);
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
