//! Test hooks, only registered with `PDFRS_DEBUG=1`.
//!
//! Window actions for driving the app from outside (see
//! `tools/headless/check.py`), called over D-Bus:
//!
//! - `win.debug-state(path)`: writes the window's state as JSON
//! - `win.debug-screenshot(path)`: saves the window at device resolution,
//!   as PNG, or as binary PPM if `path` ends in `.ppm` (easy to read in tests)
//! - `win.debug-go-to(text)`: types `text` into the page entry and presses
//!   Enter
//! - `win.debug-maximize("")`: maximizes the window, which puts it at the
//!   screen's top-left corner, so window coordinates are screen coordinates
//!   (for the virtual pointer in tests)

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};

/// Whether the test hooks are enabled.
pub fn enabled() -> bool {
    std::env::var_os("PDFRS_DEBUG").is_some()
}

/// Saves `window` at device resolution, so HiDPI sharpness shows: as PPM if
/// `path` ends in `.ppm`, otherwise as PNG.
///
/// `WidgetPaintable` doesn't draw the widget; it reuses the render node of
/// the last frame, which any `queue_draw` in the window (a tile arriving)
/// discards. So this requests a frame and saves right after it's painted.
pub fn screenshot(window: &gtk::Widget, path: &str) {
    let Some(clock) = window.frame_clock() else {
        return eprintln!("debug-screenshot: window not shown");
    };
    let handler = Rc::new(Cell::new(None::<glib::SignalHandlerId>));
    let (w, path, h) = (window.clone(), path.to_owned(), handler.clone());
    let tries = Cell::new(0);
    let id = clock.connect_after_paint(move |clock| {
        tries.set(tries.get() + 1);
        let done = save_screenshot(&w, &path);
        if !done && tries.get() >= 10 {
            eprintln!("debug-screenshot: nothing drawn in 10 frames");
        }
        if (done || tries.get() >= 10)
            && let Some(id) = h.take()
        {
            clock.disconnect(id);
        }
    });
    handler.set(Some(id));
    window.queue_draw();
}

/// Saves the screenshot; false if the window drew nothing.
fn save_screenshot(window: &gtk::Widget, path: &str) -> bool {
    let scale = window
        .native()
        .and_then(|n| n.surface())
        .map_or(1.0, |s| s.scale()) as f32;
    let paintable = gtk::WidgetPaintable::new(Some(window));
    let snapshot = gtk::Snapshot::new();
    snapshot.scale(scale, scale);
    paintable.snapshot(&snapshot, window.width() as f64, window.height() as f64);
    let renderer = window.native().and_then(|n| n.renderer());
    let (Some(node), Some(renderer)) = (snapshot.to_node(), renderer) else {
        return false;
    };
    let texture = renderer.render_texture(&node, None);
    // Written under another name and renamed, so a test waiting for the
    // file never reads it half-written.
    let part = format!("{path}.part");
    let result = if path.ends_with(".ppm") {
        std::fs::write(&part, ppm(&texture)).map_err(|e| e.to_string())
    } else {
        texture.save_to_png(&part).map_err(|e| e.to_string())
    }
    .and_then(|()| std::fs::rename(&part, path).map_err(|e| e.to_string()));
    if let Err(e) = result {
        eprintln!("debug-screenshot: {path}: {e}");
    }
    true
}

/// `texture` as binary PPM (P6).
fn ppm(texture: &gdk::Texture) -> Vec<u8> {
    let mut downloader = gdk::TextureDownloader::new(texture);
    downloader.set_format(gdk::MemoryFormat::R8g8b8);
    let (bytes, stride) = downloader.download_bytes();
    let (w, h) = (texture.width() as usize, texture.height() as usize);
    let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
    for row in bytes.chunks(stride).take(h) {
        out.extend_from_slice(&row[..w * 3]);
    }
    out
}

/// A minimal JSON object writer: string, number and bool values.
#[derive(Default)]
pub struct Json(Vec<String>);

impl Json {
    pub fn str(&mut self, key: &str, value: &str) -> &mut Self {
        self.0.push(format!("{}: {}", quote(key), quote(value)));
        self
    }

    pub fn num(&mut self, key: &str, value: f64) -> &mut Self {
        self.0.push(format!("{}: {value}", quote(key)));
        self
    }

    /// A value that is already JSON (an array, say).
    pub fn raw(&mut self, key: &str, json: &str) -> &mut Self {
        self.0.push(format!("{}: {json}", quote(key)));
        self
    }

    pub fn bool(&mut self, key: &str, value: bool) -> &mut Self {
        self.0.push(format!("{}: {value}", quote(key)));
        self
    }

    pub fn finish(&self) -> String {
        format!("{{{}}}\n", self.0.join(", "))
    }
}

fn quote(s: &str) -> String {
    let mut out = String::from('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json() {
        let s = Json::default()
            .str("a", "x \"y\"\n")
            .num("n", 1.5)
            .bool("b", true)
            .finish();
        assert_eq!(
            s,
            "{\"a\": \"x \\\"y\\\"\\u000a\", \"n\": 1.5, \"b\": true}\n"
        );
    }
}
