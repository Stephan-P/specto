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

/// Plays the frontend: copies finished tiles until the view is sharp.
/// Returns the final frame and all tile ids that were evicted on the way.
fn settle(session: &ViewSession, rx: &mpsc::Receiver<()>) -> (Frame, Vec<u64>) {
    let mut evicted = Vec::new();
    loop {
        let frame = session.frame();
        evicted.extend(&frame.evicted);
        if frame.complete {
            return (frame, evicted);
        }
        rx.recv_timeout(Duration::from_secs(10))
            .expect("tiles should arrive");
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
fn closed_sessions_are_not_found() {
    let (session, _rx) = open(1, 1 << 20);
    let id = session.id();
    assert!(find_session(id).is_some());
    drop(session);
    assert!(find_session(id).is_none());
    assert!(!claim_tile(id, 1, |_| true));
}
