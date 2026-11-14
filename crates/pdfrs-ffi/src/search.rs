//! Searching the document: this session's side of it.
//!
//! Matching itself is shared (`pdfrs_text::SearchText`, and
//! `pdfrs_view::SearchResults` for the list of matches and the current
//! one). What is here is the part around it: every page's search text,
//! prepared once by a background thread from the first search on, and the
//! search in progress. The desktop app has the same in its `document.rs`;
//! the two should become one shared type.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};

use pdfrs_geom::PdfBox;
use pdfrs_source::PdfData;
use pdfrs_text::{Extractor, Query, SearchText};
use pdfrs_view::{SearchResults, TextPages};

use crate::{SearchStatus, Shared, State, TextLayer, ViewListener};

/// The frontend hears about every this many pages indexed, to show how far
/// the search has got (and at once when a page adds matches).
const PROGRESS_PAGES: usize = 16;

pub(crate) struct SearchSession {
    /// The file and its password, for the indexer's own parsed copy.
    data: PdfData,
    password: Option<String>,
    /// Every page's text prepared for searching, filled in the background.
    index: Vec<Option<SearchText>>,
    /// Stops the indexer thread, if one is running.
    indexer: Option<Arc<AtomicBool>>,
    /// The search in progress, if any.
    pub(crate) current: Option<Search>,
}

/// A search: the query and what it has found so far.
pub(crate) struct Search {
    query: Query,
    pub(crate) results: SearchResults,
    /// The page the search started from: the first match from there on is
    /// the first one shown.
    start: usize,
}

impl SearchSession {
    pub(crate) fn new(data: PdfData, password: Option<String>, pages: usize) -> Self {
        Self {
            data,
            password,
            index: vec![None; pages],
            indexer: None,
            current: None,
        }
    }
}

impl Drop for SearchSession {
    fn drop(&mut self) {
        if let Some(stop) = &self.indexer {
            stop.store(true, Ordering::Relaxed);
        }
    }
}

impl State {
    /// Searches for `text` in the pages indexed so far (the others follow
    /// as they are indexed) and shows the first match from the current
    /// page on. Text without anything to search for ends the search.
    pub(crate) fn search(&mut self, text: &str) {
        let Some(query) = Query::new(text) else {
            self.search.current = None;
            return;
        };
        let start = self.viewer.current_page();
        let mut results = SearchResults::new();
        for (page, text) in self.search.index.iter().enumerate() {
            if let Some(text) = text {
                results.add_page(page, text.find(&query));
            }
        }
        results.select_from(start);
        self.search.current = Some(Search {
            query,
            results,
            start,
        });
        self.reveal_match();
    }

    /// Moves to the next (or previous) match, around the ends, and shows it.
    pub(crate) fn search_step(&mut self, forward: bool) {
        let Some(search) = self.search.current.as_mut() else {
            return;
        };
        if forward {
            search.results.select_next();
        } else {
            search.results.select_previous();
        }
        self.reveal_match();
    }

    /// Brings the current match on screen, turning or scrolling to it.
    fn reveal_match(&mut self) {
        let Some((_, found)) = self
            .search
            .current
            .as_ref()
            .and_then(|s| s.results.current())
        else {
            return;
        };
        let (page, chars) = (found.page, found.chars.clone());
        let mut texts = TextLayer {
            source: &self.source,
            cache: &mut self.texts,
        };
        let Some(text) = texts.text_page(page) else {
            return;
        };
        let quads = text.selection_quads(chars);
        let area = PdfBox::from_points(quads.iter().flat_map(|q| q.points));
        self.viewer.reveal(page, area);
    }

    /// Takes in a page from the indexer. Returns whether that changed what
    /// the search has found.
    fn indexed(&mut self, page: usize, text: SearchText) -> bool {
        let Some(slot) = self.search.index.get_mut(page) else {
            return false;
        };
        let mut found = false;
        let mut first = false;
        if let Some(search) = self.search.current.as_mut() {
            let had = search.results.current().is_some();
            let ranges = text.find(&search.query);
            found = !ranges.is_empty();
            search.results.add_page(page, ranges);
            if !had {
                search.results.select_from(search.start);
                first = search.results.current().is_some();
            }
        }
        *slot = Some(text);
        if self.search.index.iter().all(Option::is_some) {
            self.search.indexer = None;
        }
        if first {
            // Nothing was found until now: go to what there is.
            self.reveal_match();
        }
        found
    }

    pub(crate) fn search_status(&self) -> Option<SearchStatus> {
        let search = self.search.current.as_ref()?;
        let index = &self.search.index;
        Some(SearchStatus {
            matches: search.results.len() as u32,
            current: search.results.current().map(|(i, _)| i as u32),
            pages_searched: index.iter().filter(|t| t.is_some()).count() as u32,
            pages: index.len() as u32,
        })
    }
}

/// Starts preparing the pages not indexed yet for searching, in a
/// background thread: from the current page to the end, then from the
/// start. Does nothing if that is already running or done.
pub(crate) fn start_indexing(shared: &Arc<Shared>, state: &mut State) {
    let search = &mut state.search;
    if search.indexer.is_some() || search.index.iter().all(Option::is_some) {
        return;
    }
    let count = search.index.len();
    let from = state.viewer.current_page().min(count);
    let pages: Vec<usize> = (from..count)
        .chain(0..from)
        .filter(|&i| search.index[i].is_none())
        .collect();
    let stop = Arc::new(AtomicBool::new(false));
    let (data, password) = (search.data.clone(), search.password.clone());
    let session = Arc::downgrade(shared);
    let listener = shared.listener.clone();
    let stopped = stop.clone();
    let spawned = std::thread::Builder::new()
        .name("search index".into())
        .spawn(move || {
            index_pages(
                &data,
                password.as_deref(),
                &pages,
                &stopped,
                &session,
                &*listener,
            )
        });
    if spawned.is_ok() {
        search.indexer = Some(stop);
    }
}

/// The indexer thread. It parses the file for itself, so it shares nothing
/// with rendering or the main thread except the results it hands in.
fn index_pages(
    data: &PdfData,
    password: Option<&str>,
    pages: &[usize],
    stop: &AtomicBool,
    session: &Weak<Shared>,
    listener: &dyn ViewListener,
) {
    let Ok(pdf) = pdfrs_source::open_pdf(data.clone(), password) else {
        return;
    };
    let extractor = Extractor::new(&pdf);
    for (done, &page) in pages.iter().enumerate() {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        let text = extractor.page(page).map(|t| SearchText::new(&t));
        // The session may have been closed meanwhile.
        let Some(shared) = session.upgrade() else {
            return;
        };
        let found = shared.lock().indexed(page, text.unwrap_or_default());
        drop(shared);
        if found || (done + 1) % PROGRESS_PAGES == 0 || done + 1 == pages.len() {
            listener.search_changed();
        }
    }
}
