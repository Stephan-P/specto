//! Spike: a tiled PDF canvas in GTK4, driven by `pdfrs_view::Viewer`.
//!
//!   gtk-canvas <file.pdf>                 # interactive (keys below)
//!   gtk-canvas <file.pdf> --bench [dir]   # scripted scroll + zoom, prints frame stats,
//!                                         # optionally saves screenshots into dir
//!   gtk-canvas <file.pdf> --probe out.png # HiDPI check: prints the device-pixel mapping and
//!                                         # saves a device-resolution screenshot
//!                                         # (PDFRS_ZOOM=1.0 sets a fixed zoom)
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
        eprintln!("usage: gtk-canvas <file.pdf> [--bench [screenshot-dir] | --probe out.png]");
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
    let levels_only = std::env::var_os("PDFRS_LEVELS_ONLY").is_some();
    let mut viewer = Viewer::new(
        pages,
        Settings {
            // Baseline for comparisons: never settle, so only √2-level tiles.
            settle_delay: if levels_only {
                std::time::Duration::from_secs(1 << 30)
            } else {
                Settings::default().settle_delay
            },
            ..Settings::default()
        },
    );
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
        let s_begin = state.clone();
        pinch.connect_begin(move |_, _| {
            *last_begin.borrow_mut() = 1.0;
            s_begin.borrow_mut().viewer.begin_zoom_gesture();
        });
        let (s_end, c_end) = (state.clone(), canvas_ref.clone());
        pinch.connect_end(move |_, _| {
            s_end.borrow_mut().viewer.end_zoom_gesture();
            c_end.queue_draw();
        });
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
    } else if let Some(i) = std::env::args().position(|a| a == "--probe") {
        let out = std::env::args().nth(i + 1).map(PathBuf::from);
        run_probe(&window, &canvas, state, out.expect("--probe <out.png>"));
    }
}

/// HiDPI diagnostics: waits until the view is sharp, prints how the frame maps
/// onto device pixels, saves a screenshot at device resolution and quits.
/// `PDFRS_ZOOM=<logical px per point>` sets a fixed zoom first.
fn run_probe(
    window: &adw::ApplicationWindow,
    canvas: &PdfCanvas,
    state: Rc<RefCell<State>>,
    out: PathBuf,
) {
    let window = window.clone();
    let start = Instant::now();
    let zoom_set = std::cell::Cell::new(false);
    canvas.add_tick_callback(move |canvas, _| {
        let mut s = state.borrow_mut();
        if !zoom_set.get() && canvas.width() > 0 {
            zoom_set.set(true);
            if let Some(z) = std::env::var("PDFRS_ZOOM").ok().and_then(|z| z.parse().ok()) {
                s.viewer.set_zoom(z, ViewPoint::origin());
            }
        }
        let s = &mut *s;
        let now = Instant::now();
        let frame = s.viewer.frame(now, |k| s.cache.contains(k));
        if start.elapsed().as_secs_f64() < 1.5
            || !(s.viewer.is_settled(now) || std::env::var_os("PDFRS_LEVELS_ONLY").is_some())
            || !frame.complete
            || s.scheduler.pending() > 0
        {
            // Not on the last tick: a queued draw drops the render node that
            // the screenshot below reads.
            canvas.queue_draw();
            return glib::ControlFlow::Continue;
        }

        let native = canvas.native().expect("realized");
        let surface = native.surface().expect("surface");
        let scale = surface.scale();
        let origin = canvas
            .compute_point(&native, &gtk::graphene::Point::new(0.0, 0.0))
            .expect("origin");
        let (sx, sy) = native.surface_transform();
        println!(
            "renderer {} · surface scale {} (integer scale_factor {}) · surface {}×{} logical",
            native.renderer().map(|r| r.type_().name().to_string()).unwrap_or_default(),
            scale,
            surface.scale_factor(),
            surface.width(),
            surface.height(),
        );
        println!(
            "canvas {}×{} logical at ({}, {}) in the window, surface transform ({sx}, {sy}) → device origin ({}, {})",
            canvas.width(),
            canvas.height(),
            origin.x(),
            origin.y(),
            (origin.x() as f64 + sx) * scale,
            (origin.y() as f64 + sy) * scale,
        );
        let device = s.viewer.device_scale();
        let level = pdfrs_render::tile::level_for_scale(device.get());
        let level_scale = pdfrs_render::tile::TileScale::from_level(level);
        println!(
            "zoom {:.6} logical px/pt → device scale {:.6} px/pt (quantized) · level {level} = {:.6} px/pt · settled {} → tiles at {}",
            s.viewer.zoom(),
            device.get(),
            level_scale.get(),
            s.viewer.is_settled(now),
            if device == level_scale {
                "the level (it is exact)".to_string()
            } else {
                format!("the exact scale (a level would be drawn at {:.1}%)", device.get() / level_scale.get() * 100.0)
            },
        );
        for page in &frame.pages {
            println!("page {} rect {:?}", page.page, page.rect);
            for l in &page.layers {
                let scale = match l.key.scale.level() {
                    Some(lv) => format!("level {lv}"),
                    None => format!("exact {:.6}", l.key.scale.get()),
                };
                println!("  {scale} tile ({}, {}) exact {} dest {:?}", l.key.tx, l.key.ty, l.exact, l.dest);
            }
        }

        // Render the widget's own node at device resolution, like the window does.
        let paintable = gtk::WidgetPaintable::new(Some(canvas));
        let snap = gtk::Snapshot::new();
        snap.scale(scale as f32, scale as f32);
        paintable.snapshot(&snap, canvas.width() as f64, canvas.height() as f64);
        if let (Some(node), Some(renderer)) = (snap.to_node(), native.renderer()) {
            let texture = renderer.render_texture(&node, None);
            println!("screenshot {}×{} px → {}", texture.width(), texture.height(), out.display());
            texture.save_to_png(&out).expect("save screenshot");
        }

        // The whole window as GTK puts it on the surface: including the
        // surface transform (CSD shadow margin), which at fractional scales
        // can put the canvas on a fractional device pixel.
        let paintable = gtk::WidgetPaintable::new(Some(&window));
        let snap = gtk::Snapshot::new();
        snap.scale(scale as f32, scale as f32);
        snap.translate(&gtk::graphene::Point::new(sx as f32, sy as f32));
        paintable.snapshot(&snap, window.width() as f64, window.height() as f64);
        if let (Some(node), Some(renderer)) = (snap.to_node(), native.renderer()) {
            let viewport = gtk::graphene::Rect::new(
                0.0,
                0.0,
                (surface.width() as f64 * scale).ceil() as f32,
                (surface.height() as f64 * scale).ceil() as f32,
            );
            let texture = renderer.render_texture(&node, Some(&viewport));
            let path = out.with_file_name(format!(
                "{}-surface.png",
                out.file_stem().unwrap_or_default().to_string_lossy()
            ));
            println!("surface screenshot {}×{} px → {}", texture.width(), texture.height(), path.display());
            texture.save_to_png(&path).expect("save surface screenshot");
        }
        window.close();
        glib::ControlFlow::Break
    });
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
