//! Typed coordinate spaces for pdfrs.
//!
//! Every point, size, rectangle and transform carries the space it lives in, so
//! mixing them up is a compile error (`PdfPoint` vs `PagePoint`). Transforms are
//! typed `Src → Dst`.
//!
//! | Space         | Unit                  | Origin / axes                                             |
//! |---------------|-----------------------|-----------------------------------------------------------|
//! | [`PdfSpace`]  | PDF points            | page user space: y up, as written in the file             |
//! | [`PageSpace`] | PDF points            | displayed page: top-left of the crop box after rotation, y down |
//! | [`LayoutSpace`] | PDF points          | whole document as laid out by the viewer, y down          |
//! | [`ViewSpace`] | logical pixels        | top-left of the viewport (what the UI toolkit uses)       |
//! | [`DeviceSpace`] | physical pixels     | top-left of the viewport / bitmap                         |
//!
//! Annotation rectangles, quad points and link targets are in [`PdfSpace`].
//! Hit-testing, selection and drawing use [`PageSpace`] and upward.

mod rotation;

pub use euclid;
pub use rotation::Rotation;

use euclid::{Box2D, Point2D, Rect, Size2D, Transform2D, Vector2D};

/// PDF user space of one page (points, y up).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PdfSpace {}
/// The page as displayed: origin at the top-left of the visible area, y down, points.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PageSpace {}
/// The whole document as laid out by the viewer (points, y down).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LayoutSpace {}
/// Logical pixels relative to the viewport.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ViewSpace {}
/// Physical pixels relative to the viewport or a bitmap.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DeviceSpace {}

pub type PdfPoint = Point2D<f64, PdfSpace>;
pub type PdfBox = Box2D<f64, PdfSpace>;
pub type PagePoint = Point2D<f64, PageSpace>;
pub type PageSize = Size2D<f64, PageSpace>;
pub type PageRect = Rect<f64, PageSpace>;
pub type PageVector = Vector2D<f64, PageSpace>;
pub type LayoutPoint = Point2D<f64, LayoutSpace>;
pub type LayoutRect = Rect<f64, LayoutSpace>;
pub type LayoutSize = Size2D<f64, LayoutSpace>;
pub type ViewPoint = Point2D<f64, ViewSpace>;
pub type ViewRect = Rect<f64, ViewSpace>;
pub type ViewSize = Size2D<f64, ViewSpace>;
pub type DevicePoint = Point2D<f64, DeviceSpace>;
pub type DeviceRect = Rect<f64, DeviceSpace>;
/// Integer pixel rectangle, e.g. a tile within a page bitmap.
pub type DeviceIntRect = Box2D<u32, DeviceSpace>;
pub type DeviceIntSize = Size2D<u32, DeviceSpace>;

pub type PdfToPage = Transform2D<f64, PdfSpace, PageSpace>;
pub type PageToPdf = Transform2D<f64, PageSpace, PdfSpace>;

/// Fallback size for degenerate pages: A4 (210 × 297 mm), matching hayro.
const A4: (f64, f64) = (210.0 / 25.4 * 72.0, 297.0 / 25.4 * 72.0);

/// The geometry of one page: which part of user space is visible and how it is rotated.
///
/// Mirrors how renderers interpret the page dictionary: the visible area is
/// CropBox ∩ MediaBox, and `/Rotate` turns it clockwise for display.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageGeometry {
    /// CropBox ∩ MediaBox, normalized (min ≤ max).
    visible: PdfBox,
    rotation: Rotation,
}

impl PageGeometry {
    /// `crop_box` defaults to the media box. Boxes may be given with any corner order.
    pub fn new(media_box: PdfBox, crop_box: Option<PdfBox>, rotation: Rotation) -> Self {
        let media = normalize(media_box);
        let visible = match crop_box {
            Some(c) => normalize(c).intersection_unchecked(&media),
            None => media,
        };
        Self { visible, rotation }
    }

    /// The same page with an additional clockwise rotation applied on top of
    /// `/Rotate` (the user's "rotate page" command).
    pub fn rotated(self, extra: Rotation) -> Self {
        Self {
            rotation: self.rotation + extra,
            ..self
        }
    }

    pub fn rotation(&self) -> Rotation {
        self.rotation
    }

    /// The visible area in PDF user space.
    pub fn visible_box(&self) -> PdfBox {
        self.visible
    }

    /// Unrotated width and height of the visible area. Degenerate (zero-area)
    /// pages are treated as A4, and each side is at least 1 pt.
    fn base_size(&self) -> (f64, f64) {
        let (w, h) = (self.visible.width(), self.visible.height());
        if !(w > f64::EPSILON && h > f64::EPSILON) {
            A4
        } else {
            (w.max(1.0), h.max(1.0))
        }
    }

    /// Size of the page as displayed, in points.
    pub fn size(&self) -> PageSize {
        let (w, h) = self.base_size();
        if self.rotation.swaps_axes() {
            PageSize::new(h, w)
        } else {
            PageSize::new(w, h)
        }
    }

    /// Size of the full-page bitmap at `scale` device pixels per point.
    ///
    /// Renderers and layout must agree on this rounding, so it lives here.
    pub fn pixel_size(&self, scale: f64) -> DeviceIntSize {
        let s = self.size();
        DeviceIntSize::new(pixel_extent(s.width, scale), pixel_extent(s.height, scale))
    }

    /// PDF user space → displayed page space.
    pub fn pdf_to_page(&self) -> PdfToPage {
        let (w, h) = self.base_size();
        // Flip y and move the visible area's top-left to the origin:
        // u = x - x0, v = base_h - (y - y0)
        let flip = Transform2D::<f64, PdfSpace, PageSpace>::new(
            1.0,
            0.0,
            0.0,
            -1.0,
            -self.visible.min.x,
            h + self.visible.min.y,
        );
        let rotate = match self.rotation {
            Rotation::None => Transform2D::identity(),
            // (u, v) → (h - v, u)
            Rotation::Cw90 => Transform2D::new(0.0, 1.0, -1.0, 0.0, h, 0.0),
            // (u, v) → (w - u, h - v)
            Rotation::Cw180 => Transform2D::new(-1.0, 0.0, 0.0, -1.0, w, h),
            // (u, v) → (v, w - u)
            Rotation::Cw270 => Transform2D::new(0.0, -1.0, 1.0, 0.0, 0.0, w),
        };
        flip.then(&rotate)
    }

    /// Displayed page space → PDF user space.
    pub fn page_to_pdf(&self) -> PageToPdf {
        self.pdf_to_page()
            .inverse()
            .expect("page transform is a rotation plus flip, always invertible")
    }
}

/// Number of pixels covering `points` at `scale` px/pt (at least 1).
pub fn pixel_extent(points: f64, scale: f64) -> u32 {
    (points * scale).round().max(1.0) as u32
}

fn normalize(b: PdfBox) -> PdfBox {
    PdfBox::new(
        PdfPoint::new(b.min.x.min(b.max.x), b.min.y.min(b.max.y)),
        PdfPoint::new(b.min.x.max(b.max.x), b.min.y.max(b.max.y)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pdf_box(x0: f64, y0: f64, x1: f64, y1: f64) -> PdfBox {
        PdfBox::new(PdfPoint::new(x0, y0), PdfPoint::new(x1, y1))
    }

    fn assert_near(a: PagePoint, b: (f64, f64)) {
        assert!(
            (a.x - b.0).abs() < 1e-9 && (a.y - b.1).abs() < 1e-9,
            "{a:?} != {b:?}"
        );
    }

    /// Letter page with a crop box offset from the origin.
    fn geometry(rotation: Rotation) -> PageGeometry {
        PageGeometry::new(
            pdf_box(0.0, 0.0, 612.0, 792.0),
            Some(pdf_box(100.0, 200.0, 400.0, 600.0)), // 300 × 400
            rotation,
        )
    }

    #[test]
    fn unrotated_maps_top_left_of_crop_box_to_origin() {
        let g = geometry(Rotation::None);
        assert_eq!(g.size(), PageSize::new(300.0, 400.0));
        let t = g.pdf_to_page();
        assert_near(t.transform_point(PdfPoint::new(100.0, 600.0)), (0.0, 0.0));
        assert_near(
            t.transform_point(PdfPoint::new(400.0, 200.0)),
            (300.0, 400.0),
        );
    }

    #[test]
    fn rotations_move_the_top_left_corner_clockwise() {
        // The crop box's top-left corner in PDF space.
        let tl = PdfPoint::new(100.0, 600.0);
        let cases = [
            (Rotation::None, (300.0, 400.0), (0.0, 0.0)),
            (Rotation::Cw90, (400.0, 300.0), (400.0, 0.0)),
            (Rotation::Cw180, (300.0, 400.0), (300.0, 400.0)),
            (Rotation::Cw270, (400.0, 300.0), (0.0, 300.0)),
        ];
        for (rot, size, tl_page) in cases {
            let g = geometry(rot);
            assert_eq!(g.size(), PageSize::new(size.0, size.1), "{rot:?}");
            assert_near(g.pdf_to_page().transform_point(tl), tl_page);
        }
    }

    #[test]
    fn page_to_pdf_round_trips() {
        for rot in [
            Rotation::None,
            Rotation::Cw90,
            Rotation::Cw180,
            Rotation::Cw270,
        ] {
            let g = geometry(rot);
            let p = PdfPoint::new(123.0, 456.0);
            let back = g
                .page_to_pdf()
                .transform_point(g.pdf_to_page().transform_point(p));
            assert!((back - p).length() < 1e-9, "{rot:?}");
        }
    }

    #[test]
    fn extra_rotation_composes() {
        let g = geometry(Rotation::Cw270).rotated(Rotation::Cw180);
        assert_eq!(g.rotation(), Rotation::Cw90);
    }

    #[test]
    fn crop_box_is_clipped_to_media_box_and_normalized() {
        let g = PageGeometry::new(
            pdf_box(0.0, 0.0, 200.0, 100.0),
            Some(pdf_box(250.0, 150.0, -50.0, 50.0)), // reversed corners, sticks out
            Rotation::None,
        );
        assert_eq!(g.visible_box(), pdf_box(0.0, 50.0, 200.0, 100.0));
        assert_eq!(g.size(), PageSize::new(200.0, 50.0));
    }

    #[test]
    fn degenerate_pages_fall_back_to_a4() {
        let g = PageGeometry::new(pdf_box(0.0, 0.0, 0.0, 0.0), None, Rotation::None);
        assert!((g.size().width - 595.28).abs() < 0.01);
        assert!((g.size().height - 841.89).abs() < 0.01);
    }

    #[test]
    fn pixel_size_rounds_and_is_never_zero() {
        let g = geometry(Rotation::None);
        assert_eq!(g.pixel_size(1.5), DeviceIntSize::new(450, 600));
        assert_eq!(g.pixel_size(0.0001), DeviceIntSize::new(1, 1));
    }
}
