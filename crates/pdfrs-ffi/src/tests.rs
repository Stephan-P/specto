use std::sync::mpsc;
use std::time::Duration;

use super::*;

/// `pages` pages of 200×300 pt, each with a red triangle.
fn sample_pdf(pages: usize) -> Vec<u8> {
    let content = "1 0 0 rg 0 0 m 200 0 l 200 300 l f";
    let kids: String = (0..pages).map(|i| format!("{} 0 R ", 10 + i)).collect();
    let mut pdf = format!(
        "%PDF-1.7\n\
         1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n\
         2 0 obj << /Type /Pages /Kids [{kids}] /Count {pages} /MediaBox [0 0 200 300] >> endobj\n\
         4 0 obj << /Length {} >> stream\n{content}\nendstream endobj\n",
        content.len(),
    );
    for i in 0..pages {
        pdf += &format!(
            "{} 0 obj << /Type /Page /Parent 2 0 R /Contents 4 0 R >> endobj\n",
            10 + i
        );
    }
    pdf += "trailer << /Root 1 0 R /Size 99 >>\n%%EOF\n";
    pdf.into_bytes()
}

struct Listener(Mutex<mpsc::Sender<()>>);

impl ViewListener for Listener {
    fn tiles_ready(&self) {
        let _ = self.0.lock().unwrap().send(());
    }
}

fn open(pages: usize, cache_bytes: u64) -> (Arc<ViewSession>, mpsc::Receiver<()>) {
    let (tx, rx) = mpsc::channel();
    let listener = Arc::new(Listener(Mutex::new(tx)));
    let session = ViewSession::open(sample_pdf(pages), None, cache_bytes, listener).unwrap();
    (session, rx)
}

/// Plays the frontend: copies finished tiles until the view is settled and
/// sharp. Returns the final frame and all tile ids evicted on the way.
fn settle(session: &ViewSession, rx: &mpsc::Receiver<()>) -> (Frame, Vec<u64>) {
    let mut evicted = Vec::new();
    loop {
        let frame = session.frame();
        evicted.extend(&frame.evicted);
        if frame.complete && frame.settled {
            return (frame, evicted);
        }
        // Wake up for new tiles, or when the view is due to settle.
        let wait = match frame.settles_in_ms {
            Some(ms) => Duration::from_millis(u64::from(ms) + 1),
            None => Duration::from_secs(10),
        };
        if rx.recv_timeout(wait).is_err() {
            assert!(frame.settles_in_ms.is_some(), "tiles should arrive");
        }
        for tile in session.take_finished() {
            let copied = claim_tile(session.id(), tile.id, |b| {
                assert_eq!((b.width, b.height), (tile.width, tile.height));
                true
            });
            assert!(copied);
        }
    }
}

#[test]
fn rejects_garbage() {
    let (tx, _rx) = mpsc::channel();
    let listener = Arc::new(Listener(Mutex::new(tx)));
    assert!(matches!(
        ViewSession::open(b"nope".to_vec(), None, 1 << 20, listener),
        Err(PdfError::Invalid)
    ));
}

#[test]
fn frame_becomes_sharp_and_covers_the_page() {
    let (session, rx) = open(3, 64 << 20);
    assert_eq!(session.page_count(), 3);
    // 200 dp wide at density 2: fit width → the page is 2 × (200 − 2 × 8) px wide.
    session.set_viewport(200.0, 300.0, 2.0);
    let first = session.frame();
    assert!(!first.complete);
    assert_eq!(first.zoom_mode, ZoomMode::FitWidth);

    let (frame, _) = settle(&session, &rx);
    assert_eq!(frame.current_page, 0);
    let page = &frame.pages[0];
    assert_eq!(
        (page.rect.x, page.rect.y, page.rect.width),
        (16.0, 16.0, 368.0)
    );
    assert!(!page.tiles.is_empty());
    // Sharp tiles tile the visible part of the page without gaps: their
    // union spans the page's width.
    let right = page
        .tiles
        .iter()
        .map(|t| t.dest.x + t.dest.width)
        .fold(0.0, f32::max);
    assert!((right - (page.rect.x + page.rect.width)).abs() < 1.0);
}

#[test]
fn scrolling_and_zooming_move_the_view() {
    let (session, rx) = open(5, 64 << 20);
    session.set_viewport(200.0, 300.0, 2.0);
    settle(&session, &rx);

    session.go_to_page(3);
    assert_eq!(session.frame().current_page, 3);
    session.scroll_by(0.0, -100_000.0);
    assert_eq!(session.frame().current_page, 0);

    let before = session.frame().zoom;
    session.zoom_by(2.0, 100.0, 150.0);
    let frame = session.frame();
    assert!((frame.zoom / before - 2.0).abs() < 1e-9);
    assert_eq!(frame.zoom_mode, ZoomMode::Fixed);
    session.set_zoom_mode(ZoomMode::FitWidth);
    assert!((session.frame().zoom - before).abs() < 1e-9);
}

#[test]
fn settles_to_exact_tiles_after_a_zoom() {
    let (session, rx) = open(2, 64 << 20);
    // Density 2.625, as on many phones: fit width lands on no zoom level.
    session.set_viewport(200.0, 300.0, 2.625);
    let (frame, _) = settle(&session, &rx);
    assert_eq!(covered_exactly(&frame.pages[0]), 1.0);

    // During a pinch the view never settles, however long it pauses.
    session.begin_zoom_gesture();
    session.zoom_by(1.3, 100.0, 150.0);
    let zooming = session.frame();
    assert!(!zooming.settled);
    assert_eq!(zooming.settles_in_ms, None);

    // Afterwards it asks to be redrawn, and then sharpens.
    session.end_zoom_gesture();
    let ended = session.frame();
    assert!(!ended.settled);
    assert!(ended.settles_in_ms.is_some());
    let (frame, _) = settle(&session, &rx);
    assert_eq!(covered_exactly(&frame.pages[0]), 1.0);
}

/// Share of the page's on-screen area covered by 1:1 tiles (which sit on
/// whole device pixels). The viewport in these tests is 525 × 787.5 px.
fn covered_exactly(page: &PageDraw) -> f32 {
    let visible = |r: &Rect| {
        let w = (r.x + r.width).min(525.0) - r.x.max(0.0);
        let h = (r.y + r.height).min(787.5) - r.y.max(0.0);
        w.max(0.0) * h.max(0.0)
    };
    let mut exact = 0.0;
    for t in page.tiles.iter().filter(|t| t.exact) {
        assert_eq!((t.dest.x.fract(), t.dest.y.fract()), (0.0, 0.0));
        let clipped = Rect {
            x: t.dest.x.max(page.rect.x),
            y: t.dest.y.max(page.rect.y),
            width: (t.dest.x + t.dest.width).min(page.rect.x + page.rect.width)
                - t.dest.x.max(page.rect.x),
            height: (t.dest.y + t.dest.height).min(page.rect.y + page.rect.height)
                - t.dest.y.max(page.rect.y),
        };
        exact += visible(&clipped);
    }
    exact / visible(&page.rect)
}

#[test]
fn small_budget_evicts_tiles_that_left_the_screen() {
    // Room for roughly one screen of tiles.
    let (session, rx) = open(30, 400 * 600 * 4);
    session.set_viewport(200.0, 300.0, 2.0);
    let mut evicted = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for page in [0, 10, 20, 29] {
        session.go_to_page(page);
        let (frame, gone) = settle(&session, &rx);
        evicted.extend(gone);
        // Nothing the frame draws has been reported as evicted.
        for tile in frame.pages.iter().flat_map(|p| &p.tiles) {
            assert!(!evicted.contains(&tile.id));
            seen.insert(tile.id);
        }
    }
    evicted.extend(session.frame().evicted);
    assert!(
        !evicted.is_empty(),
        "the budget should have forced evictions"
    );

    // Trimming keeps exactly what the last frame used.
    let last = session.frame();
    session.trim_memory();
    let after = session.frame();
    let ids = |f: &Frame| -> Vec<u64> {
        f.pages
            .iter()
            .flat_map(|p| &p.tiles)
            .map(|t| t.id)
            .collect()
    };
    assert_eq!(ids(&last), ids(&after));
    assert!(after.complete);
}

#[test]
fn thumbnails_reuse_placeholders_and_survive_eviction_while_shown() {
    // Room for roughly one screen of tiles.
    let (session, rx) = open(30, 400 * 600 * 4);
    session.set_viewport(200.0, 300.0, 2.0);
    assert_eq!(session.page_sizes().len(), 30);
    assert_eq!(
        session.page_sizes()[0],
        PageSize {
            width: 200.0,
            height: 300.0
        }
    );

    // The grid shows pages 20–25, far from the page view at the top.
    session.set_thumbnail_range(20, 6);
    let first = session.frame();
    assert_eq!((first.thumbnail_first, first.thumbnails.len()), (20, 6));
    assert!(first.thumbnails.iter().all(|&id| id == 0));
    let ready = |session: &ViewSession| loop {
        let (frame, _) = settle(session, &rx);
        if frame.thumbnails.iter().all(|&id| id != 0) {
            return frame.thumbnails;
        }
        rx.recv_timeout(Duration::from_secs(10))
            .expect("thumbnails");
        for tile in session.take_finished() {
            claim_tile(session.id(), tile.id, |_| true);
        }
    };
    let shown = ready(&session);

    // Scrolling the page view through the document churns the small cache,
    // but never evicts a thumbnail that is on screen.
    let mut evicted = Vec::new();
    for page in [5, 12, 28] {
        session.go_to_page(page);
        evicted.extend(settle(&session, &rx).1);
    }
    assert!(!evicted.is_empty());
    assert!(shown.iter().all(|id| !evicted.contains(id)));
    assert_eq!(session.frame().thumbnails, shown);

    // A range past the end is clamped; an empty one reports nothing.
    session.set_thumbnail_range(28, 10);
    assert_eq!(session.frame().thumbnails.len(), 2);
    session.set_thumbnail_range(0, 0);
    assert!(session.frame().thumbnails.is_empty());
}

#[test]
fn closed_sessions_are_not_found() {
    let (session, _rx) = open(1, 1 << 20);
    let id = session.id();
    assert!(find_session(id).is_some());
    drop(session);
    assert!(find_session(id).is_none());
    assert!(!claim_tile(id, 1, |_| true));
}

/// Reads the frame layout documented in `hot.rs`, like the Kotlin side does.
fn decode_frame(mut b: &[u8]) -> Frame {
    fn take<const N: usize>(b: &mut &[u8]) -> [u8; N] {
        let (head, rest) = b.split_at(N);
        *b = rest;
        head.try_into().unwrap()
    }
    let u32_ = |b: &mut &[u8]| u32::from_ne_bytes(take(b));
    let rect = |b: &mut &[u8]| Rect {
        x: f32::from_ne_bytes(take(b)),
        y: f32::from_ne_bytes(take(b)),
        width: f32::from_ne_bytes(take(b)),
        height: f32::from_ne_bytes(take(b)),
    };
    let flags = u32_(&mut b);
    let zoom_mode = [ZoomMode::Fixed, ZoomMode::FitWidth, ZoomMode::FitPage][u32_(&mut b) as usize];
    let current_page = u32_(&mut b);
    let settles = i32::from_ne_bytes(take(&mut b));
    let zoom = f64::from_ne_bytes(take(&mut b));
    let device_scale = f64::from_ne_bytes(take(&mut b));
    let evicted_count = u32_(&mut b);
    let page_count = u32_(&mut b);
    let evicted = (0..evicted_count)
        .map(|_| u64::from_ne_bytes(take(&mut b)))
        .collect();
    let pages = (0..page_count)
        .map(|_| {
            let page = u32_(&mut b);
            let tile_count = u32_(&mut b);
            let page_rect = rect(&mut b);
            let tiles = (0..tile_count)
                .map(|_| {
                    let id = u64::from_ne_bytes(take(&mut b));
                    let dest = rect(&mut b);
                    let exact = u32_(&mut b) != 0;
                    u32_(&mut b);
                    TileDraw { id, dest, exact }
                })
                .collect();
            PageDraw {
                page,
                rect: page_rect,
                tiles,
            }
        })
        .collect();
    let thumbnail_first = u32_(&mut b);
    let thumbnail_count = u32_(&mut b);
    let thumbnails = (0..thumbnail_count)
        .map(|_| u64::from_ne_bytes(take(&mut b)))
        .collect();
    assert!(b.is_empty(), "trailing bytes");
    Frame {
        thumbnail_first,
        thumbnails,
        pages,
        evicted,
        complete: flags & 1 != 0,
        settled: flags & 2 != 0,
        settles_in_ms: u32::try_from(settles).ok(),
        device_scale,
        current_page,
        zoom,
        zoom_mode,
    }
}

#[test]
fn hot_path_matches_the_uniffi_one() {
    let (session, rx) = open(8, 64 << 20);
    session.set_viewport(200.0, 300.0, 2.625);
    settle(&session, &rx);
    let id = session.id();

    assert!(hot::scroll_by(id, 0.0, 450.0));
    assert!(hot::zoom_by(id, 1.25, 100.0, 150.0));
    let mut buf = vec![0u8; 1 << 16];
    let n = hot::frame(id, &mut buf);
    assert!(n > 0);
    let hot_frame = decode_frame(&buf[..n as usize]);
    // Nothing changed in between, so the UniFFI call plans the same frame
    // (its eviction list is empty: the hot call already took it).
    let frame = session.frame();
    assert_eq!(
        format!("{:?}", hot_frame.pages),
        format!("{:?}", frame.pages)
    );
    assert_eq!(hot_frame.current_page, frame.current_page);
    assert_eq!(hot_frame.zoom, frame.zoom);
    assert_eq!(hot_frame.zoom_mode, ZoomMode::Fixed);
    assert_eq!(hot_frame.device_scale, frame.device_scale);
    assert!(!hot_frame.settled && hot_frame.settles_in_ms.is_some());

    // A buffer that is too small reports the size needed; the frame can then
    // be fetched again without planning a new one.
    let mut small = [0u8; 16];
    let needed = hot::frame(id, &mut small);
    assert!(needed < -16);
    let mut exact = vec![0u8; (-needed) as usize];
    assert_eq!(hot::last_frame(id, &mut exact), -needed);
    decode_frame(&exact);

    // Finished tiles: count, then (id, width, height) each.
    let (finished, _) = (hot::take_finished(id, &mut buf), ());
    assert!(finished >= 8);
    let count = u32::from_ne_bytes(buf[..4].try_into().unwrap()) as i64;
    assert_eq!(finished, 8 + count * 16);

    // A closed session is not found.
    drop(session);
    assert!(!hot::scroll_by(id, 0.0, 1.0));
    assert_eq!(hot::frame(id, &mut buf), 0);
}
