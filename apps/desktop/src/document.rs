//! An open document: scheduler, viewer state and tile cache, shared by the
//! canvas and the thumbnail sidebar of one window.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use gtk::prelude::*;
use gtk::{gdk, glib};
use pdfrs_geom::ViewPoint;
use pdfrs_render::OpenError;
use pdfrs_render::scheduler::Scheduler;
use pdfrs_render::tile::{TileCache, TileKey};
use pdfrs_source::{Link, SourceDocument, Target};
use pdfrs_view::{Settings, ViewPage, Viewer, placeholder_level};

const CACHE_BUDGET: usize = 384 << 20;

pub type SharedDocument = Rc<RefCell<Document>>;

pub struct Document {
    pub name: String,
    /// Structure (outline, links, metadata), separate from rendering.
    pub source: SourceDocument,
    /// Links per page, parsed on first use.
    links: HashMap<usize, Vec<Link>>,
    pub viewer: Viewer,
    pub cache: TileCache<gdk::Texture>,
    scheduler: Scheduler,
    /// Requests from the canvas's last frame, highest priority first.
    canvas_requests: Vec<TileKey>,
    /// Pages whose thumbnails are currently on screen in the sidebar.
    pub thumbnails_shown: BTreeSet<usize>,
    /// Tiles that failed to render; never requested again.
    failed: HashSet<TileKey>,
}

impl Document {
    /// Parses `data` and starts render workers. `wake` is called from a worker
    /// thread when tiles are ready; it should get the main loop to call
    /// [`receive_tiles`](Self::receive_tiles).
    pub fn open(
        name: String,
        data: Vec<u8>,
        password: Option<&str>,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Result<Self, OpenError> {
        let threads = std::thread::available_parallelism().map_or(2, |n| n.get());
        let scheduler = Scheduler::new(threads, wake);
        let data: Arc<[u8]> = data.into();
        let opened = scheduler.open(data.clone(), password)?;
        let source = SourceDocument::open(data, password)?;
        let pages: Vec<ViewPage> = opened
            .pages
            .iter()
            .enumerate()
            .map(|(i, &geometry)| ViewPage {
                doc: opened.id,
                page: i as u32,
                geometry,
                rotation: pdfrs_geom::Rotation::None,
            })
            .collect();
        // Placeholders double as thumbnails: evict them only after everything else.
        let levels: BTreeSet<i32> = pages
            .iter()
            .filter_map(|p| placeholder_level(&p.displayed()))
            .collect();
        Ok(Self {
            name,
            source,
            links: HashMap::new(),
            viewer: Viewer::new(pages, Settings::default()),
            cache: TileCache::new(CACHE_BUDGET)
                .with_evict_last(move |k| k.scale.level().is_some_and(|l| levels.contains(&l))),
            scheduler,
            canvas_requests: Vec::new(),
            thumbnails_shown: BTreeSet::new(),
            failed: HashSet::new(),
        })
    }

    /// The target of the link under `p` (viewport coordinates), if any.
    pub fn link_at(&mut self, p: ViewPoint) -> Option<Target> {
        let (page, at) = self.viewer.hit_test(p)?;
        let view_page = self.viewer.pages().get(page)?;
        let pdf = view_page.displayed().page_to_pdf().transform_point(at);
        let source_page = view_page.page as usize;
        let source = &self.source;
        let links = self
            .links
            .entry(source_page)
            .or_insert_with(|| source.links(source_page));
        links
            .iter()
            .rev()
            .find(|l| l.contains(pdf))
            .map(|l| l.target.clone())
    }

    pub fn set_canvas_requests(&mut self, requests: Vec<TileKey>) {
        if requests != self.canvas_requests {
            self.canvas_requests = requests;
            self.flush_requests();
        }
    }

    /// Sends the combined request list: canvas first, then sidebar thumbnails.
    /// (`Scheduler::request` replaces the whole queue, so it must see both.)
    pub fn flush_requests(&self) {
        let thumbnails = self
            .thumbnails_shown
            .iter()
            .filter_map(|&i| self.viewer.placeholder_key(i))
            .filter(|k| !self.cache.contains(k));
        let all = self.canvas_requests.iter().copied().chain(thumbnails);
        self.scheduler
            .request(all.filter(|k| !self.failed.contains(k)));
    }

    /// Moves finished tiles into the cache. Returns whether any arrived.
    pub fn receive_tiles(&mut self) -> bool {
        let finished = self.scheduler.take_finished();
        let any = !finished.is_empty();
        for f in finished {
            match f.result {
                Ok(bitmap) => {
                    let bytes = bitmap.data.len();
                    let texture = gdk::MemoryTexture::new(
                        bitmap.width as i32,
                        bitmap.height as i32,
                        gdk::MemoryFormat::R8g8b8a8Premultiplied,
                        &glib::Bytes::from_owned(bitmap.data),
                        bitmap.width as usize * 4,
                    );
                    self.cache.insert(f.key, texture.upcast(), bytes);
                }
                Err(e) => {
                    glib::g_warning!("pdfrs", "rendering {:?} failed: {e}", f.key);
                    self.failed.insert(f.key);
                }
            }
        }
        any
    }
}
