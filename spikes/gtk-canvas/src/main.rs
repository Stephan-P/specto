//! M0 spike: is a custom GTK4 widget fast enough for a tiled PDF canvas?
//!
//!   gtk-canvas <file.pdf>                 # interactive: scroll, ctrl+scroll / pinch to zoom
//!   gtk-canvas <file.pdf> --bench [dir]   # scripted scroll + zoom, prints frame stats,
//!                                         # optionally saves screenshots into dir
//!
//! Throwaway code; the real thing will be driven by `pdfrs-view`.

mod canvas;
mod workers;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use adw::prelude::*;
use gtk::{gdk, glib};
use pdfrs_render::{Document, HayroDocument};

use canvas::{PdfCanvas, State};
use workers::Pool;

fn main() -> glib::ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let Some(path) = args.get(1).map(PathBuf::from) else {
        eprintln!("usage: gtk-canvas <file.pdf> [--bench [screenshot-dir]]");
        return glib::ExitCode::FAILURE;
    };
    let bench = args.iter().any(|a| a == "--bench");
    let shots = args
        .iter()
        .position(|a| a == "--bench")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from);

    let app = adw::Application::builder()
        .application_id("dev.pdfrs.GtkCanvasSpike")
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.connect_activate(move |app| build(app, &path, bench, shots.clone()));
    app.run_with_args::<&str>(&[])
}

fn build(app: &adw::Application, path: &PathBuf, bench: bool, shots: Option<PathBuf>) {
    let data = Arc::new(std::fs::read(path).expect("read pdf"));
    let doc = HayroDocument::open((*data).clone(), None).expect("open pdf");
    let pages = (0..doc.page_count())
        .map(|i| {
            let s = doc.page_size(i).unwrap();
            (s.width as f64, s.height as f64)
        })
        .collect();
    let state = Rc::new(RefCell::new(State::new(pages)));

    let (tx, rx) = async_channel::unbounded();
    let threads = std::env::var("PDFRS_THREADS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(2, |n| n.get()));
    state.borrow_mut().pool = Some(Pool::new(data, threads, tx));

    let canvas = PdfCanvas::new(state.clone());

    // Results → textures on the main thread.
    {
        let state = state.clone();
        let canvas = canvas.clone();
        glib::spawn_future_local(async move {
            while let Ok(r) = rx.recv().await {
                let bytes = r.bitmap.data.len();
                let texture = gdk::MemoryTexture::new(
                    r.bitmap.width as i32,
                    r.bitmap.height as i32,
                    gdk::MemoryFormat::R8g8b8a8Premultiplied,
                    &glib::Bytes::from_owned(r.bitmap.data),
                    r.bitmap.width as usize * 4,
                );
                let mut s = state.borrow_mut();
                s.stats.tiles_rendered += 1;
                s.stats.tile_ms.push(r.ms);
                s.insert(r.key, texture.upcast(), bytes);
                drop(s);
                canvas.queue_draw();
            }
        });
    }

    // Scrolling; ctrl+scroll zooms around the pointer.
    let scroll = gtk::EventControllerScroll::new(
        gtk::EventControllerScrollFlags::BOTH_AXES | gtk::EventControllerScrollFlags::KINETIC,
    );
    let pointer = Rc::new(RefCell::new((0.0, 0.0)));
    {
        let motion = gtk::EventControllerMotion::new();
        let pointer = pointer.clone();
        motion.connect_motion(move |_, x, y| *pointer.borrow_mut() = (x, y));
        canvas.add_controller(motion);
    }
    {
        let state = state.clone();
        let canvas_ref = canvas.clone();
        let pointer = pointer.clone();
        scroll.connect_scroll(move |ctl, dx, dy| {
            let (w, h) = (canvas_ref.width() as f64, canvas_ref.height() as f64);
            let mut s = state.borrow_mut();
            let discrete = ctl.unit() == gdk::ScrollUnit::Wheel;
            if ctl
                .current_event_state()
                .contains(gdk::ModifierType::CONTROL_MASK)
            {
                let (px, py) = *pointer.borrow();
                s.zoom_at(
                    (-dy * if discrete { 0.15 } else { 0.01 }).exp(),
                    px,
                    py,
                    w,
                    h,
                );
            } else {
                let step = if discrete { 60.0 } else { 1.0 };
                s.scroll_x += dx * step;
                s.scroll_y += dy * step;
                s.clamp_scroll(w, h);
            }
            drop(s);
            canvas_ref.queue_draw();
            glib::Propagation::Stop
        });
    }
    canvas.add_controller(scroll);

    // Touchpad / touchscreen pinch.
    let pinch = gtk::GestureZoom::new();
    {
        let state = state.clone();
        let canvas_ref = canvas.clone();
        let last = Rc::new(RefCell::new(1.0));
        let last_begin = last.clone();
        pinch.connect_begin(move |_, _| *last_begin.borrow_mut() = 1.0);
        pinch.connect_scale_changed(move |g, scale| {
            let (cx, cy) = g.bounding_box_center().unwrap_or((0.0, 0.0));
            let factor = scale / *last.borrow();
            *last.borrow_mut() = scale;
            let (w, h) = (canvas_ref.width() as f64, canvas_ref.height() as f64);
            state.borrow_mut().zoom_at(factor, cx, cy, w, h);
            canvas_ref.queue_draw();
        });
    }
    canvas.add_controller(pinch);

    let header = adw::HeaderBar::new();
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&canvas));
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title(path.file_name().unwrap().to_string_lossy().as_ref())
        .default_width(1100)
        .default_height(900)
        .content(&view)
        .build();
    window.present();

    // Fit width initially.
    {
        let state = state.clone();
        let canvas_ref = canvas.clone();
        canvas.connect_realize(move |_| {
            let mut s = state.borrow_mut();
            let max_w = s.pages.iter().map(|p| p.0).fold(0.0, f64::max);
            s.zoom = ((canvas_ref.width().max(1100) as f64 - 4.0 * canvas::GAP) / max_w).min(1.5);
        });
    }

    if bench {
        run_bench(&window, &canvas, state, shots);
    }
}

/// Scripted interaction. Phases (in seconds since start):
///   0–1   settle
///   1–11  scroll down at 4000 px/s (a fast flick, sustained)
///   11–14 zoom in continuously to 3× around the viewport centre
///   14–19 scroll at 2000 px/s while zoomed in
///   19–21 zoom out to 0.4× (many pages visible)
///   21–24 scroll at 6000 px/s while zoomed out
///   24–26 idle (let tiles settle) → report
fn run_bench(
    window: &adw::ApplicationWindow,
    canvas: &PdfCanvas,
    state: Rc<RefCell<State>>,
    shots: Option<PathBuf>,
) {
    let start = Rc::new(RefCell::new(None::<Instant>));
    let last_tick = Rc::new(RefCell::new(None::<i64>));
    let intervals = Rc::new(RefCell::new(Vec::<f64>::new()));
    let shots_taken = Rc::new(RefCell::new(Vec::<f64>::new()));
    let window = window.clone();
    let base_zoom = Rc::new(RefCell::new(None::<f64>));

    canvas.add_tick_callback(move |canvas, clock| {
        let now = clock.frame_time();
        if let Some(prev) = last_tick.replace(Some(now)) {
            intervals.borrow_mut().push((now - prev) as f64 / 1000.0);
        }
        let t0 = *start.borrow_mut().get_or_insert_with(Instant::now);
        let t = t0.elapsed().as_secs_f64();
        if let Some(&last) = intervals.borrow().last()
            && last > 50.0
        {
            let q = state.borrow().pool.as_ref().map_or(0, |p| p.queue_len());
            eprintln!(
                "long frame: {last:.0} ms at t={t:.2} s (queue {q}, zoom {:.2})",
                state.borrow().zoom
            );
        }
        let dt = intervals.borrow().last().copied().unwrap_or(16.0) / 1000.0;
        let (w, h) = (canvas.width() as f64, canvas.height() as f64);
        {
            let mut s = state.borrow_mut();
            let z0 = *base_zoom.borrow_mut().get_or_insert(s.zoom);
            match t {
                t if (1.0..11.0).contains(&t) => s.scroll_y += 4000.0 * dt,
                t if (11.0..14.0).contains(&t) => {
                    let target = z0 * 3.0;
                    let f = (target / s.zoom).powf((dt / (14.0 - t)).min(1.0));
                    s.zoom_at(f, w / 2.0, h / 2.0, w, h);
                }
                t if (14.0..19.0).contains(&t) => s.scroll_y += 2000.0 * dt,
                t if (19.0..21.0).contains(&t) => {
                    let target = 0.4;
                    let f = (target / s.zoom).powf((dt / (21.0 - t)).min(1.0));
                    s.zoom_at(f, w / 2.0, h / 2.0, w, h);
                }
                t if (21.0..24.0).contains(&t) => s.scroll_y += 6000.0 * dt,
                _ => {}
            }
            s.clamp_scroll(w, h);
        }
        // Screenshots: mid-scroll, while zoomed in, zoomed out, and settled.
        if let Some(dir) = &shots {
            for at in [5.0, 13.9, 16.5, 22.5, 25.9] {
                if t >= at && !shots_taken.borrow().contains(&at) {
                    shots_taken.borrow_mut().push(at);
                    screenshot(canvas, &dir.join(format!("t{at:04.1}.png")));
                }
            }
        }
        canvas.queue_draw();
        if t >= 26.0 {
            report(&state.borrow(), &intervals.borrow(), canvas);
            window.close();
            return glib::ControlFlow::Break;
        }
        glib::ControlFlow::Continue
    });
}

fn screenshot(widget: &PdfCanvas, path: &std::path::Path) {
    let paintable = gtk::WidgetPaintable::new(Some(widget));
    let snap = gtk::Snapshot::new();
    paintable.snapshot(&snap, widget.width() as f64, widget.height() as f64);
    let (Some(node), Some(native)) = (snap.to_node(), widget.native()) else {
        return;
    };
    if let Some(renderer) = native.renderer() {
        let _ = std::fs::create_dir_all(path.parent().unwrap());
        let _ = renderer.render_texture(&node, None).save_to_png(path);
    }
}

fn pct(v: &[f64], q: f64) -> f64 {
    let mut v = v.to_vec();
    v.sort_by(f64::total_cmp);
    v.get(((v.len() as f64 - 1.0) * q) as usize)
        .copied()
        .unwrap_or(0.0)
}

fn report(s: &State, intervals: &[f64], canvas: &PdfCanvas) {
    let scale = canvas
        .native()
        .and_then(|n| n.surface())
        .map_or(1.0, |s| s.scale());
    let renderer = canvas
        .native()
        .and_then(|n| n.renderer())
        .map(|r| r.type_().name().to_string())
        .unwrap_or_default();
    let total: f64 = intervals.iter().sum::<f64>() / 1000.0;
    println!(
        "renderer {renderer} · device scale {scale} · {}×{} logical px",
        canvas.width(),
        canvas.height()
    );
    println!(
        "frames: {} in {:.1} s → {:.1} fps",
        intervals.len(),
        total,
        intervals.len() as f64 / total
    );
    println!(
        "frame interval ms: median {:.1} · p95 {:.1} · p99 {:.1} · max {:.1} · >33 ms: {}",
        pct(intervals, 0.5),
        pct(intervals, 0.95),
        pct(intervals, 0.99),
        pct(intervals, 1.0),
        intervals.iter().filter(|&&i| i > 33.4).count()
    );
    println!(
        "snapshot ms: median {:.2} · p99 {:.2} · max {:.2}",
        pct(&s.stats.snapshot_ms, 0.5),
        pct(&s.stats.snapshot_ms, 0.99),
        pct(&s.stats.snapshot_ms, 1.0)
    );
    println!(
        "tiles rendered: {} · tile ms median {:.1} · p95 {:.1} · frames with placeholders: {} / {}",
        s.stats.tiles_rendered,
        pct(&s.stats.tile_ms, 0.5),
        pct(&s.stats.tile_ms, 0.95),
        s.stats.blurry_frames,
        s.stats.frames
    );
    if let Some(pool) = &s.pool {
        println!("queue at end: {}", pool.queue_len());
    }
}
