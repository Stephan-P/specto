//! Background rendering of tiles.
//!
//! The frontend tells the scheduler, once per frame, which tiles it wants in
//! priority order ([`Scheduler::request`]). That list *replaces* the queue, so
//! tiles that scrolled out of view are never rendered. Finished tiles are
//! collected with [`Scheduler::take_finished`] after the `notify` callback
//! fires (the frontend uses it to wake its main loop).
//!
//! Each worker thread opens its own copy of every document it renders, because
//! hayro's caches are not `Send`. A panic while rendering is caught and
//! reported as [`RenderError::Panicked`] for that tile only.

use std::collections::{HashMap, HashSet, VecDeque};
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::tile::{DocId, TileKey, level_scale};
use crate::{Bitmap, Document, HayroDocument, OpenError, PageGeometry, RenderError, RenderRequest};

/// After a worker has rendered this many distinct pages of one document, it
/// re-opens the document to drop hayro's ever-growing cache.
const CACHE_RESET_PAGES: usize = 32;

/// A tile that finished rendering (successfully or not).
#[derive(Debug)]
pub struct Finished {
    pub key: TileKey,
    pub result: Result<Bitmap, RenderError>,
    pub elapsed: Duration,
}

/// Information about a document opened with [`Scheduler::open`].
#[derive(Clone, Debug)]
pub struct OpenedDocument {
    pub id: DocId,
    pub pages: Vec<PageGeometry>,
}

struct Source {
    data: Arc<[u8]>,
    password: Option<String>,
}

#[derive(Default)]
struct State {
    docs: HashMap<DocId, Arc<Source>>,
    next_doc: u32,
    queue: VecDeque<TileKey>,
    in_flight: HashSet<TileKey>,
    finished: Vec<Finished>,
    shutdown: bool,
}

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
    notify: Box<dyn Fn() + Send + Sync>,
}

pub struct Scheduler {
    shared: Arc<Shared>,
    _workers: Vec<JoinHandle<()>>,
}

impl Scheduler {
    /// Starts `threads` workers. `notify` is called from a worker thread when
    /// finished tiles become available (once per batch, not per tile).
    pub fn new(threads: usize, notify: impl Fn() + Send + Sync + 'static) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            wake: Condvar::new(),
            notify: Box::new(notify),
        });
        let workers = (0..threads.max(1))
            .map(|i| {
                let shared = shared.clone();
                std::thread::Builder::new()
                    .name(format!("pdfrs-render-{i}"))
                    .spawn(move || worker(&shared))
                    .expect("spawn render worker")
            })
            .collect();
        Self {
            shared,
            _workers: workers,
        }
    }

    /// Opens a document, validating it on the calling thread.
    pub fn open(
        &self,
        data: impl Into<Arc<[u8]>>,
        password: Option<&str>,
    ) -> Result<OpenedDocument, OpenError> {
        let data: Arc<[u8]> = data.into();
        let doc = HayroDocument::open(data.clone(), password)?;
        let pages = (0..doc.page_count())
            .map(|i| doc.page_geometry(i).expect("index in range"))
            .collect();
        let mut s = self.lock();
        let id = DocId(s.next_doc);
        s.next_doc += 1;
        s.docs.insert(
            id,
            Arc::new(Source {
                data,
                password: password.map(str::to_owned),
            }),
        );
        Ok(OpenedDocument { id, pages })
    }

    /// Forgets a document. Queued tiles for it are dropped; workers release
    /// their copies the next time they look at it.
    pub fn close(&self, doc: DocId) {
        let mut s = self.lock();
        s.docs.remove(&doc);
        s.queue.retain(|k| k.doc != doc);
        s.finished.retain(|f| f.key.doc != doc);
    }

    /// Replaces the queue with `wanted`, highest priority first. Tiles already
    /// being rendered are skipped.
    pub fn request(&self, wanted: impl IntoIterator<Item = TileKey>) {
        let mut s = self.lock();
        let mut seen = HashSet::new();
        let queue: VecDeque<TileKey> = wanted
            .into_iter()
            .filter(|k| !s.in_flight.contains(k) && s.docs.contains_key(&k.doc) && seen.insert(*k))
            .collect();
        s.queue = queue;
        drop(s);
        self.shared.wake.notify_all();
    }

    /// Takes all tiles finished since the last call.
    pub fn take_finished(&self) -> Vec<Finished> {
        std::mem::take(&mut self.lock().finished)
    }

    /// Number of tiles queued or being rendered.
    pub fn pending(&self) -> usize {
        let s = self.lock();
        s.queue.len() + s.in_flight.len()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.shared.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Drop for Scheduler {
    /// Signals workers to stop but does not join them: a worker stuck on a
    /// pathological page must not block closing the app.
    fn drop(&mut self) {
        self.lock().shutdown = true;
        self.shared.wake.notify_all();
    }
}

/// A worker's private copy of a document.
struct Local {
    source: Arc<Source>,
    doc: HayroDocument,
    pages_seen: HashSet<u32>,
}

fn worker(shared: &Shared) {
    let mut docs: HashMap<DocId, Local> = HashMap::new();
    loop {
        let (key, source) = {
            let mut s = shared.state.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if s.shutdown {
                    return;
                }
                // Release copies of closed documents.
                docs.retain(|id, _| s.docs.contains_key(id));
                if let Some(key) = s.queue.pop_front()
                    && let Some(source) = s.docs.get(&key.doc).cloned()
                {
                    s.in_flight.insert(key);
                    break (key, source);
                }
                if s.queue.is_empty() {
                    s = shared.wake.wait(s).unwrap_or_else(|e| e.into_inner());
                }
            }
        };

        let start = Instant::now();
        let result = render_tile(&mut docs, key, &source);
        let finished = Finished {
            key,
            result,
            elapsed: start.elapsed(),
        };

        let was_empty = {
            let mut s = shared.state.lock().unwrap_or_else(|e| e.into_inner());
            s.in_flight.remove(&key);
            if !s.docs.contains_key(&key.doc) {
                continue; // closed while rendering
            }
            let was_empty = s.finished.is_empty();
            s.finished.push(finished);
            was_empty
        };
        if was_empty {
            (shared.notify)();
        }
    }
}

fn render_tile(
    docs: &mut HashMap<DocId, Local>,
    key: TileKey,
    source: &Arc<Source>,
) -> Result<Bitmap, RenderError> {
    // (Re-)open if missing, replaced, or the cache has grown large.
    let stale = docs
        .get(&key.doc)
        .is_none_or(|l| !Arc::ptr_eq(&l.source, source) || l.pages_seen.len() >= CACHE_RESET_PAGES);
    if stale {
        docs.remove(&key.doc);
        let doc = HayroDocument::open(source.data.clone(), source.password.as_deref())
            .map_err(|_| RenderError::PageOutOfRange(key.page as usize))?;
        docs.insert(
            key.doc,
            Local {
                source: source.clone(),
                doc,
                pages_seen: HashSet::new(),
            },
        );
    }
    let local = docs.get_mut(&key.doc).expect("just inserted");
    local.pages_seen.insert(key.page);

    let geometry = local
        .doc
        .page_geometry(key.page as usize)
        .ok_or(RenderError::PageOutOfRange(key.page as usize))?;
    let region = key
        .region(&geometry)
        .ok_or(RenderError::RegionOutOfBounds)?;
    let mut request = RenderRequest::page(key.page as usize, level_scale(key.level));
    request.rotation = key.rotation;
    request.content = key.content;
    request.region = Some(region);

    // hayro forbids unsafe code, so a panic cannot leave memory in a broken
    // state; but its caches might be inconsistent, so drop this copy.
    match std::panic::catch_unwind(AssertUnwindSafe(|| local.doc.render(&request))) {
        Ok(result) => result,
        Err(_) => {
            docs.remove(&key.doc);
            Err(RenderError::Panicked)
        }
    }
}
