//! Tiles, zoom levels and a frontend-agnostic tile cache.

use std::collections::HashMap;

use pdfrs_geom::{DeviceIntRect, PageGeometry, Rotation};

use crate::Content;

/// Edge length of a tile in device pixels.
pub const TILE_SIZE: u32 = 512;

/// Identifies a document opened in the [`Scheduler`](crate::scheduler::Scheduler).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DocId(pub(crate) u32);

/// Zoom levels are powers of √2: level `l` renders at `2^(l/2)` device px per point.
pub fn level_scale(level: i32) -> f64 {
    2f64.powf(level as f64 / 2.0)
}

/// The smallest level whose scale is at least `scale`.
///
/// Rounding *up* means tiles are only ever downsampled when drawn, never
/// upscaled, which keeps text sharp on HiDPI screens (at most 41% extra pixels).
pub fn level_for_scale(scale: f64) -> i32 {
    // The epsilon keeps exact powers (scale 2.0 → level 2) from tipping over.
    (scale.log2() * 2.0 - 1e-9).ceil() as i32
}

/// One tile of a page bitmap.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileKey {
    pub doc: DocId,
    pub page: u32,
    /// Extra clockwise rotation on top of the page's `/Rotate`.
    pub rotation: Rotation,
    pub content: Content,
    pub level: i32,
    pub tx: u32,
    pub ty: u32,
}

impl TileKey {
    /// The tile's pixel rectangle within the full page bitmap at its level, or
    /// `None` if the tile lies outside the page.
    pub fn region(&self, geometry: &PageGeometry) -> Option<DeviceIntRect> {
        let full = geometry
            .rotated(self.rotation)
            .pixel_size(level_scale(self.level));
        let (x, y) = (self.tx * TILE_SIZE, self.ty * TILE_SIZE);
        if x >= full.width || y >= full.height {
            return None;
        }
        Some(DeviceIntRect::new(
            (x, y).into(),
            (
                (x + TILE_SIZE).min(full.width),
                (y + TILE_SIZE).min(full.height),
            )
                .into(),
        ))
    }
}

/// Tile indices `(tx, ty)` covering `area` (pixels of the full page bitmap at
/// `level`), clipped to the page.
pub fn tiles_covering(
    geometry: &PageGeometry,
    level: i32,
    area: DeviceIntRect,
) -> impl Iterator<Item = (u32, u32)> {
    let full = geometry.pixel_size(level_scale(level));
    let x1 = area.max.x.min(full.width);
    let y1 = area.max.y.min(full.height);
    let (x0, y0) = (area.min.x, area.min.y);
    let empty = x0 >= x1 || y0 >= y1;
    let (tx0, ty0) = (x0 / TILE_SIZE, y0 / TILE_SIZE);
    let (tx1, ty1) = if empty {
        (tx0, ty0)
    } else {
        ((x1 - 1) / TILE_SIZE + 1, (y1 - 1) / TILE_SIZE + 1)
    };
    (ty0..ty1).flat_map(move |ty| (tx0..tx1).map(move |tx| (tx, ty)))
}

struct Entry<T> {
    value: T,
    bytes: usize,
    last_used: u64,
}

/// LRU cache of rendered tiles in whatever form the frontend draws them
/// (GDK textures, Android bitmaps, …), bounded by a byte budget.
///
/// Call [`begin_frame`](Self::begin_frame) before drawing; tiles used in the
/// current frame are never evicted.
pub struct TileCache<T> {
    map: HashMap<TileKey, Entry<T>>,
    bytes: usize,
    budget: usize,
    frame: u64,
    pinned: Box<dyn Fn(&TileKey) -> bool>,
}

impl<T> TileCache<T> {
    pub fn new(budget_bytes: usize) -> Self {
        Self {
            map: HashMap::new(),
            bytes: 0,
            budget: budget_bytes,
            frame: 0,
            pinned: Box::new(|_| false),
        }
    }

    /// Tiles matching `pinned` are never evicted (e.g. low-res placeholders).
    pub fn with_pinned(mut self, pinned: impl Fn(&TileKey) -> bool + 'static) -> Self {
        self.pinned = Box::new(pinned);
        self
    }

    pub fn begin_frame(&mut self) {
        self.frame += 1;
    }

    /// Returns the tile and marks it as used in this frame.
    pub fn get(&mut self, key: &TileKey) -> Option<&T> {
        let frame = self.frame;
        self.map.get_mut(key).map(|e| {
            e.last_used = frame;
            &e.value
        })
    }

    pub fn contains(&self, key: &TileKey) -> bool {
        self.map.contains_key(key)
    }

    pub fn insert(&mut self, key: TileKey, value: T, bytes: usize) {
        let entry = Entry {
            value,
            bytes,
            last_used: self.frame,
        };
        self.bytes += bytes;
        if let Some(old) = self.map.insert(key, entry) {
            self.bytes -= old.bytes;
        }
        if self.bytes > self.budget {
            self.evict(self.budget * 3 / 4);
        }
    }

    /// Removes everything belonging to `doc`.
    pub fn remove_document(&mut self, doc: DocId) {
        self.map.retain(|k, e| {
            let keep = k.doc != doc;
            if !keep {
                self.bytes -= e.bytes;
            }
            keep
        });
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// Evicts least recently used tiles until at most `target` bytes remain,
    /// sparing pinned tiles and those used in the current frame.
    fn evict(&mut self, target: usize) {
        let mut candidates: Vec<(u64, TileKey)> = self
            .map
            .iter()
            .filter(|(k, e)| e.last_used < self.frame && !(self.pinned)(k))
            .map(|(k, e)| (e.last_used, *k))
            .collect();
        candidates.sort_unstable();
        for (_, key) in candidates {
            if self.bytes <= target {
                break;
            }
            if let Some(e) = self.map.remove(&key) {
                self.bytes -= e.bytes;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdfrs_geom::{PdfBox, PdfPoint};

    fn letter() -> PageGeometry {
        PageGeometry::new(
            PdfBox::new(PdfPoint::new(0.0, 0.0), PdfPoint::new(612.0, 792.0)),
            None,
            Rotation::None,
        )
    }

    fn key(page: u32, level: i32) -> TileKey {
        TileKey {
            doc: DocId(0),
            page,
            rotation: Rotation::None,
            content: Content::PageOnly,
            level,
            tx: 0,
            ty: 0,
        }
    }

    #[test]
    fn levels_round_up() {
        assert_eq!(level_for_scale(1.0), 0);
        assert_eq!(level_for_scale(2.0), 2);
        assert_eq!(level_for_scale(2.01), 3);
        assert_eq!(level_for_scale(1.5), 2); // 1.5 → 2.0, not 1.41
        assert_eq!(level_for_scale(0.25), -4);
        for s in [0.3, 0.77, 1.0, 1.25, 1.5, 2.0, 2.625, 3.3] {
            let l = level_for_scale(s);
            assert!(level_scale(l) >= s - 1e-9, "{s}");
            assert!(level_scale(l - 1) < s, "{s}");
        }
    }

    #[test]
    fn tile_grid_covers_page() {
        let g = letter();
        // Level 2 → scale 2 → 1224 × 1584 px → 3 × 4 tiles.
        let all: Vec<_> = tiles_covering(
            &g,
            2,
            DeviceIntRect::new((0, 0).into(), (9999, 9999).into()),
        )
        .collect();
        assert_eq!(all.len(), 12);
        let last = TileKey {
            tx: 2,
            ty: 3,
            ..key(0, 2)
        };
        let r = last.region(&g).unwrap();
        assert_eq!(
            (r.min.x, r.min.y, r.max.x, r.max.y),
            (1024, 1536, 1224, 1584)
        );
        assert!(TileKey { tx: 3, ..key(0, 2) }.region(&g).is_none());
        // A small area in the middle touches exactly the tiles under it.
        let some: Vec<_> = tiles_covering(
            &g,
            2,
            DeviceIntRect::new((500, 500).into(), (530, 1100).into()),
        )
        .collect();
        assert_eq!(some, vec![(0, 0), (1, 0), (0, 1), (1, 1), (0, 2), (1, 2)]);
        assert_eq!(
            tiles_covering(&g, 2, DeviceIntRect::new((5, 5).into(), (5, 9).into())).count(),
            0
        );
    }

    #[test]
    fn cache_evicts_lru_but_spares_current_frame_and_pinned() {
        let mut c = TileCache::new(100).with_pinned(|k| k.level == -4);
        c.begin_frame();
        c.insert(key(0, -4), "thumb", 40); // pinned
        c.insert(key(1, 0), "old", 40);
        c.begin_frame();
        c.insert(key(2, 0), "new", 40); // over budget → evict down to 75
        assert!(c.contains(&key(0, -4)));
        assert!(!c.contains(&key(1, 0)));
        assert!(c.contains(&key(2, 0)));
        assert_eq!(c.bytes(), 80);

        c.remove_document(DocId(0));
        assert!(c.is_empty());
        assert_eq!(c.bytes(), 0);
    }
}
