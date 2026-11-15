//! An open document, as both apps hold it: the files its pages come from,
//! the page list as edited, the viewer, the text layer, the selection and
//! the search. No toolkit and no pixels: what a frontend adds is its tile
//! cache and how it gets called back.
//!
//! The pages shown come from an [`EditDoc`], so they can be rotated,
//! deleted, moved and inserted, including pages of other files. Each file
//! is a source; the first ([`MAIN`]) is the one opened, whose outline the
//! document has. Each page keeps its page label from its file. Positions in the viewer are where a page
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

use specto_doc::{Edit, EditDoc, PageRef, Rotation, SourceId, Version};
use specto_geom::{PageGeometry, ViewPoint};
use specto_render::OpenError;
use specto_render::scheduler::Scheduler;
use specto_render::tile::DocId;
use specto_source::{Link, PageLabels, PdfData, SourceDocument, Target};
use specto_text::SearchText;
use specto_view::{Selection, Settings, ViewPage, Viewer};

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
    /// The file's page labels, if it has any that differ from 1, 2, 3, …
    labels: Option<PageLabels>,
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
            labels: doc.page_labels().filter(|l| !l.is_plain()),
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
    /// Page labels, if the document has any that differ from 1, 2, 3, …:
    /// each page keeps the label it has in its file, through edits (see
    /// `PageLabels::of_pages`).
    labels: Option<PageLabels>,
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
        let labels = source.labels.clone();
        let edit = EditDoc::from_source(MAIN, source.geometries.len() as u32);
        let sources = vec![source];
        let pages = view_pages(&edit, &sources);
        Ok(Self {
            sources,
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
    pub fn outline(&self) -> Vec<specto_source::OutlineItem> {
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

/// Pages taken out of a session to put into another one, as when dragging
/// pages from one window to another: the files they come from (the data is
/// shared, not copied) and each page with the rotation the user gave it.
#[derive(Clone, Debug)]
pub struct PageBundle {
    /// Each file and its password.
    files: Vec<(PdfData, Option<String>)>,
    /// Index in `files`, page of that file, rotation.
    pages: Vec<(usize, u32, Rotation)>,
}

impl PageBundle {
    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// The files, each with its password.
    pub fn files(&self) -> &[(PdfData, Option<String>)] {
        &self.files
    }

    /// The pages: index in [`files`](Self::files), page of that file, and
    /// the rotation the user gave it.
    pub fn pages(&self) -> &[(usize, u32, Rotation)] {
        &self.pages
    }
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
    pub fn write(&self) -> Result<Vec<u8>, specto_write::WriteError> {
        specto_write::write(&self.sources(), &self.pages)
    }

    /// The new file's bytes without the document's password protection:
    /// for a print service, which can't be given a password.
    pub fn write_unprotected(&self) -> Result<Vec<u8>, specto_write::WriteError> {
        specto_write::write_unprotected(&self.sources(), &self.pages)
    }

    /// Writes the new file to `path`, replacing it safely if it exists.
    pub fn save(&self, path: &std::path::Path) -> Result<(), specto_write::WriteError> {
        specto_write::save(path, &self.sources(), &self.pages)
    }

    fn sources(&self) -> Vec<specto_write::Source<'_>> {
        self.files
            .iter()
            .map(|(id, data, password)| specto_write::Source {
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

    /// The pages at `which` (in document order), to insert into another
    /// session with [`insert_bundle`](Self::insert_bundle).
    pub fn bundle(&self, which: &[usize]) -> PageBundle {
        let mut which: Vec<usize> = which.to_vec();
        which.sort_unstable();
        which.dedup();
        let mut files: Vec<(PdfData, Option<String>)> = Vec::new();
        let mut file_of: HashMap<SourceId, usize> = HashMap::new();
        let mut pages = Vec::new();
        for p in which.iter().filter_map(|&i| self.edit.pages().get(i)) {
            let file = *file_of.entry(p.source).or_insert_with(|| {
                let s = &self.sources[p.source.0 as usize];
                files.push((s.data.clone(), s.password.clone()));
                files.len() - 1
            });
            pages.push((file, p.page, p.rotation));
        }
        PageBundle { files, pages }
    }

    /// Inserts the pages of `bundle` (from another session) before position
    /// `at`, as one edit, keeping their rotation. Files the session already
    /// has (the same data) are reused, the others opened with `scheduler`.
    /// Returns where the pages ended up, or the error opening a file (then
    /// nothing is inserted).
    pub fn insert_bundle(
        &mut self,
        scheduler: &Scheduler,
        at: usize,
        bundle: &PageBundle,
    ) -> Result<Option<Range<usize>>, OpenError> {
        let mut ids = Vec::with_capacity(bundle.files.len());
        for (data, password) in &bundle.files {
            let known = self
                .sources
                .iter()
                .position(|s| s.data.same(data))
                .map(|i| SourceId(i as u32));
            let id = match known {
                Some(id) => id,
                None => {
                    self.add_source(scheduler, data.clone(), password.as_deref())?
                        .0
                }
            };
            ids.push(id);
        }
        let refs = bundle
            .pages
            .iter()
            .map(|&(file, page, rotation)| PageRef {
                source: ids[file],
                page,
                rotation,
            })
            .collect();
        let range = self.edit.insert(at, refs);
        if range.is_some() {
            self.pages_changed();
        }
        Ok(range)
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
        let sources = &self.sources;
        self.labels = PageLabels::of_pages(self.edit.pages().iter().map(|p| {
            (
                sources[p.source.0 as usize].labels.as_ref(),
                p.page as usize,
            )
        }));
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
