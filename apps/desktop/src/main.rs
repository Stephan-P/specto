//! pdfrs desktop app (GTK4 + libadwaita).
//!
//! `pdfrs-gtk [FILE…]` opens each file in its own window.

mod canvas;
mod debug;
mod document;
mod outline;
mod print;
mod state;
mod thumbnails;
mod window;

use adw::prelude::*;
use gtk::{gio, glib};

use window::Window;

/// The application ID: D-Bus name, `.desktop` file and icon name, Flatpak
/// ID. The files in `data/` are named after it; renaming the app means
/// changing this, those file names and their contents.
pub const APP_ID: &str = "net.xfbs.pdfrs";

/// The name shown to people: window titles, the macOS menu.
pub const APP_NAME: &str = "pdfrs";

fn main() -> glib::ExitCode {
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build();

    glib::set_application_name(APP_NAME);
    app.connect_startup(|app| {
        gtk::Window::set_default_icon_name(APP_ID);
        for (action, accels) in [
            ("win.open", &["<Primary>o"][..]),
            ("win.save", &["<Primary>s"]),
            ("win.save-as", &["<Shift><Primary>s"]),
            ("win.print", &["<Primary>p"]),
            (
                "win.zoom-in",
                &["<Primary>plus", "<Primary>equal", "<Primary>KP_Add"],
            ),
            ("win.zoom-out", &["<Primary>minus", "<Primary>KP_Subtract"]),
            ("win.zoom-actual", &["<Primary>0", "<Primary>KP_0"]),
            ("win.fit-width", &["<Primary>8"]),
            ("win.fit-page", &["<Primary>9"]),
            ("win.go-to-page", &["<Primary>l"]),
            ("win.copy", &["<Primary>c"]),
            ("win.select-all", &["<Primary>a"]),
            ("win.find", &["<Primary>f"]),
            ("win.find-next", &["<Primary>g"]),
            ("win.find-previous", &["<Shift><Primary>g"]),
            ("win.undo", &["<Primary>z"]),
            ("win.redo", &["<Shift><Primary>z"]),
            ("win.rotate-left", &["<Primary>Left"]),
            ("win.rotate-right", &["<Primary>Right"]),
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
        .filter(|a| a.contains("<Primary>"))
        .map(|a| a.replace("<Primary>", "<Meta>"))
        .collect();
    out.extend(primary);
    out
}

/// The application menu bar (macOS only): File, Edit, View, Go and Tools,
/// like Preview.
fn menubar() -> gio::Menu {
    let quit = format!("Quit {APP_NAME}");
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
        section(&[
            ("Save", "win.save"),
            ("Save As…", "win.save-as"),
            ("Export Selected Pages…", "win.export-pages"),
        ]),
        section(&[("Print…", "win.print")]),
        section(&[("Close Window", "window.close")]),
        section(&[(&quit, "app.quit")]),
    ]);
    let edit = submenu(&[
        section(&[("Undo", "win.undo"), ("Redo", "win.redo")]),
        section(&[("Copy", "win.copy"), ("Select All", "win.select-all")]),
        section(&[
            ("Insert Pages from File…", "win.insert-file"),
            ("Delete Pages", "win.delete-pages"),
        ]),
        section(&[
            ("Find…", "win.find"),
            ("Find Next", "win.find-next"),
            ("Find Previous", "win.find-previous"),
        ]),
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

    let tools = submenu(&[section(&[
        ("Rotate Left", "win.rotate-left"),
        ("Rotate Right", "win.rotate-right"),
    ])]);

    let bar = gio::Menu::new();
    bar.append_submenu(Some("File"), &file);
    bar.append_submenu(Some("Edit"), &edit);
    bar.append_submenu(Some("View"), &view);
    bar.append_submenu(Some("Go"), &go);
    bar.append_submenu(Some("Tools"), &tools);
    bar
}
