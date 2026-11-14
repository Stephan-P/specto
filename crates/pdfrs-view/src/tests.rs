use std::collections::HashSet;

use pdfrs_geom::{
    PageGeometry, PagePoint, PdfBox, PdfPoint, Rotation, ViewPoint, ViewSize, ViewVector,
};
use pdfrs_render::tile::{DocId, TileKey};

use crate::{LayoutMode, Settings, ViewPage, Viewer, ZoomMode, placeholder_level};

fn page(i: u32, w: f64, h: f64) -> ViewPage {
    ViewPage {
        doc: DocId::from_raw(0),
        page: i,
        geometry: PageGeometry::new(
            PdfBox::new(PdfPoint::new(0.0, 0.0), PdfPoint::new(w, h)),
            None,
            Rotation::None,
        ),
        rotation: Rotation::None,
    }
}

fn letters(n: u32) -> Vec<ViewPage> {
    (0..n).map(|i| page(i, 612.0, 792.0)).collect()
}

/// 1000 × 800 logical px at scale 1, gap and margin 10.
fn viewer(pages: Vec<ViewPage>) -> Viewer {
    let mut v = Viewer::new(
        pages,
        Settings {
            gap: 10.0,
            margin: 10.0,
            ..Settings::default()
        },
    );
    v.set_viewport(ViewSize::new(1000.0, 800.0), 1.0);
    v
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

#[test]
fn fit_width_and_fit_page() {
    let mut v = viewer(letters(3));
    assert_eq!(v.zoom_mode(), ZoomMode::FitWidth);
    assert!(close(v.zoom(), (1000.0 - 20.0) / 612.0));
    v.set_zoom_mode(ZoomMode::FitPage);
    assert!(close(v.zoom(), (800.0 - 20.0) / 792.0));
    // Two-page: two pages plus one gap must fit the width.
    v.set_zoom_mode(ZoomMode::FitWidth);
    v.set_layout_mode(LayoutMode::TwoPageContinuous);
    assert!(close(v.zoom(), (1000.0 - 20.0 - 10.0) / 1224.0));
}

#[test]
fn continuous_scrolling_and_current_page() {
    let mut v = viewer(letters(10));
    v.set_zoom(0.5, ViewPoint::origin()); // pages 306 × 396 + 10 gap
    assert_eq!(v.current_page(), 0);
    let r0 = v.page_rect(0).unwrap();
    assert!(close(r0.origin.y, 10.0));
    assert!(close(r0.origin.x, (1000.0 - 306.0) / 2.0));

    v.scroll_by(ViewVector::new(0.0, 406.0 * 3.0));
    assert_eq!(v.current_page(), 3);
    assert!(close(v.page_rect(3).unwrap().origin.y, 10.0));

    // Clamped at the end: content is 10 × 406 + 10 tall.
    v.scroll_by(ViewVector::new(0.0, 1e6));
    assert!(close(v.scroll_offset().y, 10.0 * 406.0 + 10.0 - 800.0));
    v.go_to_page(4);
    assert!(close(v.page_rect(4).unwrap().origin.y, 10.0));
    v.next_page();
    assert_eq!(v.current_page(), 5);
}

#[test]
fn zoom_keeps_the_anchor_point_in_place() {
    let mut v = viewer(letters(5));
    v.set_zoom(1.0, ViewPoint::origin());
    v.scroll_by(ViewVector::new(0.0, 500.0));
    let anchor = ViewPoint::new(600.0, 300.0);
    let (page, before) = v.hit_test(anchor).unwrap();
    v.zoom_by(2.0, anchor);
    assert!(close(v.zoom(), 2.0));
    let after = v.page_to_view(page, before).unwrap();
    assert!(
        close(after.x, anchor.x) && close(after.y, anchor.y),
        "{after:?}"
    );
    assert_eq!(v.zoom_mode(), ZoomMode::Fixed);
}

#[test]
fn paged_modes_lay_out_one_spread() {
    let mut v = viewer(letters(5));
    v.set_layout_mode(LayoutMode::TwoPage);
    v.set_cover_alone(true);
    // Spreads: [0] [1 2] [3 4]
    assert!(v.page_rect(0).is_some() && v.page_rect(1).is_none());
    v.next_page();
    assert_eq!(v.current_page(), 1);
    let (a, b) = (v.page_rect(1).unwrap(), v.page_rect(2).unwrap());
    assert!(close(a.origin.y, b.origin.y));
    assert!(close(b.origin.x - a.max_x(), 10.0));
    v.go_to_page(4);
    assert_eq!(v.current_page(), 3);
    v.previous_page();
    assert_eq!(v.current_page(), 1);
    // Switching to continuous keeps the page.
    v.set_layout_mode(LayoutMode::Continuous);
    assert_eq!(v.current_page(), 1);
}

#[test]
fn short_content_is_centred() {
    let mut v = viewer(letters(1));
    v.set_zoom(0.5, ViewPoint::origin());
    let r = v.page_rect(0).unwrap();
    assert!(
        close(r.center().x, 500.0) && close(r.center().y, 400.0),
        "{r:?}"
    );
}

#[test]
fn hit_test_round_trips_and_misses_gaps() {
    let mut v = viewer(letters(3));
    v.set_zoom(1.0, ViewPoint::origin());
    let (i, p) = v.hit_test(ViewPoint::new(500.0, 300.0)).unwrap();
    assert_eq!(i, 0);
    let back = v.page_to_view(i, p).unwrap();
    assert!(close(back.x, 500.0) && close(back.y, 300.0));
    // The gap between page 0 and 1.
    let gap_y = v.page_rect(0).unwrap().max_y() + 5.0;
    assert!(v.hit_test(ViewPoint::new(500.0, gap_y)).is_none());
    assert!(v.hit_test(ViewPoint::new(5.0, 300.0)).is_none());
    let _ = PagePoint::origin();
}

#[test]
fn rotated_pages_use_displayed_size() {
    let mut pages = letters(2);
    pages[1].rotation = Rotation::Cw90;
    let mut v = viewer(pages);
    v.set_zoom(1.0, ViewPoint::origin());
    let r = v.page_rect(1).unwrap();
    assert!(close(r.size.width, 792.0) && close(r.size.height, 612.0));
}

// --- frame planning ------------------------------------------------------------

fn all_cached(_: &TileKey) -> bool {
    true
}

#[test]
fn frame_requests_placeholders_first_then_sharp_tiles() {
    let mut v = viewer(letters(20));
    v.set_viewport(ViewSize::new(1000.0, 800.0), 2.0);
    v.set_zoom(1.0, ViewPoint::origin());
    let f = v.frame(|_| false);
    assert!(!f.complete);
    // Page 0 spans y 10..802; page 1 starts below the 800 px viewport.
    let visible: Vec<usize> = f.pages.iter().map(|p| p.page).collect();
    assert_eq!(visible, vec![0]);
    let pl = placeholder_level(&v.pages()[0].displayed()).unwrap();
    // The visible page's placeholder comes first, then its sharp tiles at
    // zoom 1 × scale 2 → level 2 (1224 × 1584 px → 3 × 4 tiles; the bottom
    // row is cut off by the viewport → 3 × 4 still, 790 of 792 pt visible).
    assert_eq!((f.requests[0].page, f.requests[0].level), (0, pl));
    let sharp = f
        .requests
        .iter()
        .skip(1)
        .take_while(|k| k.page == 0 && k.level == 2)
        .count();
    assert_eq!(sharp, 12);
    // Then neighbours' placeholders, then prefetch (page 1's top tiles).
    let rest = &f.requests[1 + sharp..];
    assert_eq!((rest[0].page, rest[0].level), (1, pl));
    assert!(rest.iter().any(|k| k.page == 1 && k.level == 2));
    // No duplicates.
    let unique: HashSet<_> = f.requests.iter().collect();
    assert_eq!(unique.len(), f.requests.len());
}

#[test]
fn frame_with_everything_cached_is_complete_and_exact() {
    let mut v = viewer(letters(3));
    v.set_viewport(ViewSize::new(1000.0, 800.0), 2.0);
    v.set_zoom(1.0, ViewPoint::origin());
    let f = v.frame(all_cached);
    assert!(f.complete);
    for p in &f.pages {
        // Page rects sit on whole device pixels.
        assert_eq!(p.rect.origin.x.fract(), 0.0);
        assert_eq!(p.rect.origin.y.fract(), 0.0);
        let sharp: Vec<_> = p.layers.iter().filter(|t| t.key.level == 2).collect();
        assert!(!sharp.is_empty());
        // Device scale 2 × zoom 1 = level 2 exactly: tiles map 1:1.
        assert!(sharp.iter().all(|t| t.exact));
        for t in &sharp {
            assert_eq!(t.dest.origin.x.fract(), 0.0);
            assert_eq!(t.dest.origin.y.fract(), 0.0);
        }
    }
    // Requests only contain things outside the cache, so nothing.
    assert!(f.requests.is_empty());
}

#[test]
fn tiles_never_upscale_and_cover_the_viewport() {
    let mut v = viewer(letters(3));
    for (zoom, scale) in [(1.0, 1.0), (1.3, 1.25), (0.77, 2.0), (3.1, 1.5)] {
        v.set_viewport(ViewSize::new(1000.0, 800.0), scale);
        v.set_zoom(zoom, ViewPoint::origin());
        let f = v.frame(all_cached);
        for p in &f.pages {
            let viewport = pdfrs_geom::DeviceRect::new(
                pdfrs_geom::DevicePoint::origin(),
                (1000.0 * scale, 800.0 * scale).into(),
            );
            let Some(visible) = p.rect.intersection(&viewport) else {
                continue;
            };
            let level = p.layers.iter().map(|t| t.key.level).max().unwrap();
            let sharp: Vec<_> = p.layers.iter().filter(|t| t.key.level == level).collect();
            for t in &sharp {
                let r = t.key.region(&v.pages()[p.page].geometry).unwrap();
                assert!(
                    t.dest.size.width <= r.width() as f64 + 1e-9,
                    "upscaled at {zoom}×{scale}"
                );
            }
            // Union of sharp tiles covers the visible part (area check).
            let covered: f64 = sharp
                .iter()
                .filter_map(|t| t.dest.intersection(&visible))
                .map(|r| r.area())
                .sum();
            assert!(covered + 1e-6 >= visible.area(), "gap at {zoom}×{scale}");
        }
    }
}

#[test]
fn missing_tiles_fall_back_to_neighbouring_levels() {
    let mut v = viewer(letters(1));
    v.set_viewport(ViewSize::new(1000.0, 800.0), 1.0);
    v.set_zoom(2.0, ViewPoint::origin()); // level 2
    // Only level-1 tiles (the previous zoom) are cached.
    let f = v.frame(|k| k.level == 1);
    assert!(!f.complete);
    let levels: HashSet<i32> = f.pages[0].layers.iter().map(|t| t.key.level).collect();
    assert_eq!(levels, HashSet::from([1]));
    assert!(f.requests.iter().any(|k| k.level == 2));
}

#[test]
fn placeholder_fits_one_tile() {
    let poster = page(0, 3000.0, 5000.0);
    let l = placeholder_level(&poster.displayed()).unwrap();
    let px = poster
        .displayed()
        .pixel_size(pdfrs_render::tile::level_scale(l));
    assert!(px.width <= 512 && px.height <= 512);
    assert_eq!(
        placeholder_level(&page(0, 612.0, 792.0).displayed()),
        Some(-2)
    );
}
