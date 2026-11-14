//! An open document: scheduler, viewer state and tile cache, shared by the
//! canvas and the thumbnail sidebar of one window.
//!
//! The pages shown come from an [`EditDoc`], so they can be rotated,
//! deleted, moved and inserted, including pages of other files. Each file
//! is a [`Source`]; the first is the one opened, whose outline, page labels
//! and name the document has. Positions in the viewer are where a page is
//! now; caches of what doesn't change with editing (text, search text,
//! links) are keyed by the page of the file it comes from.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::ops::Range;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

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

/// The file a document is opened from; its first source.
pub const MAIN: SourceId = SourceId(0);

/// A file the document's pages come from.
struct Source {
    /// Structure (outline, links, metadata), separate from rendering.
    doc: SourceDocument,
    /// The file and its password, for the search indexer and saving.
    data: PdfData,
    password: Option<String>,
    /// The scheduler's name for it.
    doc_id: DocId,
    /// The geometry of each page of the file.
    geometries: Vec<PageGeometry>,
    /// Links per page of the file, parsed on first use.
    links: HashMap<u32, Vec<Link>>,
    /// Every page's text prepared for searching, filled in the background.
    index: Vec<Option<SearchText>>,
}

impl Source {
    fn open(data: PdfData, password: Option<&str>) -> Result<Self, OpenError> {
        let opened = scheduler().open(data.clone(), password)?;
        let doc = match SourceDocument::open(data.clone(), password) {
            Ok(s) => s,
            Err(e) => {
                scheduler().close(opened.id);
                return Err(e);
            }
        };
        Ok(Self {
            doc,
            data,
            password: password.map(str::to_owned),
            doc_id: opened.id,
            index: vec![None; opened.pages.len()],
            geometries: opened.pages,
            links: HashMap::new(),
        })
    }
}

pub struct Document {
    pub name: String,
    /// The files pages come from, by [`SourceId`]: [`MAIN`] first, then
    /// files pages were inserted from. Kept while undo can bring their
    /// pages back.
    sources: Vec<Source>,
    /// Page labels, if the document has any that differ from 1, 2, 3, …
    /// and its pages are still those of the file, in their original order.
    pub labels: Option<PageLabels>,
    /// The file's page labels, whatever the order.
    file_labels: Option<PageLabels>,
    /// The pages as edited.
    pub edit: EditDoc,
    pub viewer: Viewer,
    pub cache: TileCache<gdk::Texture>,
    /// Placeholder levels in use: those tiles double as thumbnails, so the
    /// cache evicts them only after everything else.
    placeholder_levels: Arc<RwLock<BTreeSet<i32>>>,
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
    /// The indexer thread, if one was started and not stopped.
    indexer: Option<Arc<Indexer>>,
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

/// A page of one of the files.
type FilePage = (SourceId, u32);

/// Text pages by page of the file, with the least recently used dropped
/// beyond [`TEXT_PAGES`].
#[derive(Default)]
struct TextCache {
    pages: HashMap<FilePage, (Arc<TextPage>, u64)>,
    clock: u64,
}

impl TextCache {
    fn get(&mut self, page: FilePage) -> Option<Arc<TextPage>> {
        self.clock += 1;
        let (text, used) = self.pages.get_mut(&page)?;
        *used = self.clock;
        Some(text.clone())
    }

    fn insert(&mut self, page: FilePage, text: Arc<TextPage>) {
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
    sources: &'a [Source],
    pages: &'a [PageRef],
    cache: &'a mut TextCache,
}

impl TextPages for TextLayer<'_> {
    /// The text of the page at position `page`.
    fn text_page(&mut self, page: usize) -> Option<Arc<TextPage>> {
        let p = self.pages.get(page)?;
        let key = (p.source, p.page);
        if let Some(t) = self.cache.get(key) {
            return Some(t);
        }
        // A fraction of a millisecond for most pages (M0-FINDINGS.md), so
        // this runs on the main thread when a page is first needed.
        let source = self.sources.get(p.source.0 as usize)?;
        let text = Arc::new(Extractor::new(source.doc.pdf()).page(p.page as usize)?);
        self.cache.insert(key, text.clone());
        Some(text)
    }
}

/// Flags shared with the search indexer thread.
#[derive(Default)]
struct Indexer {
    /// Set to stop it.
    stop: AtomicBool,
    /// Set by the thread when it's done.
    done: AtomicBool,
}

/// What the search indexer sends back: a page of one of the files and its
/// search text.
pub struct Indexed {
    page: FilePage,
    text: SearchText,
}

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
        let source = Source::open(data, password)?;
        let labels = source.doc.page_labels().filter(|l| !l.is_plain());
        let client = scheduler().client(wake);
        let edit = EditDoc::from_source(MAIN, source.geometries.len() as u32);
        let sources = vec![source];
        let pages = view_pages(&edit, &sources);
        let levels: Arc<RwLock<BTreeSet<i32>>> = Arc::default();
        let evict_last = levels.clone();
        let doc = Self {
            name,
            sources,
            file_labels: labels.clone(),
            labels,
            edit,
            viewer: Viewer::new(pages, Settings::default()),
            cache: TileCache::new(CACHE_BUDGET).with_evict_last(move |k| {
                k.scale
                    .level()
                    .is_some_and(|l| evict_last.read().is_ok_and(|s| s.contains(&l)))
            }),
            placeholder_levels: levels,
            client,
            canvas_requests: Vec::new(),
            thumbnails_shown: BTreeSet::new(),
            failed: HashSet::new(),
            texts: TextCache::default(),
            selection: None,
            indexer: None,
            search: None,
        };
        doc.update_placeholder_levels();
        Ok(doc)
    }

    /// Opens another file to insert pages from. Returns its id and page
    /// count; its pages are in the document once inserted.
    pub fn add_source(
        &mut self,
        data: PdfData,
        password: Option<&str>,
    ) -> Result<(SourceId, u32), OpenError> {
        let source = Source::open(data, password)?;
        let id = SourceId(self.sources.len() as u32);
        let count = source.geometries.len() as u32;
        self.sources.push(source);
        // The indexer has a list of pages to do; start over to include
        // the new ones.
        self.stop_indexing();
        Ok((id, count))
    }

    /// The outline of the file opened.
    pub fn outline(&self) -> Vec<pdfrs_source::OutlineItem> {
        self.sources[0].doc.outline()
    }

    /// The viewer, the text layer and the selection, borrowed together, as
    /// the selection functions need them all.
    pub fn text(&mut self) -> (&Viewer, TextLayer<'_>, &mut Option<Selection>) {
        (
            &self.viewer,
            TextLayer {
                sources: &self.sources,
                pages: self.edit.pages(),
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
    /// Destinations name pages of the file the link is in, which is
    /// returned with it.
    pub fn link_at(&mut self, p: ViewPoint) -> Option<(SourceId, Target)> {
        let (page, at) = self.viewer.hit_test(p)?;
        let view_page = self.viewer.pages().get(page)?;
        let pdf = view_page.displayed().page_to_pdf().transform_point(at);
        let file_page = *self.edit.pages().get(page)?;
        let source = self.sources.get_mut(file_page.source.0 as usize)?;
        let doc = &source.doc;
        let links = source
            .links
            .entry(file_page.page)
            .or_insert_with(|| doc.links(file_page.page as usize));
        links
            .iter()
            .rev()
            .find(|l| l.contains(pdf))
            .map(|l| (file_page.source, l.target.clone()))
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

    /// The search text of page `page` of a file, if indexed.
    fn search_text(&self, (source, page): FilePage) -> Option<&SearchText> {
        self.sources
            .get(source.0 as usize)?
            .index
            .get(page as usize)?
            .as_ref()
    }

    /// Starts indexing the document's pages not indexed yet in a background
    /// thread, from position `from` onwards and then from the start, unless
    /// that is already running or done. The receiver gets each page's
    /// search text; pass them to [`indexed`](Self::indexed).
    pub fn start_indexing(&mut self, from: usize) -> Option<async_channel::Receiver<Indexed>> {
        if self
            .indexer
            .as_ref()
            .is_some_and(|i| !i.done.load(Ordering::Relaxed))
        {
            return None;
        }
        let refs = self.edit.pages();
        let from = from.min(refs.len());
        let mut seen = HashSet::new();
        let pages: Vec<FilePage> = refs[from..]
            .iter()
            .chain(&refs[..from])
            .map(|p| (p.source, p.page))
            .filter(|&p| self.search_text(p).is_none() && seen.insert(p))
            .collect();
        if pages.is_empty() {
            return None;
        }
        // Each file the thread needs, to open its own copy of.
        let files: HashMap<SourceId, (PdfData, Option<String>)> = pages
            .iter()
            .map(|&(id, _)| {
                let s = &self.sources[id.0 as usize];
                (id, (s.data.clone(), s.password.clone()))
            })
            .collect();
        let (tx, rx) = async_channel::unbounded();
        let indexer = Arc::new(Indexer::default());
        self.indexer = Some(indexer.clone());
        std::thread::Builder::new()
            .name("search index".into())
            .spawn(move || {
                // Its own copies of the parsed files: nothing shared with
                // the main thread.
                struct Done(Arc<Indexer>);
                impl Drop for Done {
                    fn drop(&mut self) {
                        self.0.done.store(true, Ordering::Relaxed);
                    }
                }
                let done = Done(indexer);
                let stop = &done.0.stop;
                let mut pdfs = HashMap::new();
                for (id, (data, password)) in files {
                    if let Ok(pdf) = pdfrs_source::open_pdf(data, password.as_deref()) {
                        pdfs.insert(id, pdf);
                    }
                }
                for page in pages {
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    let Some(pdf) = pdfs.get(&page.0) else {
                        continue;
                    };
                    let text = Extractor::new(pdf)
                        .page(page.1 as usize)
                        .map(|t| SearchText::new(&t))
                        .unwrap_or_default();
                    if tx.send_blocking(Indexed { page, text }).is_err() {
                        return;
                    }
                }
            })
            .ok()?;
        Some(rx)
    }

    /// Stops the indexer thread, if one is running.
    fn stop_indexing(&mut self) {
        if let Some(i) = self.indexer.take() {
            i.stop.store(true, Ordering::Relaxed);
        }
    }

    /// Takes in a page from the indexer. Returns whether a search had no
    /// current match before and has one now (so the view should go there).
    pub fn indexed(&mut self, Indexed { page, text }: Indexed) -> bool {
        let Some(slot) = self
            .sources
            .get_mut(page.0.0 as usize)
            .and_then(|s| s.index.get_mut(page.1 as usize))
        else {
            return false;
        };
        *slot = Some(text);
        let text = slot.as_ref().expect("just set");
        let Some(s) = self.search.as_mut() else {
            return false;
        };
        let had = s.results.current().is_some();
        let found = text.find(&s.query);
        for (i, p) in self.edit.pages().iter().enumerate() {
            if (p.source, p.page) == page {
                s.results.add_page(i, found.clone());
            }
        }
        if !had {
            s.results.select_from(s.start);
        }
        !had && s.results.current().is_some()
    }

    /// How many of the document's pages are indexed.
    fn pages_indexed(&self) -> usize {
        self.edit
            .pages()
            .iter()
            .filter(|p| self.search_text((p.source, p.page)).is_some())
            .count()
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
        for (i, p) in self.edit.pages().iter().enumerate() {
            if let Some(t) = self.search_text((p.source, p.page)) {
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
            pages_indexed: self.pages_indexed(),
            pages: self.edit.page_count(),
        })
    }
}

/// The viewer's pages for the pages of `edit`.
fn view_pages(edit: &EditDoc, sources: &[Source]) -> Vec<ViewPage> {
    edit.pages()
        .iter()
        .map(|p| {
            let source = &sources[p.source.0 as usize];
            ViewPage {
                doc: source.doc_id,
                page: p.page,
                geometry: source.geometries[p.page as usize],
                rotation: p.rotation,
            }
        })
        .collect()
}

/// What saving needs, taken from the document so the file can be written
/// on another thread.
pub struct SaveJob {
    /// Each source's file and password, the main one first.
    files: Vec<(SourceId, PdfData, Option<String>)>,
    pages: Vec<PageRef>,
    /// The state being saved, to mark as saved when done.
    pub version: Version,
}

impl SaveJob {
    /// The new file's bytes.
    pub fn write(&self) -> Result<Vec<u8>, pdfrs_write::WriteError> {
        pdfrs_write::write(&self.sources(), &self.pages)
    }

    /// Writes the new file to `path`, replacing it safely if it exists.
    pub fn save(&self, path: &std::path::Path) -> Result<(), pdfrs_write::WriteError> {
        pdfrs_write::save(path, &self.sources(), &self.pages)
    }

    fn sources(&self) -> Vec<pdfrs_write::Source<'_>> {
        self.files
            .iter()
            .map(|(id, data, password)| pdfrs_write::Source {
                id: *id,
                data,
                password: password.as_deref(),
            })
            .collect()
    }
}

impl Document {
    // --- editing ----------------------------------------------------------

    /// A copy of what's needed to save the document as it is now.
    pub fn save_job(&self) -> SaveJob {
        self.save_pages(self.edit.pages().to_vec())
    }

    /// A copy of what's needed to save `pages` (the main file's catalog
    /// comes along).
    pub fn save_pages(&self, pages: Vec<PageRef>) -> SaveJob {
        let used: HashSet<SourceId> = pages.iter().map(|p| p.source).collect();
        let files = self
            .sources
            .iter()
            .enumerate()
            .map(|(i, s)| (SourceId(i as u32), s))
            .filter(|(id, _)| *id == MAIN || used.contains(id))
            .map(|(id, s)| (id, s.data.clone(), s.password.clone()))
            .collect();
        SaveJob {
            files,
            pages,
            version: self.edit.version(),
        }
    }

    /// Turns the pages at `which` clockwise by `by`. Returns whether
    /// anything changed.
    pub fn rotate(&mut self, which: &[usize], by: Rotation) -> bool {
        self.edit.rotate(which, by) && self.pages_changed()
    }

    /// Removes the pages at `which` (not all of them). Returns whether
    /// anything changed.
    pub fn delete(&mut self, which: &[usize]) -> bool {
        self.edit.delete(which) && self.pages_changed()
    }

    /// Moves the pages at `which` before position `to`. Returns where they
    /// ended up.
    pub fn move_pages(&mut self, which: &[usize], to: usize) -> Option<Range<usize>> {
        let range = self.edit.move_pages(which, to)?;
        self.pages_changed();
        Some(range)
    }

    /// Inserts pages `pages` of source `source` (from
    /// [`add_source`](Self::add_source)) before position `at`. Returns
    /// where they ended up.
    pub fn insert(
        &mut self,
        at: usize,
        source: SourceId,
        pages: Range<u32>,
    ) -> Option<Range<usize>> {
        let count = self.sources.get(source.0 as usize)?.geometries.len() as u32;
        let refs = pages
            .filter(|&p| p < count)
            .map(|p| PageRef::new(source, p))
            .collect();
        let range = self.edit.insert(at, refs)?;
        self.pages_changed();
        Some(range)
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

    /// The position of page `page` of file `source` (e.g. a link's
    /// target), if it's in the document.
    pub fn position_of(&self, source: SourceId, page: usize) -> Option<usize> {
        self.edit
            .pages()
            .iter()
            .position(|p| p.source == source && p.page as usize == page)
    }

    /// Updates the placeholder levels the tile cache keeps longest.
    fn update_placeholder_levels(&self) {
        let levels = self
            .viewer
            .pages()
            .iter()
            .filter_map(|p| placeholder_level(&p.displayed()))
            .collect();
        if let Ok(mut l) = self.placeholder_levels.write() {
            *l = levels;
        }
    }

    /// Updates everything that depends on the page list. Selections and
    /// search results hold positions, so the selection is dropped and the
    /// search runs again.
    fn pages_changed(&mut self) -> bool {
        let pages = view_pages(&self.edit, &self.sources);
        let main_pages = self.sources[0].geometries.len();
        let in_order = pages.len() == main_pages
            && self
                .edit
                .pages()
                .iter()
                .enumerate()
                .all(|(i, p)| p.source == MAIN && p.page as usize == i);
        self.labels = if in_order {
            self.file_labels.clone()
        } else {
            None
        };
        self.viewer.set_pages(pages);
        self.update_placeholder_levels();
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
        for s in &self.sources {
            scheduler().close(s.doc_id);
        }
        self.stop_indexing();
    }
}
