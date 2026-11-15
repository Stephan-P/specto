//! Searching: how this session runs the shared one's indexer.
//!
//! The search itself is `specto_session`'s. Every page's text is prepared for it once, by a
//! background thread, from the first search on. What is here hands that thread's pages to the
//! session and tells the frontend when what it shows of the search has changed.

use std::sync::Arc;

use crate::{Shared, State};

/// The frontend hears about every this many pages indexed, to show how far the search has got (and
/// at once when a page adds matches).
const PROGRESS_PAGES: usize = 16;

/// Starts preparing the pages not indexed yet for searching, in a background thread: from the
/// current page to the end, then from the start. Does nothing if that is already running or done.
pub(crate) fn start_indexing(shared: &Arc<Shared>, state: &mut State) {
    let from = state.session.viewer.current_page();
    let session = Arc::downgrade(shared);
    let listener = shared.listener.clone();
    let mut done = 0usize;
    state.session.start_indexing(from, move |page| {
        // The session may have been closed meanwhile.
        let Some(shared) = session.upgrade() else {
            return false;
        };
        let mut state = shared.lock();
        let before = state.session.search_status();
        if state.session.indexed(page) {
            // Nothing was found until now: go to what there is.
            state.session.reveal_match();
        }
        let after = state.session.search_status();
        drop(state);
        drop(shared);
        done += 1;
        let found = before.map(|s| s.matches) != after.map(|s| s.matches);
        let finished = after.is_some_and(|s| s.pages_indexed == s.pages);
        if found || finished || done.is_multiple_of(PROGRESS_PAGES) {
            listener.search_changed();
        }
        true
    });
}
