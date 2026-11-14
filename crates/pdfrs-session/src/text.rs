//! The text of the document's pages, extracted when first needed.

use std::collections::HashMap;
use std::sync::Arc;

use pdfrs_doc::PageRef;
use pdfrs_text::{Extractor, TextPage};
use pdfrs_view::TextPages;

use crate::{FilePage, Source};

/// Text pages by page of the file, with the least recently used dropped
/// beyond `limit`.
pub(crate) struct TextCache {
    pages: HashMap<FilePage, (Arc<TextPage>, u64)>,
    clock: u64,
    limit: usize,
}

impl TextCache {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            pages: HashMap::new(),
            clock: 0,
            limit: limit.max(1),
        }
    }

    fn get(&mut self, page: FilePage) -> Option<Arc<TextPage>> {
        self.clock += 1;
        let (text, used) = self.pages.get_mut(&page)?;
        *used = self.clock;
        Some(text.clone())
    }

    fn insert(&mut self, page: FilePage, text: Arc<TextPage>) {
        self.clock += 1;
        self.pages.insert(page, (text, self.clock));
        if self.pages.len() > self.limit
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

/// The document's text pages, extracted on demand (see
/// [`Session::text`](crate::Session::text)).
pub struct TextLayer<'a> {
    pub(crate) sources: &'a [Source],
    pub(crate) pages: &'a [PageRef],
    pub(crate) cache: &'a mut TextCache,
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
        // this runs on the caller's thread when a page is first needed.
        let source = self.sources.get(p.source.0 as usize)?;
        let text = Arc::new(Extractor::new(source.doc.pdf()).page(p.page as usize)?);
        self.cache.insert(key, text.clone());
        Some(text)
    }
}
