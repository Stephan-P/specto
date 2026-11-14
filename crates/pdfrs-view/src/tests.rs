use std::collections::HashSet;

use pdfrs_geom::{
    PageGeometry, PagePoint, PdfBox, PdfPoint, Rotation, ViewPoint, ViewSize, ViewVector,
};
use std::time::Instant;

use pdfrs_geom::{DevicePoint, DeviceRect};
use pdfrs_render::tile::{DocId, TileKey, TileScale};

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

fn level(k: &TileKey) -> Option<i32> {
    k.scale.level()
}

#[test]
fn frame_requests_placeholders_first_then_sharp_tiles() {
    let mut v = viewer(letters(20));
    v.set_viewport(ViewSize::new(1000.0, 800.0), 2.0);
    v.set_zoom(1.0, ViewPoint::origin());
    let f = v.frame(Instant::now(), |_| false);
    assert!(!f.complete);
    // Page 0 spans y 10..802; page 1 starts below the 800 px viewport.
    let visible: Vec<usize> = f.pages.iter().map(|p| p.page).collect();
    assert_eq!(visible, vec![0]);
    let pl = placeholder_level(&v.pages()[0].displayed());
    // The visible page's placeholder comes first, then its sharp tiles at
    // zoom 1 × scale 2 → level 2 (1224 × 1584 px → 3 × 4 tiles).
    assert_eq!((f.requests[0].page, level(&f.requests[0])), (0, pl));
    let sharp = f
        .requests
        .iter()
        .skip(1)
        .take_while(|k| k.page == 0 && level(k) == Some(2))
        .count();
    assert_eq!(sharp, 12);
    // Then neighbours' placeholders, then prefetch (page 1's top tiles).
    let rest = &f.requests[1 + sharp..];
    assert_eq!((rest[0].page, level(&rest[0])), (1, pl));
    assert!(rest.iter().any(|k| k.page == 1 && level(k) == Some(2)));
    // No duplicates.
    let unique: HashSet<_> = f.requests.iter().collect();
    assert_eq!(unique.len(), f.requests.len());
}

#[test]
fn frame_with_everything_cached_is_complete_and_exact() {
    let mut v = viewer(letters(3));
    v.set_viewport(ViewSize::new(1000.0, 800.0), 2.0);
    v.set_zoom(1.0, ViewPoint::origin());
    let f = v.frame(Instant::now(), all_cached);
    assert!(f.complete);
    assert_eq!(f.settles_at, None); // 100% at scale 2 *is* level 2
    for p in &f.pages {
        // Page rects sit on whole device pixels.
        assert_eq!(p.rect.origin.x.fract(), 0.0);
        assert_eq!(p.rect.origin.y.fract(), 0.0);
        let sharp: Vec<_> = p
            .layers
            .iter()
            .filter(|t| level(&t.key) == Some(2))
            .collect();
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
    // Fully covered pages don't list the placeholder underneath (overdraw).
    for p in &f.pages {
        let pl = v.placeholder_key(p.page).unwrap();
        assert!(p.layers.iter().all(|t| t.key != pl));
    }
}

/// Checks the sharpest layer of every visible page: never upscaled, and
/// covering the visible part of the page. Returns those layers' scales.
fn check_coverage(v: &Viewer, f: &crate::Frame, what: &str) -> HashSet<TileScale> {
    let (vw, vh, scale) = (v.viewport().width, v.viewport().height, v.scale_factor());
    let viewport = DeviceRect::new(DevicePoint::origin(), (vw * scale, vh * scale).into());
    let mut scales = HashSet::new();
    for p in &f.pages {
        let Some(visible) = p.rect.intersection(&viewport) else {
            continue;
        };
        let best = p.layers.iter().map(|t| t.key.scale).max().unwrap();
        scales.insert(best);
        let sharp: Vec<_> = p.layers.iter().filter(|t| t.key.scale == best).collect();
        for t in &sharp {
            let r = t.key.region(&v.pages()[p.page].geometry).unwrap();
            assert!(
                t.dest.size.width <= r.width() as f64 + 1e-9,
                "upscaled ({what})"
            );
        }
        let covered: f64 = sharp
            .iter()
            .filter_map(|t| t.dest.intersection(&visible))
            .map(|r| r.area())
            .sum();
        assert!(covered + 1e-6 >= visible.area(), "gap ({what})");
    }
    scales
}

#[test]
fn zooming_uses_levels_and_settled_views_use_exact_tiles() {
    let mut v = viewer(letters(3));
    let t0 = Instant::now();
    let delay = v.settings().settle_delay;
    v.frame(t0, all_cached); // first frame: settled immediately
    for (zoom, scale) in [(1.3, 1.25), (0.77, 2.0), (3.1, 1.5), (0.906, 2.0)] {
        let what = format!("{zoom}×{scale}");
        v.set_viewport(ViewSize::new(1000.0, 800.0), scale);
        v.set_zoom(zoom, ViewPoint::origin());
        // Right after the change: level tiles, and a time to settle.
        let f = v.frame(t0, all_cached);
        let scales = check_coverage(&v, &f, &what);
        assert!(
            scales.iter().all(|s| s.level().is_some()),
            "{what}: {scales:?}"
        );
        assert_eq!(f.settles_at, Some(t0 + delay), "{what}");
        assert!(!v.is_settled(t0));

        // Settled: exact tiles at the device scale, drawn 1:1.
        let later = t0 + delay;
        let f = v.frame(later, all_cached);
        assert!(v.is_settled(later));
        assert_eq!(f.settles_at, None);
        let scales = check_coverage(&v, &f, &what);
        assert_eq!(scales, HashSet::from([v.device_scale()]), "{what}");
        for p in &f.pages {
            for t in p.layers.iter().filter(|t| t.key.scale == v.device_scale()) {
                assert!(t.exact, "{what}: {t:?}");
                assert_eq!(t.dest.origin.x.fract(), 0.0);
                assert_eq!(t.dest.origin.y.fract(), 0.0);
            }
        }
        // Reset the clock for the next round.
        v.set_zoom(1.0, ViewPoint::origin());
        v.frame(later, all_cached);
    }
}

#[test]
fn settled_view_requests_exact_tiles_and_falls_back_to_levels() {
    let mut v = viewer(letters(1));
    v.set_viewport(ViewSize::new(1000.0, 800.0), 2.0);
    v.set_zoom(0.906, ViewPoint::origin()); // device 1.812 → level 2 drawn at 90.6%
    let t0 = Instant::now();
    v.frame(t0, |_| false);
    let later = t0 + v.settings().settle_delay;
    // Only level tiles are cached (from while zooming).
    let f = v.frame(later, |k| level(k).is_some());
    assert!(!f.complete);
    let device = v.device_scale();
    assert_eq!(device.level(), None);
    assert!(f.pages[0].layers.iter().all(|t| level(&t.key).is_some()));
    assert!(f.pages[0].layers.iter().any(|t| level(&t.key) == Some(2)));
    assert!(f.requests.iter().any(|k| k.scale == device));
    assert!(!f.requests.iter().any(|k| level(k) == Some(2)));
}

#[test]
fn no_exact_tiles_during_a_zoom_gesture() {
    let mut v = viewer(letters(1));
    v.set_viewport(ViewSize::new(1000.0, 800.0), 2.0);
    let t0 = Instant::now();
    v.frame(t0, |_| false);
    let delay = v.settings().settle_delay;
    v.begin_zoom_gesture();
    v.zoom_by(1.1, ViewPoint::new(500.0, 400.0));
    v.frame(t0, |_| false);
    // Paused mid-pinch for a long time: still only level tiles, no deadline.
    let much_later = t0 + delay * 20;
    let f = v.frame(much_later, |_| false);
    assert!(!v.is_settled(much_later));
    assert_eq!(f.settles_at, None);
    assert!(f.requests.iter().all(|k| level(k).is_some()));
    // Released: settles after the delay, measured from the release.
    v.end_zoom_gesture();
    let f = v.frame(much_later, |_| false);
    assert_eq!(f.settles_at, Some(much_later + delay));
    let f = v.frame(much_later + delay, |_| false);
    assert!(f.requests.iter().any(|k| k.scale == v.device_scale()));
}

#[test]
fn scrolling_does_not_unsettle() {
    let mut v = viewer(letters(5));
    v.set_viewport(ViewSize::new(1000.0, 800.0), 2.0);
    v.set_zoom(0.906, ViewPoint::origin());
    let t0 = Instant::now();
    v.frame(t0, |_| false);
    let later = t0 + v.settings().settle_delay;
    v.frame(later, |_| false);
    v.scroll_by(ViewVector::new(0.0, 300.0));
    v.go_to_page(3);
    assert!(v.is_settled(later));
}

#[test]
fn missing_tiles_fall_back_to_neighbouring_levels() {
    let mut v = viewer(letters(1));
    v.set_viewport(ViewSize::new(1000.0, 800.0), 1.0);
    v.set_zoom(2.0, ViewPoint::origin()); // level 2
    // Only level-1 tiles (the previous zoom) are cached.
    let f = v.frame(Instant::now(), |k| level(k) == Some(1));
    assert!(!f.complete);
    let levels: HashSet<Option<i32>> = f.pages[0].layers.iter().map(|t| level(&t.key)).collect();
    assert_eq!(levels, HashSet::from([Some(1)]));
    assert!(f.requests.iter().any(|k| level(k) == Some(2)));
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

#[test]
fn go_to_pdf_position_puts_the_target_at_the_top() {
    let mut v = viewer(letters(5));
    v.set_zoom(1.0, ViewPoint::origin());
    // Page 3 (index 2), 200 pt below its top edge (PDF y = 792 - 200).
    v.go_to_pdf_position(2, None, Some(592.0));
    let r = v.page_rect(2).unwrap();
    assert!(close(r.origin.y + 200.0, 10.0), "{r:?}"); // margin 10
    assert_eq!(v.current_page(), 2);
    // No coordinates: the page top.
    v.go_to_pdf_position(3, None, None);
    assert!(close(v.page_rect(3).unwrap().origin.y, 10.0));
    // Out-of-page positions are clamped; the last page clamps the scroll.
    v.go_to_pdf_position(4, None, Some(-5000.0));
    assert_eq!(v.current_page(), 4);
}

#[test]
fn go_to_pdf_position_respects_rotation() {
    let mut pages = letters(4);
    pages[1].rotation = Rotation::Cw90; // displayed 792 × 612, not the last page
    let mut v = viewer(pages);
    v.set_zoom(1.0, ViewPoint::origin());
    // PDF x = 300 on a page turned clockwise becomes a page-space y of 300.
    v.go_to_pdf_position(1, Some(300.0), Some(792.0));
    let r = v.page_rect(1).unwrap();
    assert!(close(r.origin.y + 300.0, 10.0), "{r:?}");
}

#[test]
fn view_state_round_trips_across_viewers_and_sizes() {
    let mut a = viewer(letters(10));
    a.set_layout_mode(LayoutMode::TwoPageContinuous);
    a.set_cover_alone(true);
    a.set_zoom(0.8, ViewPoint::origin());
    a.go_to_pdf_position(5, None, Some(792.0 - 300.0)); // 300 pt into page 6
    let state = a.view_state();
    assert_eq!(state.page, 5);
    assert!(
        (state.page_offset - 300.0 / 792.0).abs() < 0.02,
        "{state:?}"
    );
    assert_eq!(state.zoom_mode, ZoomMode::Fixed);

    // A new viewer, as after reopening the file, in a different window size.
    let mut b = viewer(letters(10));
    b.restore_view_state(&state);
    b.set_viewport(ViewSize::new(700.0, 500.0), 2.0);
    assert_eq!(b.layout_mode(), LayoutMode::TwoPageContinuous);
    assert!(b.settings().cover_alone);
    assert!(close(b.zoom(), 0.8));
    let again = b.view_state();
    assert_eq!(again.page, 5);
    assert!(
        (again.page_offset - state.page_offset).abs() < 0.02,
        "{again:?}"
    );

    // Fewer pages now: clamped instead of failing, and the last page is in
    // view (the scroll is clamped too, so it needn't be at the top).
    let mut c = viewer(letters(3));
    c.restore_view_state(&state);
    let last = c.page_rect(2).unwrap();
    assert!(
        last.min_y() < c.viewport().height && last.max_y() > 0.0,
        "{last:?}"
    );
}

#[test]
fn mode_names_round_trip() {
    for m in LayoutMode::ALL {
        assert_eq!(LayoutMode::from_name(m.name()), Some(m));
    }
    for z in [ZoomMode::Fixed, ZoomMode::FitWidth, ZoomMode::FitPage] {
        assert_eq!(ZoomMode::from_name(z.name()), Some(z));
    }
    assert_eq!(LayoutMode::from_name("nonsense"), None);
}

#[test]
fn view_state_at_the_document_start_keeps_the_margin() {
    let mut a = viewer(letters(10));
    a.set_zoom(1.0, ViewPoint::origin());
    let state = a.view_state();
    assert_eq!(state.page, 0);
    assert!(state.page_offset < 0.0, "{state:?}");
    let mut b = viewer(letters(10));
    b.restore_view_state(&state);
    assert!(close(b.scroll_offset().y, 0.0), "{:?}", b.scroll_offset());
}

#[test]
fn the_end_of_a_continuous_document_is_its_last_page() {
    let mut v = viewer(letters(10));
    v.set_zoom(0.5, ViewPoint::origin()); // two pages per viewport
    v.scroll_by(ViewVector::new(0.0, 1e6));
    // Page 8 shows as much as page 9, but the user has reached the end.
    assert_eq!(v.current_page(), 9);
    v.scroll_by(ViewVector::new(0.0, -100.0));
    assert_eq!(v.current_page(), 8);
}

/// Letter portrait, letter landscape, and a small page, swiped one at a time
/// in a 1000 × 800 viewport (margin and gap 10).
fn swiper() -> Viewer {
    let mut v = viewer(vec![
        page(0, 612.0, 792.0),
        page(1, 792.0, 612.0),
        page(2, 300.0, 400.0),
    ]);
    v.set_layout_mode(LayoutMode::SinglePage);
    v.set_zoom_mode(ZoomMode::FitPage);
    v.set_swipe(true);
    v
}

#[test]
fn swipe_fits_each_page_on_its_own() {
    let mut v = swiper();
    // Portrait: limited by the height.
    let z0 = 780.0 / 792.0;
    assert!(close(v.zoom(), z0));
    let r0 = v.page_rect(0).unwrap();
    assert!(close(r0.origin.x, (1000.0 - 612.0 * z0) / 2.0) && close(r0.origin.y, 10.0));
    // The next page waits just beyond the right edge, already at its own fit.
    let z1 = 980.0 / 792.0;
    let r1 = v.page_rect(1).unwrap();
    assert!(close(r1.origin.x, 1000.0 + 10.0 + 10.0), "{r1:?}");
    assert!(close(r1.size.width, 792.0 * z1));

    // Landscape: limited by the width, centred vertically.
    v.go_to_page(1);
    assert_eq!(v.current_page(), 1);
    assert!(close(v.zoom(), z1));
    let r1 = v.page_rect(1).unwrap();
    assert!(close(r1.origin.x, 10.0) && close(r1.origin.y, (800.0 - 612.0 * z1) / 2.0));
    assert!(v.page_rect(0).unwrap().max_x() < 0.0);

    // A small page is enlarged to fit, too.
    v.next_page();
    assert_eq!(v.current_page(), 2);
    assert!(close(v.zoom(), 780.0 / 400.0));
    v.previous_page();
    assert_eq!(v.current_page(), 1);

    // Rotating the device refits every page and stays on the current one.
    v.set_viewport(ViewSize::new(800.0, 1000.0), 1.0);
    assert_eq!(v.current_page(), 1);
    assert!(close(v.zoom(), 780.0 / 792.0));
    assert!(close(v.page_rect(1).unwrap().origin.x, 10.0));
    assert_eq!(v.snap_delta(ViewVector::zero()), None);
}

#[test]
fn swiping_slides_between_pages_and_snaps() {
    let mut v = swiper();
    let still = ViewVector::zero();
    assert_eq!(v.snap_delta(still), None);

    // A short drag: both pages move with the finger, and letting go returns.
    let before = v.page_rect(0).unwrap().origin.x;
    v.scroll_by(ViewVector::new(300.0, 0.0));
    assert!(close(v.page_rect(0).unwrap().origin.x, before - 300.0));
    assert!(close(v.page_rect(1).unwrap().origin.x, 1020.0 - 300.0));
    assert_eq!(v.current_page(), 0);
    assert_eq!(v.snap_delta(still), Some(ViewVector::new(-300.0, 0.0)));
    // The neighbour can be hit while it is partly on screen.
    assert_eq!(
        v.hit_test(ViewPoint::new(900.0, 400.0)).map(|h| h.0),
        Some(1)
    );

    // The same drag ending in a flick turns the page. Slots are a viewport
    // wide plus the gap.
    let flick = ViewVector::new(400.0, 0.0);
    assert_eq!(v.snap_delta(flick), Some(ViewVector::new(710.0, 0.0)));
    // So does dragging more than half way.
    v.scroll_by(ViewVector::new(250.0, 0.0));
    assert_eq!(v.current_page(), 1);
    let rest = v.snap_delta(still).unwrap();
    assert!(close(rest.x, 460.0));
    // ... unless the finger was moving back.
    assert_eq!(v.snap_delta(-flick), Some(ViewVector::new(-550.0, 0.0)));

    // Arriving: page 1 is current and at rest.
    v.scroll_by(rest);
    assert_eq!(v.current_page(), 1);
    assert_eq!(v.snap_delta(still), None);
    assert!(close(v.zoom(), 980.0 / 792.0));
    assert!(close(v.page_rect(1).unwrap().origin.x, 10.0));

    // Backwards, with a flick.
    v.scroll_by(ViewVector::new(-40.0, 0.0));
    let back = v.snap_delta(-flick).unwrap();
    assert!(close(back.x, -970.0));
    v.scroll_by(back);
    assert_eq!(v.current_page(), 0);

    // There is nothing before the first or after the last page.
    v.scroll_by(ViewVector::new(-500.0, 0.0));
    assert!(close(v.scroll_offset().x, 0.0));
    assert_eq!(v.snap_delta(-flick), None);
    v.go_to_page(2);
    v.scroll_by(ViewVector::new(500.0, 0.0));
    assert_eq!(v.snap_delta(flick), None);
    assert_eq!(v.current_page(), 2);
}

#[test]
fn swipe_zoom_pans_within_the_page() {
    let mut v = swiper();
    let fit = v.zoom();
    let anchor = ViewPoint::new(600.0, 300.0);
    let (page, before) = v.hit_test(anchor).unwrap();
    v.zoom_by(2.0, anchor);
    assert_eq!(v.zoom_mode(), ZoomMode::Fixed);
    assert!(close(v.zoom(), 2.0 * fit));
    let after = v.page_to_view(page, before).unwrap();
    assert!(
        close(after.x, anchor.x) && close(after.y, anchor.y),
        "{after:?}"
    );
    // The zoomed page is larger than the viewport, so it rests anywhere
    // within its slot, and the other pages are zoomed in proportion.
    assert_eq!(v.snap_delta(ViewVector::zero()), None);
    assert!(close(v.page_rect(1).unwrap().size.width, 2.0 * 980.0));

    // A fling stops at the page's edge instead of turning the page ...
    v.scroll_in_page_by(ViewVector::new(1e6, 1e6));
    assert_eq!(v.current_page(), 0);
    assert_eq!(v.snap_delta(ViewVector::zero()), None);
    let r0 = v.page_rect(0).unwrap();
    assert!(
        close(r0.max_x(), 1000.0 - 10.0) && close(r0.max_y(), 800.0 - 10.0),
        "{r0:?}"
    );
    // ... while a drag beyond it starts sliding to the next page, which is
    // shown from its top however far down this one was scrolled.
    v.scroll_by(ViewVector::new(600.0, 0.0));
    assert!(close(v.page_rect(1).unwrap().origin.y, 10.0));
    let rest = v.snap_delta(ViewVector::zero()).unwrap();
    v.scroll_by(rest);
    assert_eq!(v.current_page(), 1);
    assert!(close(v.scroll_offset().y, 0.0));
    // Going back lands on the near edge of the previous page.
    v.scroll_by(ViewVector::new(-20.0, 0.0));
    let back = v.snap_delta(ViewVector::new(-400.0, 0.0)).unwrap();
    v.scroll_by(back);
    assert_eq!(v.current_page(), 0);
    assert!(close(v.page_rect(0).unwrap().max_x(), 1000.0 - 10.0));

    // Pages can't be made smaller than fitting the viewport. Pinching out
    // around a point off-centre can leave the page slightly aside; letting
    // go centres it again.
    v.zoom_by(0.01, anchor);
    assert!(close(v.zoom(), fit));
    let centre = v.snap_delta(ViewVector::zero()).unwrap();
    assert!(centre.x.abs() < 20.0, "{centre:?}");
    v.scroll_by(centre);
    assert_eq!(v.current_page(), 0);
    assert!(close(
        v.page_rect(0).unwrap().origin.x,
        (1000.0 - 612.0 * fit) / 2.0
    ));
    assert_eq!(v.snap_delta(ViewVector::zero()), None);

    // Leaving swipe mode keeps the page and its zoom.
    v.go_to_page(1);
    v.zoom_by(1.5, anchor);
    let zoom = v.zoom();
    v.set_swipe(false);
    assert_eq!(v.current_page(), 1);
    assert!(close(v.zoom(), zoom));
    assert!(v.page_rect(0).is_none());
    v.set_swipe(true);
    assert!(close(v.zoom(), zoom));
}

#[test]
fn swipe_frames_use_each_pages_own_scale_and_prepare_the_neighbours() {
    let mut v = swiper();
    v.set_viewport(ViewSize::new(1000.0, 800.0), 2.0);
    v.go_to_page(1);
    let scale = |v: &Viewer, i: usize| {
        TileScale::exact(
            v.page_rect(i).unwrap().size.width / v.pages()[i].displayed().size().width * 2.0,
        )
    };
    let (s0, s1, s2) = (scale(&v, 0), scale(&v, 1), scale(&v, 2));
    assert!(s0 != s1 && s1 != s2);

    // At rest only the current page is drawn, but both neighbours' tiles are
    // requested, each at its own exact scale.
    let f = v.frame(Instant::now(), |_| false);
    assert_eq!(f.pages.iter().map(|p| p.page).collect::<Vec<_>>(), vec![1]);
    let sharp = |page: u32, s: TileScale| {
        f.requests
            .iter()
            .filter(|k| k.page == page && k.scale == s)
            .count()
    };
    assert!(sharp(1, s1) > 0 && sharp(0, s0) > 0 && sharp(2, s2) > 0);
    // The visible page's tiles come before the neighbours'.
    let first_neighbour = f.requests.iter().position(|k| k.page != 1).unwrap();
    assert!(f.requests[..first_neighbour].iter().any(|k| k.scale == s1));

    // Turning the page changes the scale, but nothing is zooming: the view
    // stays settled and draws exact tiles 1:1 right away.
    let now = Instant::now();
    v.frame(now, all_cached);
    v.go_to_page(2);
    let f = v.frame(now, all_cached);
    assert!(f.complete && f.settles_at.is_none());
    assert!(v.is_settled(now));
    assert_eq!(f.pages.len(), 1);
    assert!(
        f.pages[0]
            .layers
            .iter()
            .all(|l| l.exact && l.key.scale == s2)
    );

    // While sliding, both pages are drawn, each from its own tiles.
    v.scroll_by(ViewVector::new(-400.0, 0.0));
    let f = v.frame(now, all_cached);
    let drawn: Vec<(usize, TileScale)> = f
        .pages
        .iter()
        .map(|p| (p.page, p.layers[0].key.scale))
        .collect();
    assert_eq!(drawn, vec![(1, s1), (2, s2)]);
    assert!(f.pages.iter().all(|p| p.layers.iter().all(|l| l.exact)));

    // Zooming still goes through levels and settles afterwards.
    let rest = v.snap_delta(ViewVector::zero()).unwrap();
    v.scroll_by(rest);
    v.zoom_by(1.3, ViewPoint::new(500.0, 400.0));
    let f = v.frame(now, all_cached);
    assert!(f.settles_at.is_some());
    assert!(
        f.pages[0]
            .layers
            .iter()
            .all(|l| l.key.scale.level().is_some())
    );
}

#[test]
fn view_state_round_trips_in_swipe_mode() {
    let mut a = swiper();
    a.go_to_page(1);
    a.zoom_by(2.0, ViewPoint::new(500.0, 400.0));
    a.scroll_by(ViewVector::new(0.0, 150.0));
    let state = a.view_state();
    assert_eq!((state.page, state.zoom_mode), (1, ZoomMode::Fixed));
    assert!(state.page_offset > 0.0, "{state:?}");

    // Restored in a viewer that was on another page at another zoom.
    let mut b = swiper();
    b.go_to_page(2);
    b.restore_view_state(&state);
    assert_eq!(b.current_page(), 1);
    assert!(close(b.zoom(), a.zoom()));
    assert!(close(b.scroll_offset().y, a.scroll_offset().y));
    assert_eq!(b.snap_delta(ViewVector::zero()), None);

    // A fitted page comes back fitted.
    a.set_zoom_mode(ZoomMode::FitPage);
    let state = a.view_state();
    b.restore_view_state(&state);
    assert_eq!((b.current_page(), b.zoom_mode()), (1, ZoomMode::FitPage));
    assert!(close(b.zoom(), 980.0 / 792.0));
}
