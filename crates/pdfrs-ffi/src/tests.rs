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

    // The same signal: tests that search wait for either.
    fn search_changed(&self) {
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

#[cfg(unix)]
#[test]
fn opens_from_a_file_descriptor() {
    use std::os::fd::IntoRawFd;
    let path = std::env::temp_dir().join(format!("pdfrs-ffi-fd-{}.pdf", std::process::id()));
    std::fs::write(&path, sample_pdf(3)).unwrap();
    let fd = std::fs::File::open(&path).unwrap().into_raw_fd();
    std::fs::remove_file(&path).unwrap();
    let file = PdfFile::from_fd(fd).unwrap();
    assert!(!file.is_mapped(), "small files are read");
    assert_eq!(file.size(), sample_pdf(3).len() as u64);
    // Parts of it can be read back, clamped to what is there.
    assert_eq!(file.read(0, 8), b"%PDF-1.7");
    assert_eq!(file.read(file.size() - 6, 100), b"%%EOF\n");
    assert_eq!(file.read(file.size() + 5, 100), b"");
    assert_eq!(file.read(1, 0), b"");
    assert_eq!(file.read(0, u32::MAX).len() as u64, file.size());
    let (tx, _rx) = mpsc::channel();
    let listener = Arc::new(Listener(Mutex::new(tx)));
    // The same file serves several attempts (e.g. with passwords).
    for _ in 0..2 {
        let session =
            ViewSession::open_file(file.clone(), None, 1 << 20, listener.clone()).unwrap();
        assert_eq!(session.page_count(), 3);
    }
    assert!(matches!(PdfFile::from_fd(-1), Err(PdfError::Io { .. })));
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
fn paged_mode_swipes_one_fitted_page_at_a_time() {
    let (session, rx) = open(5, 64 << 20);
    // 200 × 300 pt pages in a 216 × 316 dp view: fitted, 1 dp per point.
    session.set_viewport(216.0, 316.0, 2.0);
    session.go_to_page(2);
    session.set_view_mode(ViewMode::Paged);
    let (frame, _) = settle(&session, &rx);
    assert!(frame.paged);
    assert_eq!(
        (frame.current_page, frame.zoom, frame.zoom_factor),
        (2, 1.0, 1.0)
    );
    assert_eq!(frame.pages.len(), 1);
    let rect = frame.pages[0].rect;
    assert_eq!(
        (rect.x, rect.y, rect.width, rect.height),
        (16.0, 16.0, 400.0, 600.0)
    );
    assert_eq!(session.snap_delta(0.0, 0.0), None);

    // Dragging a third of the way and letting go returns to the page; a
    // flick turns it. Through the per-frame path, as the app does.
    let id = session.id();
    assert!(hot::scroll_by(id, 72.0, 0.0));
    assert_eq!(session.frame().pages.len(), 2);
    assert_eq!(
        session.snap_delta(0.0, 0.0),
        Some(Offset { x: -72.0, y: 0.0 })
    );
    let turn = session.snap_delta(500.0, 0.0).unwrap();
    assert_eq!(
        turn,
        Offset {
            x: 216.0 + 8.0 - 72.0,
            y: 0.0
        }
    );
    hot::scroll_by(id, turn.x, turn.y);
    let (frame, _) = settle(&session, &rx);
    assert_eq!((frame.current_page, frame.pages.len()), (3, 1));
    assert_eq!(session.snap_delta(0.0, 0.0), None);

    // Zoomed in, a fling stays on the page, and `fit` returns to the page.
    session.zoom_by(2.0, 108.0, 158.0);
    assert_eq!(session.frame().zoom_factor, 2.0);
    assert!(hot::scroll_in_page_by(id, 10_000.0, 0.0));
    assert_eq!(session.frame().current_page, 3);
    assert_eq!(session.snap_delta(0.0, 0.0), None);
    session.fit();
    let frame = session.frame();
    assert_eq!(
        (frame.current_page, frame.zoom_factor, frame.zoom_mode),
        (3, 1.0, ZoomMode::FitPage)
    );
    assert_eq!(session.snap_delta(0.0, 0.0), None);

    // Back to scrolling: same page, fitted to the width.
    session.set_view_mode(ViewMode::Continuous);
    let frame = session.frame();
    assert!(!frame.paged);
    assert_eq!(
        (frame.current_page, frame.zoom_mode),
        (3, ZoomMode::FitWidth)
    );
}

/// Four 200 × 300 pt pages with an outline (Chapter 1 › Section 1.1,
/// Chapter 2) and, on page 1, a link to page 3 and a link to a web page.
fn linked_pdf() -> Vec<u8> {
    let content = "1 0 0 rg 0 0 m 200 0 l 200 300 l f";
    format!(
        "%PDF-1.7\n\
         1 0 obj << /Type /Catalog /Pages 2 0 R /Outlines 20 0 R >> endobj\n\
         2 0 obj << /Type /Pages /Kids [10 0 R 11 0 R 12 0 R 13 0 R] /Count 4 /MediaBox [0 0 200 300] >> endobj\n\
         4 0 obj << /Length {} >> stream\n{content}\nendstream endobj\n\
         10 0 obj << /Type /Page /Parent 2 0 R /Contents 4 0 R /Annots [30 0 R 31 0 R] >> endobj\n\
         11 0 obj << /Type /Page /Parent 2 0 R /Contents 4 0 R >> endobj\n\
         12 0 obj << /Type /Page /Parent 2 0 R /Contents 4 0 R >> endobj\n\
         13 0 obj << /Type /Page /Parent 2 0 R /Contents 4 0 R >> endobj\n\
         20 0 obj << /Type /Outlines /First 21 0 R /Last 23 0 R /Count 3 >> endobj\n\
         21 0 obj << /Title (Chapter 1) /Parent 20 0 R /Next 23 0 R /First 22 0 R /Last 22 0 R /Count 1 /Dest [10 0 R /XYZ 0 300 0] >> endobj\n\
         22 0 obj << /Title (Section 1.1) /Parent 21 0 R /Dest [11 0 R /Fit] >> endobj\n\
         23 0 obj << /Title (Chapter 2) /Parent 20 0 R /Prev 21 0 R /Dest [13 0 R /XYZ 0 150 0] >> endobj\n\
         30 0 obj << /Type /Annot /Subtype /Link /Rect [20 200 180 240] /Dest [12 0 R /Fit] >> endobj\n\
         31 0 obj << /Type /Annot /Subtype /Link /Rect [20 100 180 140] /A << /S /URI /URI (https://example.com/) >> >> endobj\n\
         trailer << /Root 1 0 R /Size 99 >>\n%%EOF\n",
        content.len(),
    )
    .into_bytes()
}

#[test]
fn outline_and_links() {
    let (tx, _rx) = mpsc::channel();
    let listener = Arc::new(Listener(Mutex::new(tx)));
    let session = ViewSession::open(linked_pdf(), None, 64 << 20, listener).unwrap();
    // Fit width: 1 dp per point, page 1 at (8, 8).
    session.set_viewport(216.0, 316.0, 2.0);

    let outline = session.outline();
    let summary: Vec<(&str, u32, Option<u32>, bool)> = outline
        .iter()
        .map(|e| (e.title.as_str(), e.depth, e.page, e.has_children))
        .collect();
    assert_eq!(
        summary,
        vec![
            ("Chapter 1", 0, Some(0), true),
            ("Section 1.1", 1, Some(1), false),
            ("Chapter 2", 0, Some(3), false),
        ]
    );
    assert_eq!(session.activate_outline(2), Activated::Moved);
    assert_eq!(session.frame().current_page, 3);
    assert_eq!(session.activate_outline(1), Activated::Moved);
    assert_eq!(session.frame().current_page, 1);
    assert_eq!(session.activate_outline(7), Activated::Nothing);

    // The links on page 1: PDF y 200–240 is 60–100 pt from the page's top.
    session.go_to_page(0);
    assert_eq!(session.tap(108.0, 8.0 + 20.0), Activated::Nothing);
    assert_eq!(
        session.tap(108.0, 8.0 + 180.0),
        Activated::Uri {
            uri: "https://example.com/".into()
        }
    );
    assert_eq!(session.frame().current_page, 0);
    assert_eq!(session.tap(108.0, 8.0 + 80.0), Activated::Moved);
    assert_eq!(session.frame().current_page, 2);

    // The same in paged mode, where the page is centred and fitted.
    session.set_view_mode(ViewMode::Paged);
    session.go_to_page(0);
    assert_eq!(session.tap(108.0, 8.0 + 80.0), Activated::Moved);
    let frame = session.frame();
    assert_eq!((frame.current_page, frame.pages.len()), (2, 1));
    assert_eq!(session.snap_delta(0.0, 0.0), None);
    // In the margin beside the page there is nothing to tap.
    assert_eq!(session.tap(2.0, 150.0), Activated::Nothing);
}

#[test]
fn printed_page_numbers() {
    // Front matter i–ii, then 1, 2, …: the page printed "1" is the third.
    let pdf = String::from_utf8(sample_pdf(5)).unwrap().replace(
        "/Type /Catalog",
        "/Type /Catalog /PageLabels << /Nums [0 << /S /r >> 2 << /S /D >>] >>",
    );
    let (tx, _rx) = mpsc::channel();
    let listener = Arc::new(Listener(Mutex::new(tx)));
    let session = ViewSession::open(pdf.into_bytes(), None, 1 << 20, listener).unwrap();
    assert_eq!(
        session.page_labels(),
        Some(["i", "ii", "1", "2", "3"].map(String::from).to_vec())
    );
    // Labels win over positions; case doesn't matter; positions still work
    // where no label says otherwise.
    assert_eq!(session.find_page("ii".into()), Some(1));
    assert_eq!(session.find_page(" II ".into()), Some(1));
    assert_eq!(session.find_page("1".into()), Some(2));
    assert_eq!(session.find_page("5".into()), Some(4));
    assert_eq!(session.find_page("6".into()), None);
    assert_eq!(session.find_page("x".into()), None);

    // Without labels (or with plain ones) there is nothing to show, and
    // text is a position.
    let (plain, _rx) = open(5, 1 << 20);
    assert_eq!(plain.page_labels(), None);
    assert_eq!(plain.find_page("3".into()), Some(2));
    assert_eq!(plain.find_page("0".into()), None);
}

#[test]
fn saved_view_continues_where_the_reader_left_off() {
    let (a, _rx) = open(8, 1 << 20);
    a.set_viewport(216.0, 316.0, 2.0);
    // Fresh documents are continuous and fitted, at the top.
    let fresh = a.saved_view();
    assert_eq!((fresh.page, fresh.paged, fresh.zoom), (0, false, None));
    assert!(!fresh.two_pages && !fresh.cover_alone);

    // Paged, on page 6, zoomed in and scrolled down a little.
    a.set_view_mode(ViewMode::Paged);
    a.go_to_page(5);
    a.zoom_by(2.0, 108.0, 158.0);
    hot::scroll_in_page_by(a.id(), 0.0, 40.0);
    let saved = a.saved_view();
    assert_eq!((saved.page, saved.paged, saved.zoom), (5, true, Some(2.0)));
    assert!(saved.page_offset > 0.0);

    let (b, _rx) = open(8, 1 << 20);
    b.set_viewport(216.0, 316.0, 2.0);
    b.restore_view(saved);
    let frame = b.frame();
    assert!(frame.paged);
    assert_eq!(
        (frame.current_page, frame.zoom, frame.zoom_factor),
        (5, 2.0, 2.0)
    );
    assert_eq!(b.saved_view(), saved);
    assert_eq!(b.snap_delta(0.0, 0.0), None);

    // Continuous, half way down page 3.
    a.set_view_mode(ViewMode::Continuous);
    a.go_to_page(2);
    a.scroll_by(0.0, 150.0);
    let saved = a.saved_view();
    assert_eq!((saved.page, saved.paged, saved.zoom), (2, false, None));
    b.restore_view(saved);
    let frame = b.frame();
    assert_eq!((frame.paged, frame.zoom_mode), (false, ZoomMode::FitWidth));
    assert!((b.saved_view().page_offset - saved.page_offset).abs() < 1e-9);

    // A page that no longer exists is clamped to the last one.
    let (short, _rx) = open(2, 1 << 20);
    short.set_viewport(216.0, 316.0, 2.0);
    short.restore_view(SavedView { page: 7, ..saved });
    assert_eq!(short.frame().current_page, 1);
}

#[test]
fn fit_page_or_fit_width() {
    // Pages of 200×300 pt in a wide view: fitted whole they are 300 high,
    // fitted to the width three times as large.
    let (a, _rx) = open(4, 1 << 20);
    a.set_viewport(616.0, 316.0, 2.0);
    a.set_view_mode(ViewMode::Paged);
    let whole = a.frame();
    assert_eq!((whole.zoom, whole.zoom_factor), (1.0, 1.0));
    assert!(!a.saved_view().fit_width);

    a.set_fit_width(true);
    let wide = a.frame();
    assert_eq!((wide.zoom, wide.zoom_factor), (3.0, 1.0));
    assert_eq!(wide.zoom_mode, ZoomMode::FitWidth);
    // The rest of the page is reached by scrolling within it.
    hot::scroll_in_page_by(a.id(), 0.0, 100.0);
    assert!(a.frame().pages[0].rect.y < -90.0);

    // Turning the page and zooming come back to the chosen fit.
    a.turn_page(true);
    let next = a.frame();
    assert_eq!(
        (next.current_page, next.zoom, next.zoom_factor),
        (1, 3.0, 1.0)
    );
    a.zoom_by(2.0, 308.0, 158.0);
    assert_eq!(a.frame().zoom_factor, 2.0);
    a.fit();
    assert_eq!(a.frame().zoom, 3.0);

    // It is saved with the view, zoomed in or not.
    a.zoom_by(2.0, 308.0, 158.0);
    let saved = a.saved_view();
    assert!(saved.fit_width && saved.zoom == Some(6.0));
    let (b, _rx) = open(4, 1 << 20);
    b.set_viewport(616.0, 316.0, 2.0);
    b.restore_view(saved);
    assert_eq!(b.saved_view(), saved);
    assert_eq!(b.frame().zoom_factor, 2.0);
    b.fit();
    assert_eq!(b.frame().zoom, 3.0);

    // Each view mode starts with its own fit; choosing the same mode again
    // changes nothing.
    a.set_view_mode(ViewMode::Paged);
    assert!(a.saved_view().fit_width);
    a.set_view_mode(ViewMode::Continuous);
    assert!(a.saved_view().fit_width);
    a.set_fit_width(false);
    let whole = a.frame();
    assert_eq!((whole.zoom, whole.zoom_factor), (1.0, 1.0));
    a.set_view_mode(ViewMode::Paged);
    assert!(!a.saved_view().fit_width);
    a.set_view_mode(ViewMode::Continuous);
    assert!(a.saved_view().fit_width);
}

/// Two pages of 200×300 pt with text in Helvetica, 16 pt: "Hello brave
/// world" (from x 20; "brave" from about 61, "world" from about 105) with
/// "second line here" below it, and "Other page".
fn text_pdf() -> Vec<u8> {
    let first = "BT /F1 16 Tf 20 250 Td (Hello brave world) Tj 0 -30 Td (second line here) Tj ET";
    let second = "BT /F1 16 Tf 20 250 Td (Other page) Tj ET";
    format!(
        "%PDF-1.7\n\
         1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n\
         2 0 obj << /Type /Pages /Kids [10 0 R 11 0 R] /Count 2 /MediaBox [0 0 200 300]\
           /Resources << /Font << /F1 3 0 R >> >> >> endobj\n\
         3 0 obj << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> endobj\n\
         4 0 obj << /Length {} >> stream\n{first}\nendstream endobj\n\
         5 0 obj << /Length {} >> stream\n{second}\nendstream endobj\n\
         10 0 obj << /Type /Page /Parent 2 0 R /Contents 4 0 R >> endobj\n\
         11 0 obj << /Type /Page /Parent 2 0 R /Contents 5 0 R >> endobj\n\
         trailer << /Root 1 0 R /Size 99 >>\n%%EOF\n",
        first.len(),
        second.len(),
    )
    .into_bytes()
}

#[test]
fn selecting_text() {
    let (tx, _rx) = mpsc::channel();
    let listener = Arc::new(Listener(Mutex::new(tx)));
    let session = ViewSession::open(text_pdf(), None, 1 << 20, listener).unwrap();
    // Fitted, a page is shown 1:1 with its top-left corner at (8, 8), so
    // PDF (x, y) is at (8 + x, 8 + 300 - y) in the view.
    session.set_viewport(216.0, 316.0, 2.0);
    session.set_view_mode(ViewMode::Paged);
    let at = |x: f64, y: f64| (8.0 + x, 308.0 - y);
    let text = |s: &ViewSession| s.selected_text().unwrap_or_default();
    let frame = session.frame();
    assert!(!frame.selection && frame.highlights.is_empty());
    assert_eq!(session.selected_text(), None);

    // A long press away from any text selects nothing; on a word, the word.
    let (x, y) = at(100.0, 100.0);
    assert!(!session.select_word_at(x, y));
    assert!(!session.frame().selection);
    let (x, y) = at(30.0, 255.0);
    assert!(session.select_word_at(x, y));
    assert_eq!(text(&session), "Hello");

    // It is highlighted where the word is, in device pixels (two per
    // logical one), and both ends are on screen for their handles.
    let frame = session.frame();
    assert!(frame.selection);
    let [h] = frame.highlights[..] else {
        panic!("one line is selected: {:?}", frame.highlights);
    };
    assert!((h.start_bottom.x - 56.0).abs() < 4.0, "{h:?}");
    assert!((h.end_bottom.x - 129.0).abs() < 6.0, "{h:?}");
    assert!(h.start_top.y < h.start_bottom.y && h.start_bottom.y < 2.0 * 70.0);
    let (start, end) = (frame.selection_start.unwrap(), frame.selection_end.unwrap());
    assert_eq!((start.bottom, start.top), (h.start_bottom, h.start_top));
    assert_eq!((end.bottom, end.top), (h.end_bottom, h.end_top));

    // Moving on after the long press selects word by word.
    let (x, y) = at(80.0, 255.0);
    session.select_to(x, y);
    assert_eq!(text(&session), "Hello brave");

    // Taking the end by its handle moves it character by character, here
    // on to the next line.
    session.grab_selection(true);
    let (x, y) = at(72.0, 225.0);
    session.select_to(x, y);
    assert_eq!(text(&session), "Hello brave world\nsecond");
    assert_eq!(session.frame().highlights.len(), 2);

    // The start likewise; the end stays.
    session.grab_selection(false);
    let (x, y) = at(62.0, 255.0);
    session.select_to(x, y);
    assert_eq!(text(&session), "brave world\nsecond");

    // An end dragged onto the other doesn't leave an empty selection.
    session.grab_selection(true);
    let (x, y) = at(62.0, 255.0);
    session.select_to(x, y);
    assert_eq!(text(&session), "brave world\nsecond");

    // The frame's compact form carries the same.
    let mut buf = vec![0u8; 1 << 16];
    let n = hot::frame(session.id(), &mut buf);
    let hot_frame = decode_frame(&buf[..n as usize]);
    let frame = session.frame();
    assert!(hot_frame.selection);
    assert_eq!(hot_frame.highlights, frame.highlights);
    assert_eq!(hot_frame.selection_start, frame.selection_start);
    assert_eq!(hot_frame.selection_end, frame.selection_end);

    // Everything, across pages; an end on a page that isn't shown has no
    // place for a handle.
    session.select_all();
    assert_eq!(
        text(&session),
        "Hello brave world\nsecond line here\nOther page"
    );
    let frame = session.frame();
    assert!(frame.selection && frame.selection_start.is_some());
    if frame.pages.iter().all(|p| p.page != 1) {
        assert_eq!(frame.selection_end, None);
    }

    // The selection stays with the text when the view changes.
    session.zoom_by(2.0, 108.0, 158.0);
    assert_eq!(
        text(&session),
        "Hello brave world\nsecond line here\nOther page"
    );
    assert!(!session.frame().highlights.is_empty());

    session.clear_selection();
    let frame = session.frame();
    assert!(!frame.selection && frame.highlights.is_empty());
    assert_eq!((frame.selection_start, frame.selection_end), (None, None));
    assert_eq!(session.selected_text(), None);
}

#[test]
fn searching() {
    let (tx, rx) = mpsc::channel();
    let listener = Arc::new(Listener(Mutex::new(tx)));
    let session = ViewSession::open(text_pdf(), None, 1 << 20, listener).unwrap();
    session.set_viewport(216.0, 316.0, 2.0);
    session.set_view_mode(ViewMode::Paged);
    assert_eq!(session.search_status(), None);
    let kinds = |s: &ViewSession| -> Vec<HighlightKind> {
        s.frame().highlights.iter().map(|h| h.kind).collect()
    };

    // The first search has the document read in the background; the
    // listener hears when there is more to show. "page" is only on the
    // second page, which the view then goes to.
    session.search("page".into());
    let status = loop {
        let status = session.search_status().unwrap();
        if status.pages_searched == status.pages {
            break status;
        }
        rx.recv_timeout(Duration::from_secs(10))
            .expect("the indexer reports its progress");
    };
    assert_eq!(
        (status.matches, status.current, status.pages),
        (1, Some(0), 2)
    );
    assert_eq!(session.frame().current_page, 1);
    assert_eq!(kinds(&session), [HighlightKind::CurrentMatch]);

    // From now on searches are answered at once. Case and spacing don't
    // matter; the view returns to the first page for this one.
    session.search("HELLO   brave".into());
    let status = session.search_status().unwrap();
    assert_eq!((status.matches, status.current), (1, Some(0)));
    let frame = session.frame();
    assert_eq!(frame.current_page, 0);
    let [h] = frame.highlights[..] else {
        panic!("one match on one line: {:?}", frame.highlights);
    };
    // "Hello brave" runs from x 20 to about 101 pt: 56 to about 218 px.
    assert!((h.start_bottom.x - 56.0).abs() < 4.0, "{h:?}");
    assert!((h.end_bottom.x - 218.0).abs() < 8.0, "{h:?}");

    // Three matches on the first page and one on the second, starting with
    // the first one on the page being looked at.
    session.search("o".into());
    let status = session.search_status().unwrap();
    assert_eq!((status.matches, status.current), (4, Some(0)));
    assert_eq!(
        kinds(&session),
        [
            HighlightKind::CurrentMatch,
            HighlightKind::Match,
            HighlightKind::Match
        ]
    );
    // Stepping goes through them and around, turning pages as needed.
    session.search_step(true);
    assert_eq!(session.search_status().unwrap().current, Some(1));
    assert_eq!(
        kinds(&session),
        [
            HighlightKind::Match,
            HighlightKind::CurrentMatch,
            HighlightKind::Match
        ]
    );
    session.search_step(true);
    session.search_step(true);
    assert_eq!(session.search_status().unwrap().current, Some(3));
    assert_eq!(session.frame().current_page, 1);
    assert_eq!(kinds(&session), [HighlightKind::CurrentMatch]);
    session.search_step(true);
    assert_eq!(session.search_status().unwrap().current, Some(0));
    assert_eq!(session.frame().current_page, 0);
    session.search_step(false);
    assert_eq!(session.search_status().unwrap().current, Some(3));
    assert_eq!(session.frame().current_page, 1);

    // A search started on the second page begins with the match there.
    session.search("o".into());
    assert_eq!(session.search_status().unwrap().current, Some(3));

    // Matches and the selection are highlighted side by side.
    session.go_to_page(0);
    session.select_all();
    let both = kinds(&session);
    assert!(both.contains(&HighlightKind::Selection) && both.contains(&HighlightKind::Match));
    assert_eq!(
        both.first(),
        Some(&HighlightKind::Match),
        "the selection is drawn on top"
    );
    session.clear_selection();

    // Nothing found, and no search at all.
    session.search("zebra".into());
    let status = session.search_status().unwrap();
    assert_eq!((status.matches, status.current), (0, None));
    assert!(session.frame().highlights.is_empty());
    session.search("  ".into());
    assert_eq!(session.search_status(), None);
    session.search("o".into());
    session.end_search();
    assert_eq!(session.search_status(), None);
    assert!(session.frame().highlights.is_empty());
}

#[test]
fn what_a_document_says_about_itself() {
    let (plain, _rx) = open(1, 1 << 20);
    let info = plain.info();
    assert_eq!(info.pdf_version, "1.7");
    assert_eq!((info.title, info.author, info.created), (None, None, None));

    let mut pdf = String::from_utf8(sample_pdf(1)).unwrap();
    pdf = pdf.replace(
        "trailer << /Root 1 0 R",
        "5 0 obj << /Title (Notes) /Creator (ed) /CreationDate (D:20240305142210+01'00') >> endobj\n\
         trailer << /Info 5 0 R /Root 1 0 R",
    );
    let (tx, _rx) = mpsc::channel();
    let listener = Arc::new(Listener(Mutex::new(tx)));
    let session = ViewSession::open(pdf.into_bytes(), None, 1 << 20, listener).unwrap();
    let info = session.info();
    assert_eq!(info.title.as_deref(), Some("Notes"));
    assert_eq!(info.creator.as_deref(), Some("ed"));
    assert_eq!(info.created.as_deref(), Some("2024-03-05T14:22:10+01:00"));
    assert_eq!((info.author, info.modified), (None, None));
}

#[test]
fn two_pages_side_by_side() {
    let (session, rx) = open(5, 64 << 20);
    // Wide enough for two 200 × 300 pt pages at 1 dp per point:
    // 8 + 200 + 8 + 200 + 8 by 8 + 300 + 8.
    session.set_viewport(424.0, 316.0, 2.0);
    session.set_view_mode(ViewMode::Paged);
    session.go_to_page(2);
    session.set_spread(true, false);
    let (frame, _) = settle(&session, &rx);
    assert!(frame.two_pages && !frame.cover_alone && frame.paged);
    // Pages 3 and 4 as a pair, fitted; the reader's page stays on screen.
    assert_eq!(
        (frame.current_page, frame.zoom, frame.zoom_factor),
        (2, 1.0, 1.0)
    );
    let pages: Vec<(u32, f32)> = frame.pages.iter().map(|p| (p.page, p.rect.x)).collect();
    assert_eq!(pages, vec![(2, 16.0), (3, 432.0)]);
    assert_eq!((frame.current_page, frame.last_page), (2, 3));

    // A flick turns to the next pair (here the last page, alone).
    hot::scroll_by(session.id(), 50.0, 0.0);
    let turn = session.snap_delta(500.0, 0.0).unwrap();
    hot::scroll_by(session.id(), turn.x, turn.y);
    let frame = session.frame();
    assert_eq!((frame.current_page, frame.pages.len()), (4, 1));
    assert_eq!(frame.last_page, 4);

    // The first page alone shifts the pairs: [1] [2 3] [4 5].
    session.set_spread(true, true);
    let frame = session.frame();
    assert!(frame.cover_alone);
    assert_eq!(
        frame.pages.iter().map(|p| p.page).collect::<Vec<_>>(),
        vec![3, 4]
    );

    // Scrolling instead of swiping keeps the pairs, fitted to the width.
    session.set_view_mode(ViewMode::Continuous);
    let frame = session.frame();
    assert!(frame.two_pages && !frame.paged);
    assert_eq!(frame.zoom_mode, ZoomMode::FitWidth);

    // Saved and restored with the rest of the view.
    let saved = session.saved_view();
    assert!(saved.two_pages && saved.cover_alone);
    let (other, _rx) = open(5, 1 << 20);
    other.set_viewport(424.0, 316.0, 2.0);
    other.restore_view(saved);
    let frame = other.frame();
    assert!(frame.two_pages && frame.cover_alone && !frame.paged);
    assert_eq!(other.saved_view(), saved);

    // And back to one page at a time.
    session.set_spread(false, false);
    assert!(!session.frame().two_pages);
}

#[test]
fn turning_pages_without_gestures() {
    let (session, _rx) = open(5, 1 << 20);
    session.set_viewport(424.0, 316.0, 2.0);
    let page = |s: &ViewSession| s.frame().current_page;

    // Continuous: page by page, stopping at both ends.
    session.turn_page(false);
    assert_eq!(page(&session), 0);
    session.turn_page(true);
    session.turn_page(true);
    assert_eq!(page(&session), 2);

    // Paged, also while zoomed in (where a swipe would only pan).
    session.set_view_mode(ViewMode::Paged);
    session.zoom_by(3.0, 100.0, 100.0);
    session.turn_page(true);
    assert_eq!(page(&session), 3);
    assert_eq!(session.snap_delta(0.0, 0.0), None);
    session.turn_page(true);
    session.turn_page(true);
    assert_eq!(page(&session), 4);

    // Two pages: pair by pair.
    session.fit();
    session.set_spread(true, false);
    session.go_to_page(0);
    session.turn_page(true);
    assert_eq!(page(&session), 2);
    session.turn_page(false);
    assert_eq!(page(&session), 0);
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
    let last_page = u32_(&mut b);
    u32_(&mut b);
    let zoom = f64::from_ne_bytes(take(&mut b));
    let device_scale = f64::from_ne_bytes(take(&mut b));
    let zoom_factor = f64::from_ne_bytes(take(&mut b));
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
    let point = |b: &mut &[u8]| Point {
        x: f32::from_ne_bytes(take(b)),
        y: f32::from_ne_bytes(take(b)),
    };
    let highlight_count = u32_(&mut b);
    let edges = u32_(&mut b);
    let highlights = (0..highlight_count)
        .map(|_| {
            let kind = [
                HighlightKind::Selection,
                HighlightKind::Match,
                HighlightKind::CurrentMatch,
            ][u32_(&mut b) as usize];
            assert_eq!(u32_(&mut b), 0);
            Highlight {
                kind,
                start_bottom: point(&mut b),
                end_bottom: point(&mut b),
                end_top: point(&mut b),
                start_top: point(&mut b),
            }
        })
        .collect();
    let mut edge = |present: bool| {
        let e = SelectionEdge {
            bottom: point(&mut b),
            top: point(&mut b),
        };
        present.then_some(e)
    };
    let selection_start = edge(edges & 1 != 0);
    let selection_end = edge(edges & 2 != 0);
    assert!(b.is_empty(), "trailing bytes");
    Frame {
        selection: flags & 32 != 0,
        highlights,
        selection_start,
        selection_end,
        thumbnail_first,
        thumbnails,
        pages,
        evicted,
        complete: flags & 1 != 0,
        settled: flags & 2 != 0,
        paged: flags & 4 != 0,
        two_pages: flags & 8 != 0,
        cover_alone: flags & 16 != 0,
        zoom_factor,
        settles_in_ms: u32::try_from(settles).ok(),
        device_scale,
        current_page,
        last_page,
        zoom,
        zoom_mode,
    }
}

/// A frame with something in every field. The Android app's `ViewFrameTest`
/// decodes the encoded form of exactly this.
fn fixture_frame() -> Frame {
    let rect = |x, y, width, height| Rect {
        x,
        y,
        width,
        height,
    };
    Frame {
        pages: vec![
            PageDraw {
                page: 4,
                rect: rect(21.0, -300.0, 1038.0, 1469.0),
                tiles: vec![
                    TileDraw {
                        id: 7,
                        dest: rect(21.0, -300.0, 512.0, 512.0),
                        exact: true,
                    },
                    TileDraw {
                        id: 1 << 40,
                        dest: rect(533.0, -300.0, 463.5, 463.25),
                        exact: false,
                    },
                ],
            },
            PageDraw {
                page: 5,
                rect: rect(21.0, 1190.0, 1038.0, 734.0),
                tiles: vec![],
            },
        ],
        evicted: vec![3, 9_000_000_000],
        complete: false,
        settled: true,
        settles_in_ms: Some(180),
        device_scale: 1.744537353515625,
        current_page: 4,
        last_page: 5,
        zoom: 0.6645856618881226,
        zoom_mode: ZoomMode::FitPage,
        zoom_factor: 1.5,
        paged: true,
        two_pages: true,
        cover_alone: false,
        thumbnail_first: 2,
        thumbnails: vec![0, 11, 12],
        selection: true,
        highlights: vec![
            Highlight {
                kind: HighlightKind::Match,
                start_bottom: Point { x: 400.0, y: 300.0 },
                end_bottom: Point { x: 480.0, y: 300.0 },
                end_top: Point { x: 480.0, y: 270.0 },
                start_top: Point { x: 400.0, y: 270.0 },
            },
            Highlight {
                kind: HighlightKind::CurrentMatch,
                start_bottom: Point { x: 400.0, y: 500.0 },
                end_bottom: Point { x: 480.0, y: 500.0 },
                end_top: Point { x: 480.0, y: 470.0 },
                start_top: Point { x: 400.0, y: 470.0 },
            },
            Highlight {
                kind: HighlightKind::Selection,
                start_bottom: Point { x: 100.0, y: 80.5 },
                end_bottom: Point { x: 900.0, y: 80.5 },
                end_top: Point { x: 900.0, y: 50.25 },
                start_top: Point { x: 100.0, y: 50.25 },
            },
            Highlight {
                kind: HighlightKind::Selection,
                start_bottom: Point { x: 60.0, y: 120.0 },
                end_bottom: Point { x: 300.0, y: 121.0 },
                end_top: Point { x: 300.5, y: 91.0 },
                start_top: Point { x: 60.5, y: 90.0 },
            },
        ],
        // The selection starts above the view.
        selection_start: None,
        selection_end: Some(SelectionEdge {
            bottom: Point { x: 300.0, y: 121.0 },
            top: Point { x: 300.5, y: 91.0 },
        }),
    }
}

/// `fixtures/frame.bin` is what Rust writes for [`fixture_frame`] and what
/// the Kotlin decoder is tested with, so the two can't drift apart unnoticed.
/// After a deliberate change to the layout, update `ViewFrame.decode` and
/// rerun with `PDFRS_UPDATE_FIXTURES=1`.
#[test]
#[cfg(target_endian = "little")]
fn frame_fixture_is_current() {
    let mut bytes = Vec::new();
    hot::encode_frame(&fixture_frame(), &mut bytes);
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/frame.bin");
    if std::env::var_os("PDFRS_UPDATE_FIXTURES").is_some() {
        std::fs::write(&path, &bytes).unwrap();
    }
    let fixture = std::fs::read(&path).unwrap();
    assert!(
        fixture == bytes,
        "the frame layout changed: update ViewFrame.decode in apps/android, \
         then rerun with PDFRS_UPDATE_FIXTURES=1"
    );
    // And it reads back as what was written.
    assert_eq!(
        format!("{:?}", decode_frame(&bytes)),
        format!("{:?}", fixture_frame())
    );
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
