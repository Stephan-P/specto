//! Render worker pool.
//!
//! The GUI publishes the full list of tiles it wants, in priority order, once per
//! frame; that list *replaces* the queue, so tiles that scrolled out of view are
//! dropped before anyone renders them. Each worker owns its own parsed document
//! (hayro's caches are `!Send`).

use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

use pdfrs_render::{Bitmap, Document, HayroDocument, PixelRect, RenderRequest};

pub const TILE: u32 = 512;

/// A tile of a page at a zoom level. Level `l` means scale `2^(l/2)` device px per pt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileKey {
    pub page: u32,
    pub level: i32,
    pub tx: u32,
    pub ty: u32,
}

pub fn level_scale(level: i32) -> f64 {
    2f64.powf(level as f64 / 2.0)
}

pub fn scale_level(scale: f64) -> i32 {
    (scale.log2() * 2.0).round() as i32
}

pub struct Rendered {
    pub key: TileKey,
    pub bitmap: Bitmap,
    pub ms: f64,
}

#[derive(Default)]
struct Shared {
    queue: VecDeque<TileKey>,
    in_flight: HashSet<TileKey>,
    shutdown: bool,
}

pub struct Pool {
    shared: Arc<(Mutex<Shared>, Condvar)>,
}

impl Pool {
    pub fn new(
        data: Arc<Vec<u8>>,
        threads: usize,
        results: async_channel::Sender<Rendered>,
    ) -> Self {
        let shared = Arc::new((Mutex::new(Shared::default()), Condvar::new()));
        for _ in 0..threads {
            let shared = shared.clone();
            let data = data.clone();
            let results = results.clone();
            std::thread::spawn(move || {
                let doc = HayroDocument::open((*data).clone(), None).expect("worker open");
                loop {
                    let key = {
                        let (lock, cv) = &*shared;
                        let mut s = lock.lock().unwrap();
                        loop {
                            if s.shutdown {
                                return;
                            }
                            if let Some(k) = s.queue.pop_front() {
                                s.in_flight.insert(k);
                                break k;
                            }
                            s = cv.wait(s).unwrap();
                        }
                    };
                    let start = Instant::now();
                    let bitmap = render_tile(&doc, key);
                    let ms = start.elapsed().as_secs_f64() * 1000.0;
                    shared.0.lock().unwrap().in_flight.remove(&key);
                    if let Some(bitmap) = bitmap
                        && results.send_blocking(Rendered { key, bitmap, ms }).is_err()
                    {
                        return;
                    }
                }
            });
        }
        Self { shared }
    }

    /// Replaces the queue with `wanted` (highest priority first).
    pub fn set_wanted(&self, wanted: Vec<TileKey>) {
        let (lock, cv) = &*self.shared;
        let mut s = lock.lock().unwrap();
        let in_flight = std::mem::take(&mut s.in_flight);
        s.queue = wanted
            .into_iter()
            .filter(|k| !in_flight.contains(k))
            .collect();
        s.in_flight = in_flight;
        cv.notify_all();
    }

    pub fn queue_len(&self) -> usize {
        self.shared.0.lock().unwrap().queue.len()
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        self.shared.0.lock().unwrap().shutdown = true;
        self.shared.1.notify_all();
    }
}

/// Pixel size of a page at a level.
pub fn page_pixels(doc: &impl Document, page: u32, level: i32) -> Option<(u32, u32)> {
    Some(
        doc.page_size(page as usize)?
            .pixels(level_scale(level) as f32),
    )
}

fn render_tile(doc: &HayroDocument, key: TileKey) -> Option<Bitmap> {
    let (w, h) = page_pixels(doc, key.page, key.level)?;
    let (x, y) = (key.tx * TILE, key.ty * TILE);
    if x >= w || y >= h {
        return None;
    }
    let mut req = RenderRequest::page(key.page as usize, level_scale(key.level) as f32);
    req.region = Some(PixelRect {
        x,
        y,
        width: TILE.min(w - x),
        height: TILE.min(h - y),
    });
    doc.render(&req).ok()
}
