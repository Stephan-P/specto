//! Signatures: made once (drawn, or taken from a photo), kept with the app's data, and placed on
//! pages as often as wanted.
//!
//! One file a signature in `signatures` in the app's data directory: the strokes of a drawn one as
//! text, a signature from a photo as a PNG with a transparent background. They are pictures of a
//! signature. Placing one doesn't sign a document in the cryptographic sense, and nothing here says
//! it does.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use specto_annot::{Color, InkSettings, StampImage, ink_from_photo};

use crate::i18n::gettext;

/// A signature, as it is kept.
#[derive(Clone, Debug, PartialEq)]
pub enum Signature {
    /// Strokes in a box of `size`, y pointing down.
    ///
    /// They stay sharp at any size, and are placed as a drawing.
    Drawn {
        strokes: Vec<Vec<(f64, f64)>>,
        size: (f64, f64),
    },
    /// Ink on a transparent background, placed as a picture.
    Photo(Arc<StampImage>),
}

impl Signature {
    /// Width by height.
    pub fn size(&self) -> (f64, f64) {
        match self {
            Self::Drawn { size, .. } => *size,
            Self::Photo(image) => (f64::from(image.width), f64::from(image.height)),
        }
    }
}

/// Strokes as they are drawn: each a run of points, y pointing down.
type Strokes = Vec<Vec<(f64, f64)>>;

/// The width of the pen a signature is drawn with, in the units of its strokes (pixels of the box
/// it was drawn in).
pub const PEN: f64 = 2.5;

/// Where signatures are kept.
fn dir() -> PathBuf {
    glib::user_data_dir().join("specto").join("signatures")
}

/// The saved signatures by name, oldest first. Files that can't be read are left out.
pub fn saved() -> Vec<(String, Signature)> {
    let Ok(entries) = std::fs::read_dir(dir()) else {
        return Vec::new();
    };
    let mut all: Vec<(String, Signature)> = entries
        .filter_map(Result::ok)
        .filter_map(|e| {
            let path = e.path();
            let name = path.file_stem()?.to_str()?.to_owned();
            let signature = match path.extension()?.to_str()? {
                "strokes" => read_strokes(&std::fs::read_to_string(&path).ok()?)?,
                "png" => Signature::Photo(Arc::new(read_image(&gio::File::for_path(&path))?)),
                _ => return None,
            };
            Some((name, signature))
        })
        .collect();
    all.sort_by(|a, b| a.0.cmp(&b.0));
    all
}

/// Keeps `signature`. Returns the name it got.
pub fn save(signature: &Signature) -> std::io::Result<String> {
    let dir = dir();
    std::fs::create_dir_all(&dir)?;
    // Named by when it was made, which also puts them in order.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let name = (0..)
        .map(|n| format!("{now:014}-{n}"))
        .find(|name| {
            !["strokes", "png"]
                .iter()
                .any(|ext| dir.join(format!("{name}.{ext}")).exists())
        })
        .expect("a free name");
    match signature {
        Signature::Drawn { strokes, size } => {
            let mut text = format!("specto-signature 1\nsize {} {}\n", size.0, size.1);
            for stroke in strokes {
                let line: Vec<String> = stroke
                    .iter()
                    .map(|(x, y)| format!("{x:.2} {y:.2}"))
                    .collect();
                text.push_str(&line.join(" "));
                text.push('\n');
            }
            std::fs::write(dir.join(format!("{name}.strokes")), text)?;
        }
        Signature::Photo(image) => {
            texture_of(image)
                .save_to_png(dir.join(format!("{name}.png")))
                .map_err(std::io::Error::other)?;
        }
    }
    Ok(name)
}

/// Forgets the signature called `name`.
pub fn delete(name: &str) {
    for ext in ["strokes", "png"] {
        let _ = std::fs::remove_file(dir().join(format!("{name}.{ext}")));
    }
}

fn read_strokes(text: &str) -> Option<Signature> {
    let mut lines = text.lines();
    if lines.next()? != "specto-signature 1" {
        return None;
    }
    let numbers = |line: &str| -> Option<Vec<f64>> {
        line.split_ascii_whitespace()
            .map(|n| n.parse::<f64>().ok().filter(|v| v.is_finite()))
            .collect()
    };
    let size = numbers(lines.next()?.strip_prefix("size ")?)?;
    let [w, h] = size[..] else { return None };
    let strokes: Vec<Vec<(f64, f64)>> = lines
        .filter_map(|line| {
            let n = numbers(line)?;
            let (pairs, _) = n.as_chunks::<2>();
            (!pairs.is_empty()).then(|| pairs.iter().map(|&[x, y]| (x, y)).collect())
        })
        .collect();
    (w > 0.0 && h > 0.0 && !strokes.is_empty()).then_some(Signature::Drawn {
        strokes,
        size: (w, h),
    })
}

/// The picture in `file`, in any format GTK reads.
fn read_image(file: &gio::File) -> Option<StampImage> {
    let texture = gdk::Texture::from_file(file).ok()?;
    let mut downloader = gdk::TextureDownloader::new(&texture);
    downloader.set_format(gdk::MemoryFormat::R8g8b8a8);
    let (bytes, stride) = downloader.download_bytes();
    let (width, height) = (texture.width() as usize, texture.height() as usize);
    let mut rgba = Vec::with_capacity(width * height * 4);
    for row in bytes.chunks(stride).take(height) {
        rgba.extend_from_slice(row.get(..width * 4)?);
    }
    Some(StampImage {
        width: width as u32,
        height: height as u32,
        rgba,
    })
}

fn texture_of(image: &StampImage) -> gdk::Texture {
    gdk::MemoryTexture::new(
        image.width as i32,
        image.height as i32,
        gdk::MemoryFormat::R8g8b8a8,
        &glib::Bytes::from(&image.rgba[..]),
        image.width as usize * 4,
    )
    .upcast()
}

/// Strokes as a signature of their own size: moved so that they start at the top left of the box
/// around them.
fn trimmed(strokes: &[Vec<(f64, f64)>]) -> Option<Signature> {
    let points = || strokes.iter().flatten();
    let min = |f: fn(&(f64, f64)) -> f64| points().map(f).fold(f64::INFINITY, f64::min);
    let max = |f: fn(&(f64, f64)) -> f64| points().map(f).fold(f64::NEG_INFINITY, f64::max);
    let (x0, y0) = (min(|p| p.0), min(|p| p.1));
    let (w, h) = (max(|p| p.0) - x0, max(|p| p.1) - y0);
    // Something of a size: not a dot, not a slip of the hand.
    if !(w >= 8.0 || h >= 8.0) {
        return None;
    }
    Some(Signature::Drawn {
        strokes: strokes
            .iter()
            .filter(|s| !s.is_empty())
            .map(|s| s.iter().map(|(x, y)| (x - x0, y - y0)).collect())
            .collect(),
        size: (w.max(1.0), h.max(1.0)),
    })
}

/// Draws `strokes` with cairo, scaled by `scale` and moved by `offset`.
fn draw_strokes(
    cr: &gtk::cairo::Context,
    strokes: &[Vec<(f64, f64)>],
    scale: f64,
    offset: (f64, f64),
) {
    cr.set_line_cap(gtk::cairo::LineCap::Round);
    cr.set_line_join(gtk::cairo::LineJoin::Round);
    cr.set_line_width((PEN * scale).max(1.0));
    for stroke in strokes {
        let Some(&(x, y)) = stroke.first() else {
            continue;
        };
        cr.move_to(offset.0 + x * scale, offset.1 + y * scale);
        // A single point is a dot: a line to itself, with round ends.
        for &(x, y) in stroke
            .iter()
            .skip(1)
            .chain(stroke.first().filter(|_| stroke.len() == 1))
        {
            cr.line_to(offset.0 + x * scale, offset.1 + y * scale);
        }
        let _ = cr.stroke();
    }
}

/// A small picture of `signature`, for a list to choose from.
///
/// Drawn dark on a white slip, as on paper, whatever the style.
pub fn preview(signature: &Signature) -> gtk::Widget {
    const WIDTH: i32 = 180;
    const HEIGHT: i32 = 64;
    let area = gtk::DrawingArea::builder()
        .content_width(WIDTH)
        .content_height(HEIGHT)
        .build();
    let signature = signature.clone();
    let picture = match &signature {
        Signature::Photo(image) => Some(texture_of(image)),
        Signature::Drawn { .. } => None,
    };
    area.set_draw_func(move |_, cr, width, height| {
        let (w, h) = (f64::from(width), f64::from(height));
        cr.set_source_rgb(1.0, 1.0, 1.0);
        let _ = cr.paint();
        let (sw, sh) = signature.size();
        let scale = ((w - 12.0) / sw).min((h - 12.0) / sh);
        let offset = ((w - sw * scale) / 2.0, (h - sh * scale) / 2.0);
        match &signature {
            Signature::Drawn { strokes, .. } => {
                cr.set_source_rgb(0.1, 0.1, 0.1);
                draw_strokes(cr, strokes, scale, offset);
            }
            Signature::Photo(image) => {
                // Through a snapshot would be the GTK 4 way, but a cairo surface of the pixels does
                // for a preview.
                let _ = picture.as_ref();
                let mut data = Vec::with_capacity(image.rgba.len());
                for p in image.rgba.as_chunks::<4>().0.iter() {
                    let a = u32::from(p[3]);
                    let pre = |c: u8| (u32::from(c) * a / 255) as u8;
                    // Cairo's ARGB32: blue, green, red, alpha in memory on little-endian machines,
                    // colors multiplied by alpha.
                    data.extend(if cfg!(target_endian = "little") {
                        [pre(p[2]), pre(p[1]), pre(p[0]), p[3]]
                    } else {
                        [p[3], pre(p[0]), pre(p[1]), pre(p[2])]
                    });
                }
                let surface = gtk::cairo::ImageSurface::create_for_data(
                    data,
                    gtk::cairo::Format::ARgb32,
                    image.width as i32,
                    image.height as i32,
                    image.width as i32 * 4,
                );
                if let Ok(surface) = surface {
                    let _ = cr.save();
                    cr.translate(offset.0, offset.1);
                    cr.scale(scale, scale);
                    let _ = cr.set_source_surface(&surface, 0.0, 0.0);
                    let _ = cr.paint();
                    let _ = cr.restore();
                }
            }
        }
    });
    area.upcast()
}

/// Shows the dialog for making a signature: drawn with the pointer, or taken from a picture of one
/// on paper.
///
/// `done` gets the signature once it is saved, with its name.
pub fn new_signature(parent: &impl IsA<gtk::Widget>, done: impl Fn(String, Signature) + 'static) {
    let stack = adw::ViewStack::new();

    // Drawn
    let strokes: Rc<RefCell<Strokes>> = Rc::default();
    let pad = gtk::DrawingArea::builder()
        .content_width(480)
        .content_height(200)
        .hexpand(true)
        .css_classes(["card"])
        .build();
    let shown = strokes.clone();
    pad.set_draw_func(move |_, cr, width, height| {
        let (w, h) = (f64::from(width), f64::from(height));
        cr.set_source_rgb(1.0, 1.0, 1.0);
        let _ = cr.paint();
        // A line to sign on.
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.15);
        cr.set_line_width(1.0);
        cr.move_to(24.0, (h * 0.72).round() + 0.5);
        cr.line_to(w - 24.0, (h * 0.72).round() + 0.5);
        let _ = cr.stroke();
        cr.set_source_rgb(0.1, 0.1, 0.1);
        draw_strokes(cr, &shown.borrow(), 1.0, (0.0, 0.0));
    });
    let drag = gtk::GestureDrag::new();
    let start = Rc::new(Cell::new((0.0, 0.0)));
    let (drawn, from, area) = (strokes.clone(), start.clone(), pad.clone());
    drag.connect_drag_begin(move |_, x, y| {
        from.set((x, y));
        drawn.borrow_mut().push(vec![(x, y)]);
        area.queue_draw();
    });
    let (drawn, area) = (strokes.clone(), pad.clone());
    drag.connect_drag_update(move |_, dx, dy| {
        let (x0, y0) = start.get();
        let at = (x0 + dx, y0 + dy);
        if let Some(stroke) = drawn.borrow_mut().last_mut() {
            // Not every twitch: a point about every pixel.
            let far = stroke
                .last()
                .is_none_or(|l| (l.0 - at.0).hypot(l.1 - at.1) >= 1.0);
            if far {
                stroke.push(at);
            }
        }
        area.queue_draw();
    });
    pad.add_controller(drag);
    let clear = gtk::Button::builder()
        .label(gettext("Clear"))
        .halign(gtk::Align::Center)
        .build();
    let (drawn, area) = (strokes.clone(), pad.clone());
    clear.connect_clicked(move |_| {
        drawn.borrow_mut().clear();
        area.queue_draw();
    });
    let draw_page = gtk::Box::new(gtk::Orientation::Vertical, 12);
    draw_page.append(&pad);
    draw_page.append(
        &gtk::Label::builder()
            .label(gettext("Sign with the mouse or the touchpad"))
            .css_classes(["dim-label"])
            .build(),
    );
    draw_page.append(&clear);
    stack.add_titled_with_icon(
        &draw_page,
        Some("draw"),
        &gettext("Draw"),
        "document-edit-symbolic",
    );

    // From a picture

    // The picture chosen, and the signature made of it with the settings.
    let photo: Rc<RefCell<Option<StampImage>>> = Rc::default();
    let result: Rc<RefCell<Option<Arc<StampImage>>>> = Rc::default();
    let result_view = gtk::Picture::builder()
        .content_fit(gtk::ContentFit::Contain)
        .height_request(200)
        .hexpand(true)
        .css_classes(["card", "signature-paper"])
        .build();
    let hint = gtk::Label::builder()
        .label(gettext(
            "Sign on white paper, take a picture of it, and choose that picture. The paper is taken away, and the ink stays.",
        ))
        .wrap(true)
        .justify(gtk::Justification::Center)
        .css_classes(["dim-label"])
        .build();
    let choose = gtk::Button::builder()
        .label(gettext("Choose Picture…"))
        .halign(gtk::Align::Center)
        .build();
    let threshold = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.05, 0.8, 0.01);
    threshold.set_value(f64::from(InkSettings::default().threshold));
    threshold.set_hexpand(true);
    threshold.set_tooltip_text(Some(&gettext(
        "To the left, faint strokes are kept, and more of the paper's shadows. To the right, only what is dark.",
    )));
    let ink = gtk::DropDown::from_strings(&[
        &gettext("Ink as in the picture"),
        &gettext("Black ink"),
        &gettext("Blue ink"),
    ]);
    let dials = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    dials.append(&gtk::Label::new(Some(&gettext("Faint"))));
    dials.append(&threshold);
    dials.append(&gtk::Label::new(Some(&gettext("Dark"))));
    dials.append(&ink);
    dials.set_sensitive(false);
    let photo_page = gtk::Box::new(gtk::Orientation::Vertical, 12);
    photo_page.append(&result_view);
    photo_page.append(&hint);
    photo_page.append(&dials);
    photo_page.append(&choose);
    stack.add_titled_with_icon(
        &photo_page,
        Some("photo"),
        &gettext("From a Picture"),
        "image-x-generic-symbolic",
    );

    // Makes the signature of the picture anew, after a setting changed.
    let develop: Rc<dyn Fn()> = {
        let (photo, result, view) = (photo.clone(), result.clone(), result_view.clone());
        let (threshold, ink, hint) = (threshold.clone(), ink.clone(), hint.clone());
        Rc::new(move || {
            let Some(source) = photo.borrow().clone() else {
                return;
            };
            let settings = InkSettings {
                threshold: threshold.value() as f32,
                color: match ink.selected() {
                    1 => Some(Color::new(0.08, 0.08, 0.08)),
                    2 => Some(Color::new(0.05, 0.15, 0.55)),
                    _ => None,
                },
            };
            let made = ink_from_photo(&source, settings).map(Arc::new);
            view.set_paintable(made.as_deref().map(texture_of).as_ref());
            hint.set_label(&if made.is_some() {
                gettext("Move the slider until the paper is gone and the strokes are whole.")
            } else {
                gettext("No ink found in this picture. Move the slider to the left.")
            });
            result.replace(made);
        })
    };
    let again = develop.clone();
    threshold.connect_value_changed(move |_| again());
    let again = develop.clone();
    ink.connect_selected_notify(move |_| again());
    let (photo_in, dials_on, again, widget) = (
        photo.clone(),
        dials.clone(),
        develop.clone(),
        parent.clone().upcast::<gtk::Widget>(),
    );
    choose.connect_clicked(move |_| {
        let filter = gtk::FileFilter::new();
        filter.set_name(Some(&gettext("Pictures")));
        filter.add_pixbuf_formats();
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let dialog = gtk::FileDialog::builder()
            .title(gettext("Choose a Picture of a Signature"))
            .filters(&filters)
            .build();
        let (photo, dials, develop) = (photo_in.clone(), dials_on.clone(), again.clone());
        let window = widget.root().and_downcast::<gtk::Window>();
        glib::spawn_future_local(async move {
            if let Ok(file) = dialog.open_future(window.as_ref()).await
                && let Some(image) = read_image(&file)
            {
                photo.replace(Some(image));
                dials.set_sensitive(true);
                develop();
            }
        });
    });

    // The dialog
    let cancel = gtk::Button::with_mnemonic(&gettext("_Cancel"));
    let save = gtk::Button::builder()
        .label(gettext("_Save"))
        .use_underline(true)
        .css_classes(["suggested-action"])
        .build();
    let header = adw::HeaderBar::builder()
        .show_start_title_buttons(false)
        .show_end_title_buttons(false)
        .title_widget(
            &adw::ViewSwitcher::builder()
                .stack(&stack)
                .policy(adw::ViewSwitcherPolicy::Wide)
                .build(),
        )
        .build();
    header.pack_start(&cancel);
    header.pack_end(&save);
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .margin_top(18)
        .margin_bottom(18)
        .margin_start(18)
        .margin_end(18)
        .build();
    content.append(&stack);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&content));
    let dialog = adw::Dialog::builder()
        .title(gettext("New Signature"))
        .content_width(560)
        .child(&view)
        .build();
    let closing = dialog.clone();
    cancel.connect_clicked(move |_| {
        closing.close();
    });
    let closing = dialog.clone();
    save.connect_clicked(move |_| {
        let signature = match stack.visible_child_name().as_deref() {
            Some("photo") => result.borrow().clone().map(Signature::Photo),
            _ => trimmed(&strokes.borrow()),
        };
        // Nothing drawn or developed: nothing to save, and the dialog stays.
        let Some(signature) = signature else {
            closing
                .root()
                .and_downcast::<gtk::Window>()
                .inspect(gtk::Window::error_bell);
            return;
        };
        if let Ok(name) = self::save(&signature) {
            done(name, signature);
        }
        closing.close();
    });
    dialog.present(Some(parent));
}

#[cfg(test)]
mod tests {
    use super::{Signature, read_strokes, trimmed};

    #[test]
    fn strokes_are_trimmed_and_read_back() {
        let strokes = vec![vec![(110.0, 60.0), (150.0, 40.0)], vec![(130.0, 90.0)]];
        let Some(Signature::Drawn { strokes, size }) = trimmed(&strokes) else {
            panic!("a signature");
        };
        assert_eq!(size, (40.0, 50.0));
        assert_eq!(strokes[0], [(0.0, 20.0), (40.0, 0.0)]);
        assert_eq!(strokes[1], [(20.0, 50.0)]);
        // A dot is no signature.
        assert!(trimmed(&[vec![(5.0, 5.0), (6.0, 6.0)]]).is_none());
        assert!(trimmed(&[]).is_none());

        let text = "specto-signature 1\nsize 40 50\n0.00 20.00 40.00 0.00\n20.00 50.00\n";
        assert_eq!(read_strokes(text), Some(Signature::Drawn { strokes, size }));
        assert!(read_strokes("something else\n").is_none());
        assert!(read_strokes("specto-signature 1\nsize 0 0\n1 2\n").is_none());
        assert!(read_strokes("specto-signature 1\nsize 40 50\n").is_none());
    }
}
