//! A session through its public interface: text, search and links across
//! page edits, pages of a second file, and saving.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use pdfrs_doc::Rotation;
use pdfrs_geom::{PdfPoint, ViewSize};
use pdfrs_render::scheduler::Scheduler;
use pdfrs_session::{Indexed, MAIN, Options, Session};
use pdfrs_source::Target;
use pdfrs_view::{Selection, TextPages};

/// A file of `pages` pages of 200 × 300 pt. Page n (from 1) reads
/// "`word` page n" and, below that, "common text". The first two pages are
/// numbered i and ii, the rest from 1. A link on the first page leads to
/// the third, if there is one.
fn book(pages: usize, word: &str) -> Vec<u8> {
    let mut objects: Vec<(usize, String)> = Vec::new();
    let kids: String = (0..pages).map(|i| format!("{} 0 R ", 10 + 2 * i)).collect();
    objects.push((
        1,
        "<< /Type /Catalog /Pages 2 0 R \
         /PageLabels << /Nums [0 << /S /r >> 2 << /S /D >>] >> >>"
            .into(),
    ));
    objects.push((
        2,
        format!(
            "<< /Type /Pages /Kids [{kids}] /Count {pages} /MediaBox [0 0 200 300] \
             /Resources << /Font << /F1 3 0 R >> >> >>"
        ),
    ));
    objects.push((
        3,
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
    ));
    for i in 0..pages {
        let annots = if i == 0 && pages > 2 {
            " /Annots [5 0 R]"
        } else {
            ""
        };
        objects.push((
            10 + 2 * i,
            format!(
                "<< /Type /Page /Parent 2 0 R /Contents {} 0 R{annots} >>",
                11 + 2 * i
            ),
        ));
        let content = format!(
            "BT /F1 16 Tf 20 250 Td ({word} page {}) Tj 0 -30 Td (common text) Tj ET",
            i + 1
        );
        objects.push((
            11 + 2 * i,
            format!(
                "<< /Length {} >>\nstream\n{content}\nendstream",
                content.len()
            ),
        ));
    }
    if pages > 2 {
        objects.push((
            5,
            "<< /Type /Annot /Subtype /Link /Rect [20 100 180 140] /Dest [14 0 R /Fit] >>".into(),
        ));
    }
    objects.sort_by_key(|(number, _)| *number);

    // With a proper cross-reference table: saving reads the file strictly.
    let size = objects.last().map_or(1, |(number, _)| number + 1);
    let mut out = String::from("%PDF-1.7\n");
    let mut offsets = vec![None; size];
    for (number, body) in &objects {
        offsets[*number] = Some(out.len());
        out += &format!("{number} 0 obj\n{body}\nendobj\n");
    }
    let xref = out.len();
    out += &format!("xref\n0 {size}\n");
    for (number, offset) in offsets.iter().enumerate() {
        out += &match offset {
            Some(offset) => format!("{offset:010} 00000 n \n"),
            None if number == 0 => "0000000000 65535 f \n".into(),
            None => "0000000000 00000 f \n".into(),
        };
    }
    out += &format!("trailer\n<< /Size {size} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n");
    out.into_bytes()
}

/// The Android app keeps its session behind a mutex and calls it from
/// several threads.
#[test]
fn a_session_can_move_between_threads() {
    fn is_send<T: Send>() {}
    is_send::<Session>();
}

fn open(scheduler: &Scheduler, data: Vec<u8>) -> Session {
    let mut session = Session::open(scheduler, data.into(), None, Options::default()).unwrap();
    session
        .viewer
        .set_viewport(ViewSize::new(400.0, 600.0), 1.0);
    session
}

/// The text of every page, in the order the pages are in now.
fn all_text(session: &mut Session) -> String {
    let pages = session.viewer.page_count();
    session.selection = Some(Selection::all(pages));
    let text = session.selected_text().unwrap_or_default();
    session.selection = None;
    text
}

/// Runs the indexer until it has nothing more to hand over, as a frontend
/// would: its pages come over a channel and go into the session here.
/// Returns how many pages it indexed.
fn index(session: &mut Session, from: usize) -> usize {
    let (tx, rx) = mpsc::channel::<Indexed>();
    if !session.start_indexing(from, move |page| tx.send(page).is_ok()) {
        return 0;
    }
    let mut pages = 0;
    // The channel closes when the thread is done.
    while let Ok(page) = rx.recv_timeout(Duration::from_secs(30)) {
        session.indexed(page);
        pages += 1;
    }
    pages
}

#[test]
fn text_follows_the_pages() {
    let scheduler = Scheduler::new(1, || {});
    let mut session = open(&scheduler, book(3, "alpha"));
    let text = all_text(&mut session);
    assert!(text.starts_with("alpha page 1\ncommon text\n"), "{text:?}");
    assert!(text.contains("alpha page 3"));

    // A selection holds positions, so an edit drops it.
    session.selection = Some(Selection::all(3));
    assert_eq!(session.move_pages(&[0], 3), Some(2..3));
    assert!(session.selection.is_none());
    let moved = all_text(&mut session);
    assert!(moved.starts_with("alpha page 2\n"), "{moved:?}");
    assert!(moved.trim_end().ends_with("alpha page 1\ncommon text"));

    // The text layer goes by position too.
    let (_, mut texts, _) = session.text();
    let last = texts.text_page(2).unwrap();
    assert!(last.text().starts_with("alpha page 1"));
    assert!(texts.text_page(3).is_none());
    session.close(&scheduler);
}

#[test]
fn pages_keep_their_labels() {
    let scheduler = Scheduler::new(1, || {});
    let mut session = open(&scheduler, book(4, "alpha"));
    assert!(session.labels().is_some());
    assert_eq!(session.page_label(0), "i");
    assert_eq!(session.page_label(2), "1");
    assert_eq!(session.find_page("ii"), Some(1));
    // "1" is the page labelled 1, not the first page.
    assert_eq!(session.find_page("1"), Some(2));

    // Turned, deleted or moved, pages keep their labels.
    assert!(session.rotate(&[0], Rotation::Cw90));
    assert_eq!(session.page_label(0), "i");
    assert!(session.delete(&[1]));
    let labels: Vec<String> = (0..3).map(|i| session.page_label(i)).collect();
    assert_eq!(labels, ["i", "1", "2"]);
    assert_eq!(session.find_page("1"), Some(1));
    assert_eq!(session.find_page("ii"), None);
    assert_eq!(session.move_pages(&[0], 3), Some(2..3));
    let labels: Vec<String> = (0..3).map(|i| session.page_label(i)).collect();
    assert_eq!(labels, ["1", "2", "i"]);
    // Numbers that aren't labels are positions.
    assert_eq!(session.find_page("3"), Some(2));

    assert!(session.undo().is_some());
    assert!(session.undo().is_some());
    assert_eq!(session.page_label(1), "ii");

    // Back in the file's order, the labels are the file's again; a file
    // whose labels are 1, 2, 3, … has none.
    assert!(session.undo().is_some());
    assert_eq!(session.labels(), session.main().page_labels().as_ref());
    session.close(&scheduler);
}

#[test]
fn links_name_pages_of_their_file() {
    let scheduler = Scheduler::new(1, || {});
    let mut session = open(&scheduler, book(4, "alpha"));
    let on_link = session
        .viewer
        .pdf_to_view(0, PdfPoint::new(100.0, 120.0))
        .unwrap();
    let beside = session
        .viewer
        .pdf_to_view(0, PdfPoint::new(100.0, 200.0))
        .unwrap();
    assert!(session.link_at(beside).is_none());
    let (source, target) = session.link_at(on_link).unwrap();
    assert_eq!(source, MAIN);
    let Target::Destination(destination) = target else {
        panic!("not a destination: {target:?}");
    };
    assert_eq!(destination.page, 2);
    assert_eq!(session.position_of(source, destination.page), Some(2));

    // The page it leads to moves up, and then goes.
    assert!(session.delete(&[1]));
    assert_eq!(session.position_of(MAIN, 2), Some(1));
    assert!(session.delete(&[1]));
    assert_eq!(session.position_of(MAIN, 2), None);
    // The link itself is still on the first page.
    let on_link = session
        .viewer
        .pdf_to_view(0, PdfPoint::new(100.0, 120.0))
        .unwrap();
    assert!(session.link_at(on_link).is_some());
    session.close(&scheduler);
}

#[test]
fn search_follows_indexing_and_edits() {
    let scheduler = Scheduler::new(1, || {});
    let mut session = open(&scheduler, book(4, "alpha"));

    // Nothing is indexed yet, so nothing is found yet.
    assert!(!session.search("ALPHA"));
    let status = session.search_status().unwrap();
    assert_eq!(
        (status.matches, status.pages_indexed, status.pages),
        (0, 0, 4)
    );
    assert_eq!(index(&mut session, 0), 4);
    let status = session.search_status().unwrap();
    assert_eq!((status.matches, status.current), (4, Some(0)));
    assert_eq!(status.pages_indexed, 4);
    // All done: no second run.
    assert_eq!(index(&mut session, 0), 0);

    assert!(session.search_step(true));
    assert_eq!(session.current_match().unwrap().page, 1);
    assert_eq!(session.matches_on(1).len(), 1);
    assert!(session.matches_on(1)[0].1, "the current one");
    assert!(!session.matches_on(0)[0].1);

    // Results hold positions: after an edit the search runs again.
    assert!(session.delete(&[0]));
    let status = session.search_status().unwrap();
    assert_eq!((status.matches, status.pages), (3, 3));
    assert!(session.undo().is_some());
    assert_eq!(session.search_status().unwrap().matches, 4);

    // What is found on a page is found wherever the page goes.
    assert!(session.search("page 4"));
    assert_eq!(session.current_match().unwrap().page, 3);
    session.move_pages(&[3], 0);
    assert_eq!(session.current_match().unwrap().page, 0);

    session.end_search();
    assert!(session.search_status().is_none());
    assert!(session.matches_on(0).is_empty());
    session.close(&scheduler);
}

#[test]
fn pages_of_another_file() {
    let scheduler = Scheduler::new(1, || {});
    let mut session = open(&scheduler, book(2, "alpha"));
    assert_eq!(index(&mut session, 0), 2);
    assert!(session.search("page"));
    assert_eq!(session.search_status().unwrap().matches, 2);

    let (other, count) = session
        .add_source(&scheduler, book(3, "beta").into(), None)
        .unwrap();
    assert_eq!(count, 3);
    // Opened, but not in the document until inserted.
    assert_eq!(session.viewer.page_count(), 2);
    assert_eq!(session.insert(1, &[other]), Some(1..4));
    assert_eq!(session.viewer.page_count(), 5);
    let text = all_text(&mut session);
    let order: Vec<&str> = text.lines().filter(|l| l.contains("page")).collect();
    assert_eq!(
        order,
        [
            "alpha page 1",
            "beta page 1",
            "beta page 2",
            "beta page 3",
            "alpha page 2"
        ]
    );

    // The new pages are searched once they are indexed.
    let status = session.search_status().unwrap();
    assert_eq!(
        (status.matches, status.pages_indexed, status.pages),
        (2, 2, 5)
    );
    assert_eq!(index(&mut session, 0), 3);
    let status = session.search_status().unwrap();
    assert_eq!((status.matches, status.pages_indexed), (5, 5));
    assert!(session.search("beta"));
    assert_eq!(session.current_match().unwrap().page, 1);

    // Its links and labels are its own: the pages keep their labels, and
    // the first file's link still finds its page.
    let labels: Vec<String> = (0..5).map(|i| session.page_label(i)).collect();
    assert_eq!(labels, ["i", "i", "ii", "1", "ii"]);
    assert_eq!(session.position_of(other, 2), Some(3));

    // Undone, the file stays known, for redo.
    assert!(session.undo().is_some());
    assert_eq!(session.viewer.page_count(), 2);
    assert!(session.redo().is_some());
    assert_eq!(session.viewer.page_count(), 5);
    session.close(&scheduler);
}

#[test]
fn pages_from_one_session_into_another() {
    let scheduler = Scheduler::new(1, || {});
    let mut from = open(&scheduler, book(3, "alpha"));
    let (beta, _) = from
        .add_source(&scheduler, book(2, "beta").into(), None)
        .unwrap();
    assert_eq!(from.insert(3, &[beta]), Some(3..5));
    assert!(from.rotate(&[2], Rotation::Cw90));
    // Pages of both files, in document order whatever the order asked.
    let bundle = from.bundle(&[4, 2, 0, 2]);
    assert_eq!(bundle.page_count(), 3);

    let mut to = open(&scheduler, book(2, "gamma"));
    assert_eq!(to.insert_bundle(&scheduler, 1, &bundle), Ok(Some(1..4)));
    let text = all_text(&mut to);
    let order: Vec<&str> = text.lines().filter(|l| l.contains("page")).collect();
    assert_eq!(
        order,
        [
            "gamma page 1",
            "alpha page 1",
            "alpha page 3",
            "beta page 2",
            "gamma page 2"
        ]
    );
    // The rotation comes along; it's one edit.
    assert_eq!(to.edit().pages()[2].rotation, Rotation::Cw90);
    assert_eq!(
        to.edit().undo_edit(),
        Some(pdfrs_doc::Edit::Insert { pages: 3 })
    );

    // The same files again aren't opened a second time.
    let again = from.bundle(&[1]);
    assert_eq!(to.insert_bundle(&scheduler, 0, &again), Ok(Some(0..1)));
    let sources: std::collections::HashSet<_> =
        to.edit().pages().iter().map(|p| p.source).collect();
    assert_eq!(sources.len(), 3);

    // Within one session, a bundle copies pages.
    let own = to.bundle(&[0]);
    assert_eq!(to.insert_bundle(&scheduler, 6, &own), Ok(Some(6..7)));
    assert_eq!(to.edit().pages()[6], to.edit().pages()[0]);

    // Saved, the copied pages are in the file.
    let mut saved = open(&scheduler, to.save_job().write().unwrap());
    assert_eq!(saved.viewer.page_count(), 7);
    assert!(all_text(&mut saved).contains("beta page 2"));
    for s in [&mut from, &mut to, &mut saved] {
        s.close(&scheduler);
    }
}

#[test]
fn saving_writes_the_pages_as_edited() {
    let scheduler = Scheduler::new(1, || {});
    let mut session = open(&scheduler, book(3, "alpha"));
    assert!(!session.edit().is_modified());
    assert!(session.rotate(&[0], Rotation::Cw90));
    assert!(session.delete(&[1]));
    let (other, _) = session
        .add_source(&scheduler, book(1, "beta").into(), None)
        .unwrap();
    assert_eq!(session.insert(2, &[other]), Some(2..3));
    assert!(session.edit().is_modified());

    let job = session.save_job();
    let written = job.write().unwrap();
    // An edit made while the file was being written stays unsaved.
    assert!(session.rotate(&[1], Rotation::Cw90));
    session.mark_saved(job.version);
    assert!(session.edit().is_modified());
    assert!(session.undo().is_some());
    assert!(!session.edit().is_modified());

    let mut saved = open(&scheduler, written);
    assert_eq!(saved.viewer.page_count(), 3);
    let first = saved.viewer.pages()[0].displayed().size();
    assert_eq!((first.width, first.height), (300.0, 200.0));
    let second = saved.viewer.pages()[1].displayed().size();
    assert_eq!((second.width, second.height), (200.0, 300.0));
    let text = all_text(&mut saved);
    let order: Vec<&str> = text.lines().filter(|l| l.contains("page")).collect();
    assert_eq!(order, ["alpha page 1", "alpha page 3", "beta page 1"]);

    // Some of the pages only, e.g. to export a selection.
    let pages = vec![session.edit().pages()[2]];
    let exported = session.save_pages(pages).write().unwrap();
    let mut exported = open(&scheduler, exported);
    assert_eq!(all_text(&mut exported).lines().next(), Some("beta page 1"));

    saved.close(&scheduler);
    exported.close(&scheduler);
    session.close(&scheduler);
}

#[test]
fn the_indexer_stops_when_told() {
    let scheduler = Scheduler::new(1, || {});
    let mut session = open(&scheduler, book(40, "alpha"));

    // A sink that takes one page and no more.
    let (tx, rx) = mpsc::channel::<Indexed>();
    assert!(session.start_indexing(0, move |page| {
        let _ = tx.send(page);
        false
    }));
    let first = rx.recv_timeout(Duration::from_secs(30)).unwrap();
    assert!(
        rx.recv_timeout(Duration::from_secs(30)).is_err(),
        "the thread ended"
    );
    session.search("alpha");
    assert!(session.indexed(first), "the first match is there now");
    assert_eq!(session.search_status().unwrap().pages_indexed, 1);

    // It can be started again for the rest, once the thread has gone.
    let deadline = Instant::now() + Duration::from_secs(30);
    let rx = loop {
        let (tx, rx) = mpsc::channel::<Indexed>();
        if session.start_indexing(5, move |page| tx.send(page).is_ok()) {
            break rx;
        }
        assert!(Instant::now() < deadline, "the first thread never finished");
        std::thread::sleep(Duration::from_millis(5));
    };
    // From the page asked for on, then from the start; not the one done.
    let mut pages = 0;
    let mut first_match = None;
    while let Ok(page) = rx.recv_timeout(Duration::from_secs(30)) {
        session.indexed(page);
        pages += 1;
        if pages == 1 {
            first_match = Some(session.search_status().unwrap().matches);
        }
    }
    assert_eq!(pages, 39);
    assert_eq!(first_match, Some(2));
    assert_eq!(session.search_status().unwrap().matches, 40);

    // Dropping the session stops a running indexer: its sink is let go.
    let (tx, rx) = mpsc::channel::<()>();
    let mut session = open(&scheduler, book(40, "alpha"));
    assert!(session.start_indexing(0, move |_| {
        let _ = &tx;
        std::thread::sleep(Duration::from_millis(20));
        true
    }));
    session.close(&scheduler);
    drop(session);
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(30)),
        Err(mpsc::RecvTimeoutError::Disconnected)
    );
}
