//! Arranging pages into spreads and spreads into a document layout.

use pdfrs_geom::{LayoutPoint, LayoutRect, LayoutSize, PageSize};

/// How pages are arranged.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LayoutMode {
    /// One page at a time; other pages are reached by paging.
    SinglePage,
    /// All pages in one vertical column.
    #[default]
    Continuous,
    /// Two pages side by side, one spread at a time.
    TwoPage,
    /// Two pages side by side, all spreads in one vertical column.
    TwoPageContinuous,
}

impl LayoutMode {
    pub fn is_continuous(self) -> bool {
        matches!(self, Self::Continuous | Self::TwoPageContinuous)
    }

    pub fn is_two_page(self) -> bool {
        matches!(self, Self::TwoPage | Self::TwoPageContinuous)
    }
}

/// Groups page indices into spreads.
///
/// With `cover_alone`, the first page stands on its own (like a book cover),
/// so that even pages end up on the left.
pub fn group_spreads(page_count: usize, two_page: bool, cover_alone: bool) -> Vec<Vec<usize>> {
    if !two_page {
        return (0..page_count).map(|i| vec![i]).collect();
    }
    let mut spreads = Vec::new();
    let mut i = 0;
    if cover_alone && page_count > 0 {
        spreads.push(vec![0]);
        i = 1;
    }
    while i < page_count {
        spreads.push((i..(i + 2).min(page_count)).collect());
        i += 2;
    }
    spreads
}

/// Size in points of a spread (pages side by side, no gaps).
pub(crate) fn spread_points(spread: &[usize], sizes: &[PageSize]) -> (f64, f64) {
    spread.iter().fold((0.0, 0.0), |(w, h), &i| {
        (w + sizes[i].width, f64::max(h, sizes[i].height))
    })
}

#[derive(Clone, Debug, Default)]
pub(crate) struct LaidSpread {
    pub pages: Vec<usize>,
    pub rect: LayoutRect,
}

/// The document laid out at a particular zoom, in logical pixels.
#[derive(Clone, Debug, Default)]
pub(crate) struct Layout {
    /// Spreads in this layout. In paged modes only the current one.
    pub spreads: Vec<LaidSpread>,
    /// Index into the full spread list of `spreads[0]`.
    pub first_spread: usize,
    /// Page rectangles; `None` for pages not in this layout (paged modes).
    pub page_rects: Vec<Option<LayoutRect>>,
    pub size: LayoutSize,
}

pub(crate) struct LayoutParams<'a> {
    pub sizes: &'a [PageSize],
    pub spreads: &'a [Vec<usize>],
    /// Which spreads to include (all for continuous, one for paged).
    pub range: std::ops::Range<usize>,
    pub zoom: f64,
    /// Gap between spreads and between the pages of a spread, logical px.
    pub gap: f64,
    /// Space around the content, logical px.
    pub margin: f64,
    /// Viewport width: narrower content is centred within it.
    pub viewport_width: f64,
}

pub(crate) fn layout(p: &LayoutParams<'_>) -> Layout {
    let spread_size = |s: &[usize]| {
        let (w, h) = spread_points(s, p.sizes);
        (
            w * p.zoom + p.gap * (s.len().saturating_sub(1)) as f64,
            h * p.zoom,
        )
    };
    let widest = p.spreads[p.range.clone()]
        .iter()
        .map(|s| spread_size(s).0)
        .fold(0.0, f64::max);
    let content_width = f64::max(widest + 2.0 * p.margin, p.viewport_width);

    let mut page_rects = vec![None; p.sizes.len()];
    let mut spreads = Vec::with_capacity(p.range.len());
    let mut y = p.margin;
    for pages in &p.spreads[p.range.clone()] {
        let (w, h) = spread_size(pages);
        let mut x = ((content_width - w) / 2.0).max(p.margin);
        for &i in pages {
            let (pw, ph) = (p.sizes[i].width * p.zoom, p.sizes[i].height * p.zoom);
            // Pages of different heights are centred vertically within the spread.
            page_rects[i] = Some(LayoutRect::new(
                LayoutPoint::new(x, y + (h - ph) / 2.0),
                LayoutSize::new(pw, ph),
            ));
            x += pw + p.gap;
        }
        spreads.push(LaidSpread {
            pages: pages.clone(),
            rect: LayoutRect::new(
                LayoutPoint::new(((content_width - w) / 2.0).max(p.margin), y),
                LayoutSize::new(w, h),
            ),
        });
        y += h + p.gap;
    }
    let height = if spreads.is_empty() {
        2.0 * p.margin
    } else {
        y - p.gap + p.margin
    };
    Layout {
        spreads,
        first_spread: p.range.start,
        page_rects,
        size: LayoutSize::new(content_width, height),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grouping() {
        assert_eq!(
            group_spreads(3, false, false),
            vec![vec![0], vec![1], vec![2]]
        );
        assert_eq!(
            group_spreads(5, true, false),
            vec![vec![0, 1], vec![2, 3], vec![4]]
        );
        assert_eq!(
            group_spreads(5, true, true),
            vec![vec![0], vec![1, 2], vec![3, 4]]
        );
        assert_eq!(group_spreads(0, true, true), Vec::<Vec<usize>>::new());
    }

    #[test]
    fn continuous_two_page_layout() {
        // Letter and a shorter landscape-ish page.
        let sizes = [
            PageSize::new(612.0, 792.0),
            PageSize::new(612.0, 396.0),
            PageSize::new(612.0, 792.0),
        ];
        let spreads = group_spreads(3, true, false);
        let l = layout(&LayoutParams {
            sizes: &sizes,
            spreads: &spreads,
            range: 0..spreads.len(),
            zoom: 0.5,
            gap: 10.0,
            margin: 20.0,
            viewport_width: 800.0,
        });
        // Spread 0: 306 + 10 + 306 = 622 wide, 396 tall, centred in 800.
        let a = l.page_rects[0].unwrap();
        let b = l.page_rects[1].unwrap();
        assert_eq!((a.origin.x, a.origin.y), (89.0, 20.0));
        assert_eq!((b.origin.x, b.origin.y), (89.0 + 306.0 + 10.0, 20.0 + 99.0));
        // Spread 1 (page 2 alone) starts after spread 0 plus the gap.
        let c = l.page_rects[2].unwrap();
        assert_eq!((c.origin.x, c.origin.y), (247.0, 20.0 + 396.0 + 10.0));
        assert_eq!(
            l.size,
            LayoutSize::new(800.0, 20.0 + 396.0 + 10.0 + 396.0 + 20.0)
        );
    }
}
