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
//!
//! # Guards against expensive pages
//!
//! hayro can't be cancelled and interprets the whole page for every tile, so
//! one complex page can cost seconds per tile, and pages with huge images can
//! cost gigabytes (M1: 4.5 GB and 10 s for one tile of a 212-megapixel JPEG
//! 2000 page). The scheduler therefore:
//!
//! - marks a page **expensive** when it has more than
//!   [`Limits::heavy_image_pixels`] of images (checked at open, from the image
//!   dictionaries), or when one of its renders runs longer than
//!   [`Limits::slow_after`];
//! - renders **one tile at a time** of an expensive page, so it can occupy at
//!   most one worker (and one copy of its decoded images);
//! - drops a worker's copy of the document after rendering a heavy page, which
//!   frees hayro's image cache;
//! - starts **extra workers** while renders are stalled (running longer than
//!   [`Limits::stall_after`]) and other work is waiting, so the rest of the
//!   document keeps rendering. Extra workers exit once nothing is stalled.

use std::collections::{HashMap, HashSet, VecDeque};
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::tile::{DocId, TileKey};
use crate::{Bitmap, Document, HayroDocument, OpenError, PageGeometry, RenderError, RenderRequest};

/// After a worker has rendered this many distinct pages of one document, it
/// re-opens the document to drop hayro's ever-growing cache.
const CACHE_RESET_PAGES: usize = 32;

/// Thresholds for the guards described in the [module docs](self).
#[derive(Clone, Debug)]
pub struct Limits {
    /// A page whose render takes longer than this is expensive.
    pub slow_after: Duration,
    /// A render running longer than this is stalled: if other work is waiting
    /// and no worker is free, an extra worker starts.
    pub stall_after: Duration,
    /// A page drawing more image pixels than this is expensive from the start.
    pub heavy_image_pixels: u64,
    /// At most this many extra workers on top of the regular ones.
    pub max_extra_workers: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            slow_after: Duration::from_millis(500),
            stall_after: Duration::from_secs(1),
            // Pages around 50 MP render in ≤ 160 ms and ≤ 70 MB per tile in
            // the corpus; the problem case was 212 MP.
            heavy_image_pixels: 100_000_000,
            max_extra_workers: 4,
        }
    }
}

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

type PageId = (DocId, u32);

fn page_of(key: &TileKey) -> PageId {
    (key.doc, key.page)
}

#[derive(Default)]
struct State {
    docs: HashMap<DocId, Arc<Source>>,
    next_doc: u32,
    queue: VecDeque<TileKey>,
    /// Tiles being rendered, with their start time.
    in_flight: HashMap<TileKey, Instant>,
    finished: Vec<Finished>,
    /// Pages rendered one tile at a time.
    expensive: HashSet<PageId>,
    /// Pages with huge images: workers drop their document copy afterwards.
    heavy: HashSet<PageId>,
    /// Live worker threads, and how many of them are regular.
    workers: usize,
    base_workers: usize,
    shutdown: bool,
}

impl State {
    fn stalled(&self, now: Instant, limits: &Limits) -> usize {
        self.in_flight
            .values()
            .filter(|&&t| now.duration_since(t) >= limits.stall_after)
            .count()
    }
}

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
    notify: Box<dyn Fn() + Send + Sync>,
    limits: Limits,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

pub struct Scheduler {
    shared: Arc<Shared>,
}

impl Scheduler {
    /// Starts `threads` workers with default [`Limits`]. `notify` is called
    /// from a worker thread when finished tiles become available (once per
    /// batch, not per tile).
    pub fn new(threads: usize, notify: impl Fn() + Send + Sync + 'static) -> Self {
        Self::with_limits(threads, Limits::default(), notify)
    }

    pub fn with_limits(
        threads: usize,
        limits: Limits,
        notify: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let threads = threads.max(1);
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                workers: threads,
                base_workers: threads,
                ..State::default()
            }),
            wake: Condvar::new(),
            notify: Box::new(notify),
            limits,
        });
        for _ in 0..threads {
            spawn_worker(&shared);
        }
        let s = shared.clone();
        std::thread::Builder::new()
            .name("pdfrs-render-watchdog".into())
            .spawn(move || watchdog(&s))
            .expect("spawn render watchdog");
        Self { shared }
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
        let heavy: Vec<u32> = (0..doc.page_count())
            .filter(|&i| doc.page_image_pixels(i) > self.shared.limits.heavy_image_pixels)
            .map(|i| i as u32)
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
        for page in heavy {
            s.heavy.insert((id, page));
            s.expensive.insert((id, page));
        }
        Ok(OpenedDocument { id, pages })
    }

    /// Forgets a document. Queued tiles for it are dropped; workers release
    /// their copies the next time they look at it.
    pub fn close(&self, doc: DocId) {
        let mut s = self.lock();
        s.docs.remove(&doc);
        s.queue.retain(|k| k.doc != doc);
        s.finished.retain(|f| f.key.doc != doc);
        s.expensive.retain(|p| p.0 != doc);
        s.heavy.retain(|p| p.0 != doc);
    }

    /// Replaces the queue with `wanted`, highest priority first. Tiles being
    /// rendered, or finished but not yet taken with `take_finished`, are skipped.
    pub fn request(&self, wanted: impl IntoIterator<Item = TileKey>) {
        let mut s = self.lock();
        // Rendering, or rendered but not collected yet: don't render again.
        let mut seen: HashSet<TileKey> = s.finished.iter().map(|f| f.key).collect();
        let queue: VecDeque<TileKey> = wanted
            .into_iter()
            .filter(|k| {
                !s.in_flight.contains_key(k) && s.docs.contains_key(&k.doc) && seen.insert(*k)
            })
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

    /// Whether a page is rendered one tile at a time because it is slow or
    /// has huge images. Frontends can show "rendering complex page…".
    pub fn is_expensive(&self, doc: DocId, page: u32) -> bool {
        self.lock().expensive.contains(&(doc, page))
    }

    /// Live worker threads, including extra ones started for stalled renders.
    pub fn workers(&self) -> usize {
        self.lock().workers
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.shared.lock()
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

/// Index of the first queued tile a worker may start: tiles of an expensive
/// page wait while another tile of that page is being rendered.
fn next_eligible(
    queue: &VecDeque<TileKey>,
    in_flight: &HashMap<TileKey, Instant>,
    expensive: &HashSet<PageId>,
) -> Option<usize> {
    queue.iter().position(|k| {
        let page = page_of(k);
        !(expensive.contains(&page) && in_flight.keys().any(|f| page_of(f) == page))
    })
}

/// Whether the watchdog should start another worker.
fn should_spawn(s: &State, now: Instant, limits: &Limits) -> bool {
    let idle = s.workers.saturating_sub(s.in_flight.len());
    idle == 0
        && s.workers < s.base_workers + limits.max_extra_workers
        && s.stalled(now, limits) > 0
        && next_eligible(&s.queue, &s.in_flight, &s.expensive).is_some()
}

fn spawn_worker(shared: &Arc<Shared>) {
    let s = shared.clone();
    std::thread::Builder::new()
        .name("pdfrs-render".into())
        .spawn(move || worker(&s))
        .expect("spawn render worker");
}

fn watchdog(shared: &Arc<Shared>) {
    let mut s = shared.lock();
    while !s.shutdown {
        let now = Instant::now();
        // A render already running longer than `slow_after` makes its page
        // expensive now, so its other tiles don't start in parallel.
        let slow: Vec<PageId> = s
            .in_flight
            .iter()
            .filter(|(_, t)| now.duration_since(**t) >= shared.limits.slow_after)
            .map(|(k, _)| page_of(k))
            .collect();
        s.expensive.extend(slow);
        if should_spawn(&s, now, &shared.limits) {
            s.workers += 1;
            spawn_worker(shared);
        }
        // Check a few times per stall period, at most every 100 ms.
        let interval = (shared.limits.stall_after / 4)
            .clamp(Duration::from_millis(1), Duration::from_millis(100));
        s = shared
            .wake
            .wait_timeout(s, interval)
            .unwrap_or_else(|e| e.into_inner())
            .0;
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
        let (key, source, heavy) = {
            let mut s = shared.lock();
            loop {
                if s.shutdown {
                    return;
                }
                // Release copies of closed documents.
                docs.retain(|id, _| s.docs.contains_key(id));
                // Extra workers retire once renders are no longer stalled.
                if s.workers > s.base_workers
                    && s.stalled(Instant::now(), &shared.limits) < s.workers - s.base_workers
                {
                    s.workers -= 1;
                    return;
                }
                if let Some(i) = next_eligible(&s.queue, &s.in_flight, &s.expensive) {
                    let key = s.queue.remove(i).expect("index from position");
                    if let Some(source) = s.docs.get(&key.doc).cloned() {
                        s.in_flight.insert(key, Instant::now());
                        let heavy = s.heavy.contains(&page_of(&key));
                        break (key, source, heavy);
                    }
                    continue;
                }
                s = shared.wake.wait(s).unwrap_or_else(|e| e.into_inner());
            }
        };

        let start = Instant::now();
        let result = render_tile(&mut docs, key, &source);
        let elapsed = start.elapsed();
        if heavy {
            // Frees hayro's cache of this page's decoded images.
            docs.remove(&key.doc);
        }

        let was_empty = {
            let mut s = shared.lock();
            s.in_flight.remove(&key);
            if elapsed >= shared.limits.slow_after {
                s.expensive.insert(page_of(&key));
            }
            if !s.docs.contains_key(&key.doc) {
                continue; // closed while rendering
            }
            let was_empty = s.finished.is_empty();
            s.finished.push(Finished {
                key,
                result,
                elapsed,
            });
            was_empty
        };
        // Tiles of this page may have been waiting for this one.
        shared.wake.notify_all();
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
    let mut request = RenderRequest::page(key.page as usize, key.scale.get());
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Content;
    use crate::tile::TileScale;
    use pdfrs_geom::Rotation;

    fn key(doc: u32, page: u32, tx: u32) -> TileKey {
        TileKey {
            doc: DocId(doc),
            page,
            rotation: Rotation::None,
            content: Content::PageOnly,
            scale: TileScale::from_level(0),
            tx,
            ty: 0,
        }
    }

    #[test]
    fn expensive_pages_render_one_tile_at_a_time() {
        let now = Instant::now();
        let queue: VecDeque<_> = [key(0, 1, 1), key(0, 1, 2), key(0, 2, 0)].into();
        let in_flight = HashMap::from([(key(0, 1, 0), now)]);
        // Page 1 is cheap: its next tile may start alongside the first.
        assert_eq!(next_eligible(&queue, &in_flight, &HashSet::new()), Some(0));
        // Page 1 is expensive: skip its tiles, start page 2's.
        let expensive = HashSet::from([(DocId(0), 1)]);
        assert_eq!(next_eligible(&queue, &in_flight, &expensive), Some(2));
        // Nothing else queued: wait.
        let only_page_1: VecDeque<_> = [key(0, 1, 1)].into();
        assert_eq!(next_eligible(&only_page_1, &in_flight, &expensive), None);
    }

    #[test]
    fn spawns_extra_workers_only_for_stalls_with_waiting_work() {
        let limits = Limits {
            max_extra_workers: 1,
            ..Limits::default()
        };
        let start = Instant::now();
        let later = start + limits.stall_after;
        let mut s = State {
            workers: 1,
            base_workers: 1,
            in_flight: HashMap::from([(key(0, 0, 0), start)]),
            queue: [key(0, 1, 0)].into(),
            ..State::default()
        };
        assert!(!should_spawn(&s, start, &limits), "not stalled yet");
        assert!(should_spawn(&s, later, &limits));
        s.queue.clear();
        assert!(!should_spawn(&s, later, &limits), "nothing waiting");
        s.queue.push_back(key(0, 1, 0));
        s.workers = 2;
        assert!(!should_spawn(&s, later, &limits), "cap reached");
    }
}
