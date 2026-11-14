//! An open document: scheduler, viewer state and tile cache, shared by the
//! canvas and the thumbnail sidebar of one window.
//!
//! The pages shown come from an [`EditDoc`], so they can be rotated (and
//! later deleted, moved and inserted). Positions in the viewer are where a
//! page is now; caches of what doesn't change with editing (text, search
//! text, links) are keyed by the page of the file it comes from.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use gtk::prelude::*;
use gtk::{gdk, glib};
use pdfrs_doc::{Edit, EditDoc, PageRef, Rotation, SourceId, Version};
use pdfrs_geom::{PageGeometry, ViewPoint};
use pdfrs_render::OpenError;
use pdfrs_render::scheduler::{Client, Scheduler};
use pdfrs_render::tile::{DocId, TileCache, TileKey};
use pdfrs_source::{Link, PageLabels, PdfData, SourceDocument, Target};
use pdfrs_text::{Extractor, Query, SearchText, TextPage};
use pdfrs_view::{
    SearchMatch, SearchResults, Selection, Settings, TextPages, ViewPage, Viewer, placeholder_level,
};

const CACHE_BUDGET: usize = 384 << 20;
/// Full text pages kept (for selection and highlights). A page takes about
/// 100 bytes per character, so not a whole long document; search uses the
/// compact `SearchText` instead.
const TEXT_PAGES: usize = 64;

pub type SharedDocument = Rc<RefCell<Document>>;

/// One scheduler (and worker pool) for the whole app; each document gets a
/// client of it, so windows don't each start a pool or cancel each other's
/// requests.
fn scheduler() -> &'static Scheduler {
    static SCHEDULER: OnceLock<Scheduler> = OnceLock::new();
    SCHEDULER.get_or_init(|| {
        let threads = std::thread::available_parallelism().map_or(2, |n| n.get());
        Scheduler::new(threads, || {})
    })
}

pub struct Document {
    pub name: String,
    /// Structure (outline, links, metadata), separate from rendering.
    pub source: SourceDocument,
    /// Links per page of the file, parsed on first use.
    links: HashMap<usize, Vec<Link>>,
    /// Page labels, if the document has any that differ from 1, 2, 3, …
    /// and its pages are still in their original order.
    pub labels: Option<PageLabels>,
    /// The file's page labels, whatever the order.
    file_labels: Option<PageLabels>,
    /// The pages as edited.
    pub edit: EditDoc,
    /// The geometry of each page of the file.
    geometries: Vec<PageGeometry>,
    pub viewer: Viewer,
    pub cache: TileCache<gdk::Texture>,
    doc_id: DocId,
    client: Client,
    /// Requests from the canvas's last frame, highest priority first.
    canvas_requests: Vec<TileKey>,
    /// Pages whose thumbnails are currently on screen in the sidebar.
    pub thumbnails_shown: BTreeSet<usize>,
    /// Tiles that failed to render; never requested again.
    failed: HashSet<TileKey>,
    /// Text layers of recently used pages, extracted on first use.
    texts: TextCache,
    /// The text selection, if any.
    pub selection: Option<Selection>,
    /// The file and its password, for the search indexer's own copy.
    data: PdfData,
    password: Option<String>,
    /// Every page's text prepared for searching (by page of the file),
    /// filled in the background.
    index: Vec<Option<SearchText>>,
    /// Stops the indexer thread, if one is running.
    indexer: Option<Arc<AtomicBool>>,
    /// The current search, if any.
    search: Option<Search>,
}

/// A search: the query and what it found so far.
struct Search {
    query: Query,
    results: SearchResults,
    /// The page the search started from: the first match after it is the
    /// first one shown.
    start: usize,
}

/// Text pages by page of the file, with the least recently used dropped
/// beyond [`TEXT_PAGES`].
#[derive(Default)]
struct TextCache {
    pages: HashMap<usize, (Arc<TextPage>, u64)>,
    clock: u64,
}

impl TextCache {
    fn get(&mut self, page: usize) -> Option<Arc<TextPage>> {
        self.clock += 1;
        let (text, used) = self.pages.get_mut(&page)?;
        *used = self.clock;
        Some(text.clone())
    }

    fn insert(&mut self, page: usize, text: Arc<TextPage>) {
        self.clock += 1;
        self.pages.insert(page, (text, self.clock));
        if self.pages.len() > TEXT_PAGES
            && let Some(&oldest) = self
                .pages
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(p, _)| p)
        {
            self.pages.remove(&oldest);
        }
    }
}

/// The document's text pages, extracted on demand (see [`Document::text`]).
pub struct TextLayer<'a> {
    source: &'a SourceDocument,
    pages: &'a [ViewPage],
    cache: &'a mut TextCache,
}

impl TextPages for TextLayer<'_> {
    /// The text of the page at position `page`.
    fn text_page(&mut self, page: usize) -> Option<Arc<TextPage>> {
        let page = self.pages.get(page)?.page as usize;
        if let Some(t) = self.cache.get(page) {
            return Some(t);
        }
        // A fraction of a millisecond for most pages (M0-FINDINGS.md), so
        // this runs on the main thread when a page is first needed.
        let text = Arc::new(Extractor::new(self.source.pdf()).page(page)?);
        self.cache.insert(page, text.clone());
        Some(text)
    }
}

/// What the search indexer sends back: a page of the file and its search
/// text.
pub type Indexed = (usize, SearchText);

/// How a search stands, for the search bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchStatus {
    pub matches: usize,
    /// Index of the current match.
    pub current: Option<usize>,
    pub pages_indexed: usize,
    pub pages: usize,
}

impl Document {
    /// Parses `data` and starts render workers. `wake` is called from a worker
    /// thread when tiles are ready; it should get the main loop to call
    /// [`receive_tiles`](Self::receive_tiles).
    pub fn open(
        name: String,
        data: PdfData,
        password: Option<&str>,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Result<Self, OpenError> {
        let opened = scheduler().open(data.clone(), password)?;
        let page_count = opened.pages.len();
        let source = match SourceDocument::open(data.clone(), password) {
            Ok(s) => s,
            Err(e) => {
                scheduler().close(opened.id);
                return Err(e);
            }
        };
        let labels = source.page_labels().filter(|l| !l.is_plain());
        let client = scheduler().client(wake);
        let edit = EditDoc::from_source(SOURCE, page_count as u32);
        let pages = view_pages(&edit, opened.id, &opened.pages);
        // Placeholders double as thumbnails: evict them only after everything else.
        let levels: BTreeSet<i32> = pages
            .iter()
            .filter_map(|p| placeholder_level(&p.displayed()))
            .collect();
        Ok(Self {
            name,
            source,
            file_labels: labels.clone(),
            labels,
            edit,
            geometries: opened.pages,
            links: HashMap::new(),
            viewer: Viewer::new(pages, Settings::default()),
            cache: TileCache::new(CACHE_BUDGET)
                .with_evict_last(move |k| k.scale.level().is_some_and(|l| levels.contains(&l))),
            doc_id: opened.id,
            client,
            canvas_requests: Vec::new(),
            thumbnails_shown: BTreeSet::new(),
            failed: HashSet::new(),
            texts: TextCache::default(),
            selection: None,
            data,
            password: password.map(str::to_owned),
            index: vec![None; page_count],
            indexer: None,
            search: None,
        })
    }

    /// The viewer, the text layer and the selection, borrowed together, as
    /// the selection functions need them all.
    pub fn text(&mut self) -> (&Viewer, TextLayer<'_>, &mut Option<Selection>) {
        (
            &self.viewer,
            TextLayer {
                source: &self.source,
                pages: self.viewer.pages(),
                cache: &mut self.texts,
            },
            &mut self.selection,
        )
    }

    /// The selected text, if anything is selected.
    pub fn selected_text(&mut self) -> Option<String> {
        let (_, mut texts, selection) = self.text();
        let text = selection.as_ref()?.text(&mut texts);
        (!text.is_empty()).then_some(text)
    }

    /// Whether page `i` is slow to render (huge images, or a render that
    /// took longer than the scheduler's `slow_after`).
    pub fn is_expensive(&self, i: usize) -> bool {
        self.viewer
            .pages()
            .get(i)
            .is_some_and(|p| scheduler().is_expensive(p.doc, p.page))
    }

    /// The label of page `i` ("iv", "A-3", …), or its number.
    pub fn page_label(&self, i: usize) -> String {
        self.labels
            .as_ref()
            .map_or_else(|| (i + 1).to_string(), |l| l.label(i))
    }

    /// The page the user means by `text`: a label, else a page number.
    pub fn find_page(&self, text: &str) -> Option<usize> {
        let count = self.viewer.page_count();
        self.labels.as_ref().and_then(|l| l.find(text)).or_else(|| {
            text.trim()
                .parse::<usize>()
                .ok()
                .filter(|n| (1..=count).contains(n))
                .map(|n| n - 1)
        })
    }

    /// The target of the link under `p` (viewport coordinates), if any.
    pub fn link_at(&mut self, p: ViewPoint) -> Option<Target> {
        let (page, at) = self.viewer.hit_test(p)?;
        let view_page = self.viewer.pages().get(page)?;
        let pdf = view_page.displayed().page_to_pdf().transform_point(at);
        let source_page = view_page.page as usize;
        let source = &self.source;
        let links = self
            .links
            .entry(source_page)
            .or_insert_with(|| source.links(source_page));
        links
            .iter()
            .rev()
            .find(|l| l.contains(pdf))
            .map(|l| l.target.clone())
    }

    pub fn set_canvas_requests(&mut self, requests: Vec<TileKey>) {
        if requests != self.canvas_requests {
            self.canvas_requests = requests;
            self.flush_requests();
        }
    }

    /// Sends the combined request list: canvas first, then sidebar thumbnails.
    /// (`Scheduler::request` replaces the whole queue, so it must see both.)
    pub fn flush_requests(&self) {
        let thumbnails = self
            .thumbnails_shown
            .iter()
            .filter_map(|&i| self.viewer.placeholder_key(i))
            .filter(|k| !self.cache.contains(k));
        let all = self.canvas_requests.iter().copied().chain(thumbnails);
        self.client
            .request(all.filter(|k| !self.failed.contains(k)));
    }

    /// Moves finished tiles into the cache. Returns whether any arrived.
    pub fn receive_tiles(&mut self) -> bool {
        let finished = self.client.take_finished();
        let any = !finished.is_empty();
        for f in finished {
            match f.result {
                Ok(bitmap) => {
                    let bytes = bitmap.data.len();
                    let texture = gdk::MemoryTexture::new(
                        bitmap.width as i32,
                        bitmap.height as i32,
                        gdk::MemoryFormat::R8g8b8a8Premultiplied,
                        &glib::Bytes::from_owned(bitmap.data),
                        bitmap.width as usize * 4,
                    );
                    self.cache.insert(f.key, texture.upcast(), bytes);
                }
                Err(e) => {
                    glib::g_warning!("pdfrs", "rendering {:?} failed: {e}", f.key);
                    self.failed.insert(f.key);
                }
            }
        }
        any
    }
}

impl Document {
    // --- search ---------------------------------------------------------

    /// Starts indexing the pages not indexed yet in a background thread,
    /// from page `from` onwards and then from the start, unless that is
    /// already running or done. The receiver gets each page's search text;
    /// pass them to [`indexed`](Self::indexed).
    pub fn start_indexing(&mut self, from: usize) -> Option<async_channel::Receiver<Indexed>> {
        if self.indexer.is_some() || self.index.iter().all(Option::is_some) {
            return None;
        }
        let from = self.viewer.pages().get(from).map_or(0, |p| p.page as usize);
        let n = self.index.len();
        let pages: Vec<usize> = (from..n)
            .chain(0..from.min(n))
            .filter(|&i| self.index[i].is_none())
            .collect();
        let (tx, rx) = async_channel::unbounded();
        let stop = Arc::new(AtomicBool::new(false));
        self.indexer = Some(stop.clone());
        let (data, password) = (self.data.clone(), self.password.clone());
        std::thread::Builder::new()
            .name("search index".into())
            .spawn(move || {
                // Its own copy of the parsed file: nothing shared with the
                // main thread.
                let Ok(pdf) = pdfrs_source::open_pdf(data, password.as_deref()) else {
                    return;
                };
                let extractor = Extractor::new(&pdf);
                for i in pages {
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    let text = extractor.page(i).map(|t| SearchText::new(&t));
                    if tx.send_blocking((i, text.unwrap_or_default())).is_err() {
                        return;
                    }
                }
            })
            .ok()?;
        Some(rx)
    }

    /// Takes in a page from the indexer. Returns whether a search had no
    /// current match before and has one now (so the view should go there).
    pub fn indexed(&mut self, (page, text): Indexed) -> bool {
        let Some(slot) = self.index.get_mut(page) else {
            return false;
        };
        let new = match self.search.as_mut() {
            Some(s) => {
                let had = s.results.current().is_some();
                let found = text.find(&s.query);
                for (i, p) in self.viewer.pages().iter().enumerate() {
                    if p.page as usize == page {
                        s.results.add_page(i, found.clone());
                    }
                }
                if !had {
                    s.results.select_from(s.start);
                }
                !had && s.results.current().is_some()
            }
            None => false,
        };
        *slot = Some(text);
        if self.index.iter().all(Option::is_some) {
            self.indexer = None;
        }
        new
    }

    /// Searches for `text` in the pages indexed so far (the rest follow as
    /// they're indexed); an empty `text` ends the search. Returns whether
    /// there is a current match to go to.
    pub fn search(&mut self, text: &str) -> bool {
        let Some(query) = Query::new(text) else {
            self.search = None;
            return false;
        };
        let start = self.viewer.current_page();
        let results = self.find_all(&query, start);
        let found = results.current().is_some();
        self.search = Some(Search {
            query,
            results,
            start,
        });
        found
    }

    /// The matches of `query` on the pages indexed so far, from page
    /// `start` on.
    fn find_all(&self, query: &Query, start: usize) -> SearchResults {
        let mut results = SearchResults::new();
        for (i, p) in self.viewer.pages().iter().enumerate() {
            if let Some(Some(t)) = self.index.get(p.page as usize) {
                results.add_page(i, t.find(query));
            }
        }
        results.select_from(start);
        results
    }

    pub fn end_search(&mut self) {
        self.search = None;
    }

    /// Moves to the next (or previous) match. Returns whether there is one.
    pub fn search_step(&mut self, forward: bool) -> bool {
        let Some(s) = self.search.as_mut() else {
            return false;
        };
        let m = if forward {
            s.results.select_next()
        } else {
            s.results.select_previous()
        };
        m.is_some()
    }

    /// Scrolls the current match into view.
    pub fn reveal_match(&mut self) {
        let Some(m) = self.current_match() else {
            return;
        };
        let area = {
            let (_, mut texts, _) = self.text();
            let Some(text) = texts.text_page(m.page) else {
                return;
            };
            let quads = text.selection_quads(m.chars.clone());
            pdfrs_geom::PdfBox::from_points(quads.iter().flat_map(|q| q.points))
        };
        self.viewer.reveal(m.page, area);
    }

    pub fn current_match(&self) -> Option<SearchMatch> {
        let (_, m) = self.search.as_ref()?.results.current()?;
        Some(m.clone())
    }

    /// The matches on `page`, with whether each is the current one.
    pub fn matches_on(&self, page: usize) -> Vec<(SearchMatch, bool)> {
        self.search.as_ref().map_or_else(Vec::new, |s| {
            s.results
                .on_page(page)
                .map(|(m, current)| (m.clone(), current))
                .collect()
        })
    }

    /// How the search stands, if there is one.
    pub fn search_status(&self) -> Option<SearchStatus> {
        let s = self.search.as_ref()?;
        Some(SearchStatus {
            matches: s.results.len(),
            current: s.results.current().map(|(i, _)| i),
            pages_indexed: self.index.iter().filter(|t| t.is_some()).count(),
            pages: self.index.len(),
        })
    }
}

/// The one file a document is opened from (until inserting pages from other
/// files is supported).
const SOURCE: SourceId = SourceId(0);

/// The viewer's pages for the pages of `edit`.
fn view_pages(edit: &EditDoc, doc: DocId, geometries: &[PageGeometry]) -> Vec<ViewPage> {
    edit.pages()
        .iter()
        .filter_map(|p| {
            Some(ViewPage {
                doc,
                page: p.page,
                geometry: *geometries.get(p.page as usize)?,
                rotation: p.rotation,
            })
        })
        .collect()
}

/// What saving needs, taken from the document so the file can be written
/// on another thread.
pub struct SaveJob {
    data: PdfData,
    password: Option<String>,
    pages: Vec<PageRef>,
    /// The state being saved, to mark as saved when done.
    pub version: Version,
}

impl SaveJob {
    /// The new file's bytes.
    pub fn write(&self) -> Result<Vec<u8>, pdfrs_write::WriteError> {
        pdfrs_write::write(&[self.source()], &self.pages)
    }

    /// Writes the new file to `path`, replacing it safely if it exists.
    pub fn save(&self, path: &std::path::Path) -> Result<(), pdfrs_write::WriteError> {
        pdfrs_write::save(path, &[self.source()], &self.pages)
    }

    fn source(&self) -> pdfrs_write::Source<'_> {
        pdfrs_write::Source {
            id: SOURCE,
            data: &self.data,
            password: self.password.as_deref(),
        }
    }
}

impl Document {
    // --- editing ----------------------------------------------------------

    /// A copy of what's needed to save the document as it is now.
    pub fn save_job(&self) -> SaveJob {
        SaveJob {
            data: self.data.clone(),
            password: self.password.clone(),
            pages: self.edit.pages().to_vec(),
            version: self.edit.version(),
        }
    }

    /// Turns the pages at `which` clockwise by `by`. Returns whether
    /// anything changed.
    pub fn rotate(&mut self, which: &[usize], by: Rotation) -> bool {
        self.edit.rotate(which, by) && self.pages_changed()
    }

    pub fn undo(&mut self) -> Option<Edit> {
        let edit = self.edit.undo()?;
        self.pages_changed();
        Some(edit)
    }

    pub fn redo(&mut self) -> Option<Edit> {
        let edit = self.edit.redo()?;
        self.pages_changed();
        Some(edit)
    }

    /// The position of page `page` of the file (e.g. a link's target), if
    /// it's still in the document.
    pub fn position_of(&self, page: usize) -> Option<usize> {
        self.viewer
            .pages()
            .iter()
            .position(|p| p.page as usize == page)
    }

    /// Updates everything that depends on the page list. Selections and
    /// search results hold positions, so the selection is dropped and the
    /// search runs again.
    fn pages_changed(&mut self) -> bool {
        let pages = view_pages(&self.edit, self.doc_id, &self.geometries);
        let in_order = pages.len() == self.geometries.len()
            && pages.iter().enumerate().all(|(i, p)| p.page as usize == i);
        self.labels = if in_order {
            self.file_labels.clone()
        } else {
            None
        };
        self.viewer.set_pages(pages);
        self.selection = None;
        self.thumbnails_shown.clear();
        if let Some(s) = self.search.take() {
            let start = self.viewer.current_page();
            let results = self.find_all(&s.query, start);
            self.search = Some(Search {
                query: s.query,
                results,
                start,
            });
        }
        true
    }
}

impl Drop for Document {
    fn drop(&mut self) {
        scheduler().close(self.doc_id);
        if let Some(stop) = &self.indexer {
            stop.store(true, Ordering::Relaxed);
        }
    }
}
