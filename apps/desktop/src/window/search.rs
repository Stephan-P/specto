//! The search bar and what it finds.

use super::*;

impl Window {
    pub(super) fn setup_search(&self) {
        let inner = &self.0;
        let weak = Rc::downgrade(&self.0);
        inner.search_entry.connect_search_changed(move |_| {
            if let Some(inner) = weak.upgrade() {
                Window(inner).run_search();
            }
        });
        // Enter: next match. Ctrl+G and Shift+Ctrl+G come with the entry.
        let weak = Rc::downgrade(&self.0);
        inner.search_entry.connect_activate(move |_| {
            if let Some(inner) = weak.upgrade() {
                Window(inner).search_step(true);
            }
        });
        let weak = Rc::downgrade(&self.0);
        inner.search_entry.connect_next_match(move |_| {
            if let Some(inner) = weak.upgrade() {
                Window(inner).search_step(true);
            }
        });
        let weak = Rc::downgrade(&self.0);
        inner.search_entry.connect_previous_match(move |_| {
            if let Some(inner) = weak.upgrade() {
                Window(inner).search_step(false);
            }
        });
        // Escape closes the bar.
        let weak = Rc::downgrade(&self.0);
        inner.search_entry.connect_stop_search(move |_| {
            if let Some(inner) = weak.upgrade() {
                inner.search_bar.set_search_mode(false);
            }
        });
        // Shift+Enter: previous match.
        let keys = gtk::EventControllerKey::new();
        let weak = Rc::downgrade(&self.0);
        keys.connect_key_pressed(move |_, key, _, state| {
            let enter = matches!(key, gdk::Key::Return | gdk::Key::KP_Enter);
            if enter && state.contains(gdk::ModifierType::SHIFT_MASK) {
                if let Some(inner) = weak.upgrade() {
                    Window(inner).search_step(false);
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        inner.search_entry.add_controller(keys);
        // Closing the bar (Escape, the button) ends the search. Opening it again searches for
        // what's still in the entry.
        let weak = Rc::downgrade(&self.0);
        inner
            .search_bar
            .connect_search_mode_enabled_notify(move |bar| {
                let Some(inner) = weak.upgrade() else { return };
                let w = Window(inner);
                if bar.is_search_mode() {
                    w.run_search();
                } else {
                    if let Some(doc) = w.0.document.borrow().as_ref() {
                        doc.borrow_mut().end_search();
                    }
                    w.update_search_status();
                    w.0.canvas.queue_draw();
                    w.0.canvas.grab_focus();
                }
            });
    }

    /// Searches for the entry's text, goes to the first match from the current page on, and indexes
    /// the rest of the document if needed.
    pub(super) fn run_search(&self) {
        let Some(doc) = self.0.document.borrow().clone() else {
            return;
        };
        let text = self.0.search_entry.text();
        let (found, indexer) = {
            let mut d = doc.borrow_mut();
            let found = d.search(&text);
            let from = d.viewer.current_page();
            let indexer = (!text.trim().is_empty())
                .then(|| d.start_indexing(from))
                .flatten();
            (found, indexer)
        };
        if found {
            self.reveal_match();
        }
        if let Some(rx) = indexer {
            self.receive_index(rx);
        }
        self.update_search_status();
        self.0.canvas.queue_draw();
    }

    /// Indexes pages the current search hasn't seen yet (inserted ones), if there is a search.
    pub(super) fn index_for_search(&self) {
        let Some(doc) = self.0.document.borrow().clone() else {
            return;
        };
        let rx = {
            let mut d = doc.borrow_mut();
            if d.search_status().is_none() {
                return;
            }
            let from = d.viewer.current_page();
            d.start_indexing(from)
        };
        if let Some(rx) = rx {
            self.receive_index(rx);
        }
    }

    /// Takes in the indexer's pages as they come, updating the search.
    ///
    /// The pages are for the document shown now: if it's replaced (reloaded) meanwhile, the rest
    /// are dropped.
    pub(super) fn receive_index(&self, rx: async_channel::Receiver<Indexed>) {
        let weak = Rc::downgrade(&self.0);
        let Some(for_doc) = self.0.document.borrow().as_ref().map(Rc::downgrade) else {
            return;
        };
        glib::spawn_future_local(async move {
            while let Ok(page) = rx.recv().await {
                let Some(inner) = weak.upgrade() else { break };
                let w = Window(inner);
                let Some(doc) = w.0.document.borrow().clone() else {
                    break;
                };
                if !std::ptr::eq(Rc::as_ptr(&doc), for_doc.as_ptr()) {
                    break;
                }
                let before = doc.borrow().search_status();
                let new_current = doc.borrow_mut().indexed(page);
                if new_current {
                    w.reveal_match();
                } else if doc.borrow().search_status() != before {
                    w.update_search_status();
                    w.0.canvas.queue_draw();
                }
            }
        });
    }

    pub(super) fn search_step(&self, forward: bool) {
        let Some(doc) = self.0.document.borrow().clone() else {
            return;
        };
        if !self.0.search_bar.is_search_mode() {
            self.0.search_bar.set_search_mode(true);
            return;
        }
        if doc.borrow_mut().search_step(forward) {
            self.reveal_match();
        } else {
            self.0.window.error_bell();
        }
    }

    /// Scrolls the current match into view and updates the status.
    pub(super) fn reveal_match(&self) {
        let Some(doc) = self.0.document.borrow().clone() else {
            return;
        };
        doc.borrow_mut().reveal_match();
        self.0.canvas.update_viewer(|_| {});
        self.update_search_status();
    }

    pub(super) fn update_search_status(&self) {
        let inner = &self.0;
        let status = inner
            .document
            .borrow()
            .as_ref()
            .and_then(|d| d.borrow().search_status());
        let (text, none_found) = match status {
            None => (String::new(), false),
            Some(s) => {
                let done = s.pages_indexed == s.pages;
                match (s.matches, s.current) {
                    (0, _) if done => (gettext("No matches"), true),
                    (0, _) => (gettext("Searching…"), false),
                    (n, Some(c)) => {
                        let args: &[(&str, &dyn std::fmt::Display)] =
                            &[("current", &(c + 1)), ("total", &n)];
                        let text = if done {
                            gettext_f("{current} of {total}", args)
                        } else {
                            // Translators: the search is still running, so there may be
                            // more matches than {total}.
                            gettext_f("{current} of {total}…", args)
                        };
                        (text, false)
                    }
                    (n, None) => (format!("{n}{}", if done { "" } else { "…" }), false),
                }
            }
        };
        inner.search_status.set_label(&text);
        if none_found {
            inner.search_entry.add_css_class("error");
        } else {
            inner.search_entry.remove_css_class("error");
        }
    }
}
