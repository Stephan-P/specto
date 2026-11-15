//! Text selection, shared by the frontends.
//!
//! A selection runs between two [`TextPos`]itions, which can be on different pages, in content
//! order. It grows from an anchor: the point where a drag started, or the word or line that was
//! double- or triple-clicked, in which case dragging extends it by whole words or lines.
//!
//! Text pages come from the frontend through [`TextPages`], which can extract them lazily and cache
//! them. The viewer itself holds no text.

use std::ops::Range;
use std::sync::Arc;

use specto_geom::ViewPoint;
use specto_text::TextPage;

use crate::Viewer;

/// Text pages of the document, extracted on demand.
pub trait TextPages {
    fn text_page(&mut self, page: usize) -> Option<Arc<TextPage>>;
}

/// A position between characters: before character `index` of `page`.
///
/// `index` may exceed the page's character count, meaning its end.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TextPos {
    pub page: usize,
    pub index: usize,
}

impl TextPos {
    pub fn new(page: usize, index: usize) -> Self {
        Self { page, index }
    }
}

/// What a selection grows by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Granularity {
    /// Characters: a drag.
    Char,
    /// Whole words: a double-click, then dragging.
    Word,
    /// Whole lines: a triple-click, then dragging.
    Line,
}

/// A text selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    granularity: Granularity,
    /// What was first selected: an empty range for [`Granularity::Char`], otherwise the word or
    /// line clicked.
    anchor: (TextPos, TextPos),
    start: TextPos,
    end: TextPos,
}

impl Selection {
    /// Starts a selection.
    ///
    /// For [`Granularity::Char`], `at` is a position between characters (from [`text_position`])
    /// and the selection starts empty. Otherwise `at.index` is a character (from [`text_char`]) and
    /// its word or line is selected.
    pub fn new(pages: &mut impl TextPages, at: TextPos, granularity: Granularity) -> Self {
        let anchor = match granularity {
            Granularity::Char => (at, at),
            _ => unit(pages, at, granularity),
        };
        Self {
            granularity,
            anchor,
            start: anchor.0,
            end: anchor.1,
        }
    }

    /// Everything: from the start of the first page to the end of the last.
    pub fn all(page_count: usize) -> Self {
        let start = TextPos::new(0, 0);
        let end = TextPos::new(page_count.saturating_sub(1), usize::MAX);
        Self {
            granularity: Granularity::Char,
            anchor: (start, start),
            start,
            end,
        }
    }

    /// Extends the selection from its anchor to `to`, a position between characters, by the
    /// selection's granularity.
    pub fn extend(&mut self, pages: &mut impl TextPages, to: TextPos) {
        let (a0, a1) = self.anchor;
        (self.start, self.end) = match self.granularity {
            Granularity::Char => (a0.min(to), a0.max(to)),
            g if to < a0 => {
                // Backwards: whole units from the character after `to`.
                (unit(pages, to, g).0, a1)
            }
            g => {
                // Forwards: whole units up to the character before `to`.
                let before = TextPos::new(to.page, to.index.saturating_sub(1));
                let end = if to.index == 0 {
                    to
                } else {
                    unit(pages, before, g).1
                };
                (a0, a1.max(end))
            }
        };
    }

    pub fn granularity(&self) -> Granularity {
        self.granularity
    }

    /// Start and end, in order.
    pub fn range(&self) -> (TextPos, TextPos) {
        (self.start, self.end)
    }

    pub fn is_empty(&self) -> bool {
        self.start >= self.end
    }

    /// The selected characters of `page`, which has `len` characters.
    pub fn page_range(&self, page: usize, len: usize) -> Option<Range<usize>> {
        if self.is_empty() || page < self.start.page || page > self.end.page {
            return None;
        }
        let start = if page == self.start.page {
            self.start.index.min(len)
        } else {
            0
        };
        let end = if page == self.end.page {
            self.end.index.min(len)
        } else {
            len
        };
        (start < end).then_some(start..end)
    }

    /// The selected text, for copying: lines separated by newlines, as are pages.
    pub fn text(&self, pages: &mut impl TextPages) -> String {
        if self.is_empty() {
            return String::new();
        }
        let mut parts = Vec::new();
        for page in self.start.page..=self.end.page {
            let Some(text) = pages.text_page(page) else {
                continue;
            };
            if let Some(r) = self.page_range(page, text.chars.len()) {
                let t = text.text_range(r);
                if !t.is_empty() {
                    parts.push(t);
                }
            }
        }
        parts.join("\n")
    }

    /// The highlight for page `i` on screen: one quadrilateral per selected line, as four corners
    /// in view coordinates.
    pub fn view_quads(
        &self,
        viewer: &Viewer,
        pages: &mut impl TextPages,
        page: usize,
    ) -> Vec<[ViewPoint; 4]> {
        if self.is_empty() || page < self.start.page || page > self.end.page {
            return Vec::new();
        }
        let Some(text) = pages.text_page(page) else {
            return Vec::new();
        };
        let Some(range) = self.page_range(page, text.chars.len()) else {
            return Vec::new();
        };
        text_quads(viewer, &text, page, range)
    }
}

/// The highlight for characters `range` of page `page` on screen: one quadrilateral per line, as
/// four corners in view coordinates. For selections and search matches.
pub fn text_quads(
    viewer: &Viewer,
    text: &TextPage,
    page: usize,
    range: Range<usize>,
) -> Vec<[ViewPoint; 4]> {
    text.selection_quads(range)
        .iter()
        .filter_map(|q| {
            let p = |i: usize| viewer.pdf_to_view(page, q.points[i]);
            Some([p(0)?, p(1)?, p(2)?, p(3)?])
        })
        .collect()
}

/// The word or line around character `at` (clamped to the page).
fn unit(pages: &mut impl TextPages, at: TextPos, g: Granularity) -> (TextPos, TextPos) {
    let Some(text) = pages.text_page(at.page) else {
        return (at, at);
    };
    let len = text.chars.len();
    if len == 0 {
        return (TextPos::new(at.page, 0), TextPos::new(at.page, 0));
    }
    let c = at.index.min(len - 1);
    let r = match g {
        Granularity::Char => c..c + 1,
        Granularity::Word => text.word_range(c),
        Granularity::Line => text.line_range(c),
    };
    (TextPos::new(at.page, r.start), TextPos::new(at.page, r.end))
}

/// The position between characters nearest to `p` on the nearest visible page, for starting or
/// extending a drag.
pub fn text_position(viewer: &Viewer, pages: &mut impl TextPages, p: ViewPoint) -> Option<TextPos> {
    let (page, pdf) = viewer.nearest_pdf_point(p)?;
    let text = pages.text_page(page)?;
    Some(TextPos::new(page, text.position_at(pdf)))
}

/// The character under `p`, or else the one nearest to it, for double- and triple-clicks.
pub fn text_char(viewer: &Viewer, pages: &mut impl TextPages, p: ViewPoint) -> Option<TextPos> {
    let (page, pdf) = viewer.nearest_pdf_point(p)?;
    let text = pages.text_page(page)?;
    let index = text.char_at(pdf).unwrap_or_else(|| {
        text.position_at(pdf)
            .min(text.chars.len().saturating_sub(1))
    });
    Some(TextPos::new(page, index))
}

/// Whether `p` is over a line of text (for an I-beam cursor).
pub fn over_text(viewer: &Viewer, pages: &mut impl TextPages, p: ViewPoint) -> bool {
    let Some((page, _)) = viewer.hit_test(p) else {
        return false;
    };
    let Some((_, pdf)) = viewer.nearest_pdf_point(p) else {
        return false;
    };
    pages
        .text_page(page)
        .is_some_and(|t| t.line_at(pdf).is_some())
}
