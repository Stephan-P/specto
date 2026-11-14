//! Printing, with GTK's print dialog (`GtkPrintOperation`).
//!
//! The document is printed as edited: its pages in their order, with the
//! user's rotation. Each page is scaled to fit the printable area of the
//! paper and centred; the paper is turned to landscape for pages wider
//! than tall. Pages are rendered as images at [`DPI`] on a thread of their
//! own, using GTK's deferred drawing, so the window stays responsive while
//! a large page renders.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{cairo, glib};
use pdfrs_render::{Bitmap, Document as _, HayroDocument, RenderRequest};

use crate::document::PageBundle;

/// Rendering resolution: plenty for text on paper, and a letter page is
/// then 2550 × 3300 pixels.
const DPI: f64 = 300.0;

thread_local! {
    /// The settings from the last print (printer, copies, …), offered again
    /// the next time.
    static SETTINGS: RefCell<Option<gtk::PrintSettings>> = const { RefCell::new(None) };
}

/// What to print and how the dialog starts.
pub struct Job {
    /// Every page of the document, as edited.
    pub pages: PageBundle,
    /// The document's name, for the print queue.
    pub name: String,
    /// The position shown, for "Current Page".
    pub current: usize,
    /// Positions selected in the sidebar, for "Selection" (if several).
    pub selected: Vec<usize>,
}

/// How a print ended, if not as planned.
pub enum Outcome {
    Done,
    Cancelled,
    Failed(String),
}

/// Shows the print dialog for `job` (or, with `export` set, writes the
/// result to that PDF file without a dialog, for tests) and prints.
/// `done` gets the outcome.
pub fn print(
    job: Job,
    parent: &impl IsA<gtk::Window>,
    export: Option<(std::path::PathBuf, bool)>,
    done: impl Fn(Outcome) + 'static,
) {
    let op = gtk::PrintOperation::new();
    op.set_job_name(&job.name);
    op.set_n_pages(job.pages.page_count() as i32);
    op.set_current_page(job.current as i32);
    op.set_unit(gtk::Unit::Points);
    op.set_embed_page_setup(true);
    op.set_allow_async(true);
    op.set_support_selection(true);
    op.set_has_selection(job.selected.len() > 1);
    if let Some(settings) = SETTINGS.with_borrow(Clone::clone) {
        op.set_print_settings(Some(&settings));
    }

    let renderer = Renderer::start(&job.pages);
    // Sizes of the pages as displayed, in points.
    let sizes: Vec<(f64, f64)> = renderer.sizes.clone();
    // Positions printed, by the print operation's page number: all of
    // them, or the selection.
    let printed: Rc<RefCell<Vec<usize>>> = Rc::new(RefCell::new((0..sizes.len()).collect()));

    let (p, selected) = (printed.clone(), job.selected.clone());
    op.connect_begin_print(move |op, _| {
        let settings = op.print_settings();
        if settings.is_some_and(|s| s.print_pages() == gtk::PrintPages::Selection) {
            op.set_n_pages(selected.len() as i32);
            p.replace(selected.clone());
        }
    });
    let (p, s) = (printed.clone(), sizes.clone());
    op.connect_request_page_setup(move |_, _, page_nr, setup| {
        let Some(&(w, h)) = p.borrow().get(page_nr as usize).and_then(|&i| s.get(i)) else {
            return;
        };
        setup.set_orientation(if w > h {
            gtk::PageOrientation::Landscape
        } else {
            gtk::PageOrientation::Portrait
        });
    });
    let renderer = Rc::new(renderer);
    op.connect_draw_page(move |op, context, page_nr| {
        let Some(position) = printed.borrow().get(page_nr as usize).copied() else {
            return;
        };
        let (w, h) = sizes[position];
        // Fitted to the printable area, centred.
        let (area_w, area_h) = (context.width(), context.height());
        let fit = (area_w / w).min(area_h / h);
        let (out_w, out_h) = (w * fit, h * fit);
        let (x, y) = ((area_w - out_w) / 2.0, (area_h - out_h) / 2.0);
        // Pixels at DPI, but no more than the renderer allows.
        let scale = (fit * DPI / 72.0).min(16_000.0 / w.max(h));
        op.set_defer_drawing();
        let (op, context) = (op.clone(), context.clone());
        let rx = renderer.render(position, scale);
        glib::spawn_future_local(async move {
            match rx.recv().await {
                Ok(Ok(bitmap)) => {
                    if let Err(e) = draw(&context.cairo_context(), &bitmap, x, y, out_w, out_h) {
                        glib::g_warning!("pdfrs", "printing page {}: {e}", position + 1);
                    }
                }
                Ok(Err(e)) => glib::g_warning!("pdfrs", "printing page {}: {e}", position + 1),
                Err(_) => {}
            }
            op.draw_page_finish();
        });
    });

    let finished = Rc::new(Cell::new(false));
    op.connect_done(move |op, result| {
        if finished.replace(true) {
            return;
        }
        let outcome = match result {
            gtk::PrintOperationResult::Apply => {
                SETTINGS.with_borrow_mut(|s| *s = op.print_settings());
                Outcome::Done
            }
            gtk::PrintOperationResult::Cancel => Outcome::Cancelled,
            _ => Outcome::Failed(
                op.error()
                    .map_or_else(|| "printing failed".to_owned(), |e| e.message().to_owned()),
            ),
        };
        done(outcome);
    });

    let result = match export {
        Some((path, selection)) => {
            op.set_export_filename(&path);
            if selection {
                let settings = gtk::PrintSettings::new();
                settings.set_print_pages(gtk::PrintPages::Selection);
                op.set_print_settings(Some(&settings));
            }
            op.run(gtk::PrintOperationAction::Export, Some(parent))
        }
        None => op.run(gtk::PrintOperationAction::PrintDialog, Some(parent)),
    };
    if let Err(e) = result {
        glib::g_warning!("pdfrs", "printing: {e}");
    }
}

/// Paints `bitmap` (premultiplied RGBA) into the rectangle at `x`, `y` of
/// size `w` × `h`, in the context's units.
fn draw(
    cr: &cairo::Context,
    bitmap: &Bitmap,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
) -> Result<(), cairo::Error> {
    let (bw, bh) = (bitmap.width as i32, bitmap.height as i32);
    let stride = cairo::Format::ARgb32.stride_for_width(bitmap.width)?;
    // Cairo's ARGB32 is native-endian 32-bit words: B, G, R, A in memory
    // on little-endian machines.
    let mut data = vec![0u8; stride as usize * bh as usize];
    for (row, out) in bitmap
        .data
        .chunks_exact(bitmap.width as usize * 4)
        .zip(data.chunks_exact_mut(stride as usize))
    {
        for (px, o) in row
            .as_chunks::<4>()
            .0
            .iter()
            .zip(out.as_chunks_mut::<4>().0)
        {
            let word = u32::from_be_bytes([px[3], px[0], px[1], px[2]]);
            *o = word.to_ne_bytes();
        }
    }
    let surface =
        cairo::ImageSurface::create_for_data(data, cairo::Format::ARgb32, bw, bh, stride)?;
    cr.save()?;
    cr.translate(x, y);
    cr.scale(w / f64::from(bw), h / f64::from(bh));
    cr.set_source_surface(&surface, 0.0, 0.0)?;
    cr.paint()?;
    cr.restore()
}

/// Renders pages for printing on a thread of its own, which opens each file
/// once.
struct Renderer {
    /// Each page's size as displayed, in points.
    sizes: Vec<(f64, f64)>,
    requests: std::sync::mpsc::Sender<Request>,
}

type Rendered = Result<Bitmap, String>;

struct Request {
    position: usize,
    scale: f64,
    reply: async_channel::Sender<Rendered>,
}

impl Renderer {
    fn start(pages: &PageBundle) -> Self {
        let files = pages.files().to_vec();
        let list = pages.pages().to_vec();
        // Sizes come from the files, opened here once more (cheap next to
        // rendering); a file that doesn't open gives letter-sized blanks.
        let mut docs: HashMap<usize, Option<HayroDocument>> = HashMap::new();
        let sizes = list
            .iter()
            .map(|&(file, page, rotation)| {
                let doc = docs.entry(file).or_insert_with(|| {
                    let (data, password) = &files[file];
                    HayroDocument::open(data.clone(), password.as_deref()).ok()
                });
                doc.as_ref()
                    .and_then(|d| d.page_geometry(page as usize))
                    .map_or((612.0, 792.0), |g| {
                        let s = g.rotated(rotation).size();
                        (s.width, s.height)
                    })
            })
            .collect();
        drop(docs);
        let (tx, rx) = std::sync::mpsc::channel::<Request>();
        std::thread::Builder::new()
            .name("print".into())
            .spawn(move || {
                let mut docs: HashMap<usize, Option<HayroDocument>> = HashMap::new();
                while let Ok(r) = rx.recv() {
                    let Some(&(file, page, rotation)) = list.get(r.position) else {
                        continue;
                    };
                    let doc = docs.entry(file).or_insert_with(|| {
                        let (data, password) = &files[file];
                        HayroDocument::open(data.clone(), password.as_deref()).ok()
                    });
                    let result = match doc {
                        Some(d) => {
                            let mut req = RenderRequest::page(page as usize, r.scale);
                            req.rotation = rotation;
                            d.render(&req).map_err(|e| e.to_string())
                        }
                        None => Err("couldn't open the file".to_owned()),
                    };
                    let _ = r.reply.send_blocking(result);
                }
            })
            .expect("print thread");
        Self {
            sizes,
            requests: tx,
        }
    }

    /// Renders the page at `position` at `scale` pixels per point.
    fn render(&self, position: usize, scale: f64) -> async_channel::Receiver<Rendered> {
        let (reply, rx) = async_channel::bounded(1);
        let _ = self.requests.send(Request {
            position,
            scale,
            reply,
        });
        rx
    }
}
