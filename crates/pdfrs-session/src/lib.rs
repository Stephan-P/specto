//! An open document, as both apps hold it: the files its pages come from,
//! the page list as edited, the viewer, the text layer, the selection and
//! the search. No toolkit and no pixels: what a frontend adds is its tile
//! cache and how it gets called back.
//!
//! The pages shown come from an [`EditDoc`], so they can be rotated,
//! deleted, moved and inserted, including pages of other files. Each file
//! is a source; the first ([`MAIN`]) is the one opened, whose outline and
//! page labels the document has. Positions in the viewer are where a page
//! is now; caches of what doesn't change with editing (text, search text,
//! links) are keyed by the page of the file it comes from.
//!
//! A session doesn't own the render scheduler: the desktop app shares one
//! between its windows, the Android app has one per document. The calls
//! that open or close files take it as an argument.

mod search;
mod text;

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::Arc;

use pdfrs_doc::{Edit, EditDoc, PageRef, Rotation, SourceId, Version};
use pdfrs_geom::{PageGeometry, ViewPoint};
use pdfrs_render::OpenError;
use pdfrs_render::scheduler::Scheduler;
use pdfrs_render::tile::DocId;
use pdfrs_source::{Link, PageLabels, PdfData, SourceDocument, Target};
use pdfrs_text::SearchText;
use pdfrs_view::{Selection, Settings, ViewPage, Viewer};

pub use search::{Indexed, SearchStatus};
pub use text::TextLayer;

use search::{Indexer, Search};
use text::TextCache;

/// The file a document is opened from; its first source.
pub const MAIN: SourceId = SourceId(0);

/// A page of one of the files.
type FilePage = (SourceId, u32);

/// What a frontend chooses when opening a document.
#[derive(Clone, Debug)]
pub struct Options {
    /// The viewer's settings: gaps, margins and the like.
    pub settings: Settings,
    /// Full text pages kept (for selection and highlights), the least
    /// recently used going first. A page takes about 100 bytes per
    /// character, so not a whole long document; search uses the compact
    /// `SearchText` instead.
    pub text_pages: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            settings: Settings::default(),
            text_pages: 64,
        }
    }
}

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
    fn open(
        scheduler: &Scheduler,
        data: PdfData,
        password: Option<&str>,
    ) -> Result<Self, OpenError> {
        let opened = scheduler.open(data.clone(), password)?;
        let doc = match SourceDocument::open(data.clone(), password) {
            Ok(s) => s,
            Err(e) => {
                scheduler.close(opened.id);
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

pub struct Session {
    /// The files pages come from, by [`SourceId`]: [`MAIN`] first, then
    /// files pages were inserted from. Kept while undo can bring their
    /// pages back.
    sources: Vec<Source>,
    /// Page labels, if the document has any that differ from 1, 2, 3, …
    /// and its pages are still those of the file, in their original order.
    labels: Option<PageLabels>,
    /// The file's page labels, whatever the order.
    file_labels: Option<PageLabels>,
    /// The pages as edited.
    edit: EditDoc,
    pub viewer: Viewer,
    /// Text layers of recently used pages, extracted on first use.
    texts: TextCache,
    /// The text selection, if any.
    pub selection: Option<Selection>,
    /// The indexer thread, if one was started and not stopped.
    indexer: Option<Arc<Indexer>>,
    /// The current search, if any.
    search: Option<Search>,
}

impl Session {
    /// Parses `data` and registers it with `scheduler` for rendering.
    /// Before the session is dropped, [`close`](Self::close) should take
    /// its files out of the scheduler again.
    pub fn open(
        scheduler: &Scheduler,
        data: PdfData,
        password: Option<&str>,
        options: Options,
    ) -> Result<Self, OpenError> {
        let source = Source::open(scheduler, data, password)?;
        let labels = source.doc.page_labels().filter(|l| !l.is_plain());
        let edit = EditDoc::from_source(MAIN, source.geometries.len() as u32);
        let sources = vec![source];
        let pages = view_pages(&edit, &sources);
        Ok(Self {
            sources,
            file_labels: labels.clone(),
            labels,
            edit,
            viewer: Viewer::new(pages, options.settings),
            texts: TextCache::new(options.text_pages),
            selection: None,
            indexer: None,
            search: None,
        })
    }

    /// Opens another file to insert pages from. Returns its id and page
    /// count; its pages are in the document once inserted.
    pub fn add_source(
        &mut self,
        scheduler: &Scheduler,
        data: PdfData,
        password: Option<&str>,
    ) -> Result<(SourceId, u32), OpenError> {
        let source = Source::open(scheduler, data, password)?;
        let id = SourceId(self.sources.len() as u32);
        let count = source.geometries.len() as u32;
        self.sources.push(source);
        // The indexer has a list of pages to do; start over to include
        // the new ones.
        self.stop_indexing();
        Ok((id, count))
    }

    /// Takes the session's files out of `scheduler`, the one it was opened
    /// with, and stops the search indexer. Nothing of the session renders
    /// afterwards.
    pub fn close(&mut self, scheduler: &Scheduler) {
        for s in &self.sources {
            scheduler.close(s.doc_id);
        }
        self.stop_indexing();
    }

    /// The file opened, parsed: its metadata, outline and so on.
    pub fn main(&self) -> &SourceDocument {
        &self.sources[0].doc
    }

    /// The outline of the file opened.
    pub fn outline(&self) -> Vec<pdfrs_source::OutlineItem> {
        self.main().outline()
    }

    /// The pages as edited. They change through the session only, which
    /// keeps the viewer and everything else in step.
    pub fn edit(&self) -> &EditDoc {
        &self.edit
    }

    /// Records that the state at `version` (see [`SaveJob::version`]) is
    /// what is on disk now.
    pub fn mark_saved(&mut self, version: Version) {
        self.edit.mark_saved_version(version);
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

    /// The document's own page numbering ("iv", "A-3", …), if it has one
    /// that differs from 1, 2, 3, … and its pages are still those of the
    /// file, in their original order.
    pub fn labels(&self) -> Option<&PageLabels> {
        self.labels.as_ref()
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
        // The topmost link is the last one drawn.
        links
            .iter()
            .rev()
            .find(|l| l.contains(pdf))
            .map(|l| (file_page.source, l.target.clone()))
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

/// What saving needs, taken from the session so the file can be written
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

impl Session {
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

    /// Inserts all pages of each of `sources` (from
    /// [`add_source`](Self::add_source)), in order, before position `at`,
    /// as one edit. Returns where they ended up.
    pub fn insert(&mut self, at: usize, sources: &[SourceId]) -> Option<Range<usize>> {
        let mut refs = Vec::new();
        for &source in sources {
            let count = self.sources.get(source.0 as usize)?.geometries.len() as u32;
            refs.extend((0..count).map(|p| PageRef::new(source, p)));
        }
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
        self.selection = None;
        self.search_again();
        true
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop_indexing();
    }
}
