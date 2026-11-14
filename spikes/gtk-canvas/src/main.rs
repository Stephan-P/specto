//! Spike: a tiled PDF canvas in GTK4, driven by `pdfrs_view::Viewer`.
//!
//!   gtk-canvas <file.pdf>                 # interactive (keys below)
//!   gtk-canvas <file.pdf> --bench [dir]   # scripted scroll + zoom, prints frame stats,
//!                                         # optionally saves screenshots into dir
//!
//! Keys: 1 single page · 2 continuous · 3 two-page · 4 two-page continuous ·
//! c toggle cover page alone · w fit width · p fit page · 0 actual size ·
//! +/- zoom · PgUp/PgDn/←/→ previous/next page · Home/End first/last page.
//! Scroll; ctrl+scroll or pinch to zoom.
//!
//! Throwaway code; `apps/desktop` replaces it in M1.

mod canvas;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;

use adw::prelude::*;
use gtk::{gdk, glib};
use pdfrs_geom::{Rotation, ViewPoint, ViewVector};
use pdfrs_render::scheduler::Scheduler;
use pdfrs_view::{LayoutMode, Settings, ViewPage, Viewer, ZoomMode};

use canvas::{PdfCanvas, State};

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
    let data = std::fs::read(path).expect("read pdf");
    let (tx, rx) = async_channel::unbounded::<()>();
    let threads = std::env::var("PDFRS_THREADS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(2, |n| n.get()));
    // Workers wake the main loop; the main loop then collects finished tiles.
    let scheduler = Scheduler::new(threads, move || {
        let _ = tx.try_send(());
    });
    let opened = scheduler.open(data, None).expect("open pdf");
    let pages = opened
        .pages
        .iter()
        .enumerate()
        .map(|(i, &geometry)| ViewPage {
            doc: opened.id,
            page: i as u32,
            geometry,
            rotation: Rotation::None,
        })
        .collect();
    let mut viewer = Viewer::new(pages, Settings::default());
    match std::env::var("PDFRS_LAYOUT").as_deref() {
        Ok("single") => viewer.set_layout_mode(LayoutMode::SinglePage),
        Ok("two-page") => viewer.set_layout_mode(LayoutMode::TwoPage),
        Ok("two-page-continuous") => viewer.set_layout_mode(LayoutMode::TwoPageContinuous),
        _ => {}
    }
    let state = Rc::new(RefCell::new(State::new(scheduler, viewer)));
    let canvas = PdfCanvas::new(state.clone());

    // Finished tiles → textures on the main thread.
    {
        let state = state.clone();
        let canvas = canvas.clone();
        glib::spawn_future_local(async move {
            while rx.recv().await.is_ok() {
                let mut s = state.borrow_mut();
                for f in s.scheduler.take_finished() {
                    let Ok(bitmap) = f.result else { continue };
                    let bytes = bitmap.data.len();
                    let texture = gdk::MemoryTexture::new(
                        bitmap.width as i32,
                        bitmap.height as i32,
                        gdk::MemoryFormat::R8g8b8a8Premultiplied,
                        &glib::Bytes::from_owned(bitmap.data),
                        bitmap.width as usize * 4,
                    );
                    s.stats.tiles_rendered += 1;
                    s.stats.tile_ms.push(f.elapsed.as_secs_f64() * 1000.0);
                    s.cache.insert(f.key, texture.upcast(), bytes);
                }
                drop(s);
                canvas.queue_draw();
            }
        });
    }

    let pointer = Rc::new(RefCell::new(ViewPoint::origin()));
    {
        let motion = gtk::EventControllerMotion::new();
        let pointer = pointer.clone();
        motion.connect_motion(move |_, x, y| *pointer.borrow_mut() = ViewPoint::new(x, y));
        canvas.add_controller(motion);
    }

    // Scrolling; ctrl+scroll zooms around the pointer.
    let scroll = gtk::EventControllerScroll::new(
        gtk::EventControllerScrollFlags::BOTH_AXES | gtk::EventControllerScrollFlags::KINETIC,
    );
    {
        let state = state.clone();
        let canvas_ref = canvas.clone();
        let pointer = pointer.clone();
        scroll.connect_scroll(move |ctl, dx, dy| {
            let discrete = ctl.unit() == gdk::ScrollUnit::Wheel;
            let mut s = state.borrow_mut();
            if ctl
                .current_event_state()
                .contains(gdk::ModifierType::CONTROL_MASK)
            {
                let factor = (-dy * if discrete { 0.15 } else { 0.01 }).exp();
                s.viewer.zoom_by(factor, *pointer.borrow());
            } else {
                let step = if discrete { 60.0 } else { 1.0 };
                s.viewer.scroll_by(ViewVector::new(dx * step, dy * step));
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
            state
                .borrow_mut()
                .viewer
                .zoom_by(factor, ViewPoint::new(cx, cy));
            canvas_ref.queue_draw();
        });
    }
    canvas.add_controller(pinch);

    // Keyboard.
    let keys = gtk::EventControllerKey::new();
    {
        let state = state.clone();
        let canvas_ref = canvas.clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            let mut s = state.borrow_mut();
            let v = &mut s.viewer;
            let centre = ViewPoint::new(v.viewport().width / 2.0, v.viewport().height / 2.0);
            match key {
                gdk::Key::_1 => v.set_layout_mode(LayoutMode::SinglePage),
                gdk::Key::_2 => v.set_layout_mode(LayoutMode::Continuous),
                gdk::Key::_3 => v.set_layout_mode(LayoutMode::TwoPage),
                gdk::Key::_4 => v.set_layout_mode(LayoutMode::TwoPageContinuous),
                gdk::Key::c => {
                    let on = !v.settings().cover_alone;
                    v.set_cover_alone(on);
                }
                gdk::Key::w => v.set_zoom_mode(ZoomMode::FitWidth),
                gdk::Key::p => v.set_zoom_mode(ZoomMode::FitPage),
                gdk::Key::_0 => v.set_zoom(1.0, centre),
                gdk::Key::plus | gdk::Key::equal => v.zoom_by(1.25, centre),
                gdk::Key::minus => v.zoom_by(0.8, centre),
                gdk::Key::Page_Down | gdk::Key::Right => v.next_page(),
                gdk::Key::Page_Up | gdk::Key::Left => v.previous_page(),
                gdk::Key::Home => v.go_to_page(0),
                gdk::Key::End => v.go_to_page(v.page_count().saturating_sub(1)),
                gdk::Key::Down => v.scroll_by(ViewVector::new(0.0, 60.0)),
                gdk::Key::Up => v.scroll_by(ViewVector::new(0.0, -60.0)),
                _ => return glib::Propagation::Proceed,
            }
            drop(s);
            canvas_ref.queue_draw();
            glib::Propagation::Stop
        });
    }
    canvas.add_controller(keys);

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
    canvas.grab_focus();

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
            let s = state.borrow();
            eprintln!(
                "long frame: {last:.0} ms at t={t:.2} s (pending {}, zoom {:.2})",
                s.scheduler.pending(),
                s.viewer.zoom()
            );
        }
        let dt = intervals.borrow().last().copied().unwrap_or(16.0) / 1000.0;
        {
            let mut s = state.borrow_mut();
            let v = &mut s.viewer;
            let centre = ViewPoint::new(v.viewport().width / 2.0, v.viewport().height / 2.0);
            let z0 = *base_zoom.borrow_mut().get_or_insert(v.zoom());
            let towards = |v: &Viewer, target: f64, end: f64| {
                (target / v.zoom()).powf((dt / (end - t)).min(1.0))
            };
            match t {
                t if (1.0..11.0).contains(&t) => v.scroll_by(ViewVector::new(0.0, 4000.0 * dt)),
                t if (11.0..14.0).contains(&t) => {
                    let f = towards(v, z0 * 3.0, 14.0);
                    v.zoom_by(f, centre);
                }
                t if (14.0..19.0).contains(&t) => v.scroll_by(ViewVector::new(0.0, 2000.0 * dt)),
                t if (19.0..21.0).contains(&t) => {
                    let f = towards(v, 0.4, 21.0);
                    v.zoom_by(f, centre);
                }
                t if (21.0..24.0).contains(&t) => v.scroll_by(ViewVector::new(0.0, 6000.0 * dt)),
                _ => {}
            }
        }
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
    let renderer = canvas
        .native()
        .and_then(|n| n.renderer())
        .map(|r| r.type_().name().to_string())
        .unwrap_or_default();
    let total: f64 = intervals.iter().sum::<f64>() / 1000.0;
    println!(
        "renderer {renderer} · device scale {} · {}×{} logical px",
        s.viewer.scale_factor(),
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
        "tiles rendered: {} · tile ms median {:.1} · p95 {:.1} · frames not yet sharp: {} / {}",
        s.stats.tiles_rendered,
        pct(&s.stats.tile_ms, 0.5),
        pct(&s.stats.tile_ms, 0.95),
        s.stats.blurry_frames,
        s.stats.frames
    );
    println!(
        "pending at end: {} · cache {} tiles / {} MB",
        s.scheduler.pending(),
        s.cache.len(),
        s.cache.bytes() >> 20
    );
}
