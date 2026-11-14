//! Search results, shared by the frontends.
//!
//! The frontend matches pages with [`pdfrs_text::SearchText`] (usually in
//! the background, page by page) and adds them here. [`SearchResults`]
//! keeps the matches in document order and which one is current, and moves
//! through them, wrapping around at the ends.

use std::ops::Range;

/// One match: characters of a page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchMatch {
    pub page: usize,
    pub chars: Range<usize>,
}

/// The matches found so far, and the current one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchResults {
    /// In document order.
    matches: Vec<SearchMatch>,
    current: Option<usize>,
}

impl SearchResults {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds the matches `ranges` (in order) of `page`, which hasn't been
    /// added before. The current match stays the same.
    pub fn add_page(&mut self, page: usize, ranges: Vec<Range<usize>>) {
        if ranges.is_empty() {
            return;
        }
        let at = self.matches.partition_point(|m| m.page < page);
        let n = ranges.len();
        self.matches.splice(
            at..at,
            ranges.into_iter().map(|chars| SearchMatch { page, chars }),
        );
        if let Some(c) = self.current.as_mut()
            && *c >= at
        {
            *c += n;
        }
    }

    pub fn len(&self) -> usize {
        self.matches.len()
    }

    pub fn is_empty(&self) -> bool {
        self.matches.is_empty()
    }

    /// The current match and its index (for "3 of 17").
    pub fn current(&self) -> Option<(usize, &SearchMatch)> {
        let i = self.current?;
        Some((i, &self.matches[i]))
    }

    /// Makes the first match on page `page` or later current, or else the
    /// first match (searching wraps around). Returns the new current match.
    pub fn select_from(&mut self, page: usize) -> Option<&SearchMatch> {
        if self.matches.is_empty() {
            return None;
        }
        let i = self.matches.partition_point(|m| m.page < page);
        self.current = Some(if i < self.matches.len() { i } else { 0 });
        self.current().map(|(_, m)| m)
    }

    /// Moves to the next match, after the last one to the first.
    pub fn select_next(&mut self) -> Option<&SearchMatch> {
        let n = self.matches.len();
        if n == 0 {
            return None;
        }
        self.current = Some(self.current.map_or(0, |c| (c + 1) % n));
        self.current().map(|(_, m)| m)
    }

    /// Moves to the previous match, before the first one to the last.
    pub fn select_previous(&mut self) -> Option<&SearchMatch> {
        let n = self.matches.len();
        if n == 0 {
            return None;
        }
        self.current = Some(self.current.map_or(n - 1, |c| (c + n - 1) % n));
        self.current().map(|(_, m)| m)
    }

    /// The matches on `page`, with whether each is the current one.
    pub fn on_page(&self, page: usize) -> impl Iterator<Item = (&SearchMatch, bool)> {
        let start = self.matches.partition_point(|m| m.page < page);
        let end = self.matches.partition_point(|m| m.page <= page);
        (start..end).map(move |i| (&self.matches[i], Some(i) == self.current))
    }
}
