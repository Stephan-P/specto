//! What an annotation looks like, as a few simple shapes.
//!
//! Annotations are drawn in two places: on screen, by each app's toolkit,
//! and in the saved file, as an appearance stream that other programs
//! show. Both are made from the [`Shape`]s here, so that what is saved is
//! what was on screen.

use specto_geom::euclid::Point2D;
use specto_geom::{PdfBox, PdfPoint, PdfQuad, PdfSpace};

use crate::{AnnotKind, Annotation, Color, MarkupStyle};

/// An outline of straight lines and curves, in the coordinates `U`.
#[derive(Clone, Debug, PartialEq)]
pub struct Path<U> {
    pub start: Point2D<f64, U>,
    pub segments: Vec<Segment<U>>,
    /// Whether it ends where it starts, with a line back if need be.
    pub closed: bool,
}

/// A piece of a [`Path`], from where the one before ended.
#[derive(Debug, PartialEq)]
pub enum Segment<U> {
    Line(Point2D<f64, U>),
    /// A cubic Bézier curve: two control points and the end.
    Curve(Point2D<f64, U>, Point2D<f64, U>, Point2D<f64, U>),
}

impl<U> Clone for Segment<U> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<U> Copy for Segment<U> {}

impl<U> Path<U> {
    /// The path with every point put through `f`; `None` if `f` gives none
    /// for one of them.
    pub fn map<V>(
        &self,
        f: impl Fn(Point2D<f64, U>) -> Option<Point2D<f64, V>>,
    ) -> Option<Path<V>> {
        Some(Path {
            start: f(self.start)?,
            segments: self
                .segments
                .iter()
                .map(|s| {
                    Some(match *s {
                        Segment::Line(p) => Segment::Line(f(p)?),
                        Segment::Curve(a, b, p) => Segment::Curve(f(a)?, f(b)?, f(p)?),
                    })
                })
                .collect::<Option<_>>()?,
            closed: self.closed,
        })
    }

    /// Every point of the path, control points included: they bound it.
    pub fn points(&self) -> impl Iterator<Item = Point2D<f64, U>> + '_ {
        std::iter::once(self.start).chain(self.segments.iter().flat_map(|s| match *s {
            Segment::Line(p) => vec![p],
            Segment::Curve(a, b, p) => vec![a, b, p],
        }))
    }
}

/// One piece of an annotation's look, in the coordinates of its page of the
/// file.
#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    /// A filled quadrilateral.
    Fill {
        quad: PdfQuad,
        color: Color,
        opacity: f32,
        /// Darkens what is beneath instead of covering it, as a highlighter
        /// pen does: text under it stays black.
        multiply: bool,
    },
    /// A straight line with square ends.
    Stroke {
        from: PdfPoint,
        to: PdfPoint,
        width: f64,
        color: Color,
        opacity: f32,
    },
    /// An outline, filled, drawn with a pen of a width with round ends and
    /// corners, or both.
    Path {
        path: Path<PdfSpace>,
        fill: Option<Color>,
        stroke: Option<(Color, f64)>,
        opacity: f32,
    },
}

/// The colour of markup that has none.
const DEFAULT: Color = Color::new(1.0, 0.92, 0.23);

/// The shapes that draw `annotation`, bottom to top; `None` for kinds that
/// are only ever shown as their file draws them, or that nothing here
/// draws yet.
pub fn shapes(annotation: &Annotation) -> Option<Vec<Shape>> {
    let color = annotation.color.unwrap_or(DEFAULT);
    let opacity = annotation.opacity;
    match &annotation.kind {
        AnnotKind::TextMarkup { style, quads } => Some(
            quads
                .iter()
                .map(|&quad| match style {
                    MarkupStyle::Highlight => Shape::Fill {
                        quad,
                        color,
                        opacity,
                        multiply: true,
                    },
                    // A wavy line isn't drawn yet; a straight one stands in.
                    MarkupStyle::Underline | MarkupStyle::Squiggly => {
                        line_across(&quad, 0.08, color, opacity)
                    }
                    MarkupStyle::StrikeOut => line_across(&quad, 0.4, color, opacity),
                })
                .collect(),
        ),
        AnnotKind::Ink { strokes } => Some(
            strokes
                .iter()
                .filter_map(|points| smooth(points))
                .map(|path| Shape::Path {
                    path,
                    fill: None,
                    stroke: Some((color, annotation.line_width)),
                    opacity,
                })
                .collect(),
        ),
        AnnotKind::Square { fill } => {
            let r = outline(annotation);
            let corner = |x, y| PdfPoint::new(x, y);
            Some(vec![Shape::Path {
                path: Path {
                    start: r.min,
                    segments: vec![
                        Segment::Line(corner(r.max.x, r.min.y)),
                        Segment::Line(r.max),
                        Segment::Line(corner(r.min.x, r.max.y)),
                    ],
                    closed: true,
                },
                fill: *fill,
                stroke: stroke_of(annotation),
                opacity,
            }])
        }
        AnnotKind::Circle { fill } => Some(vec![Shape::Path {
            path: ellipse(outline(annotation)),
            fill: *fill,
            stroke: stroke_of(annotation),
            opacity,
        }]),
        AnnotKind::Line { from, to } => Some(vec![Shape::Path {
            path: Path {
                start: *from,
                segments: vec![Segment::Line(*to)],
                closed: false,
            },
            fill: None,
            stroke: Some((color, annotation.line_width.max(0.1))),
            opacity,
        }]),
        AnnotKind::Stamp(_) | AnnotKind::Other { .. } => None,
    }
}

/// The pen of a rectangle or ellipse: none if its width is 0 (it is then
/// only filled) or it has no colour of its own.
fn stroke_of(annotation: &Annotation) -> Option<(Color, f64)> {
    let color = annotation.color?;
    (annotation.line_width > 0.0).then_some((color, annotation.line_width))
}

/// Where the pen of a rectangle or ellipse runs: inside the annotation's
/// rectangle by half its width, so that the line is within it.
fn outline(annotation: &Annotation) -> PdfBox {
    let r = annotation.rect;
    let inset = (annotation.line_width / 2.0)
        .min(r.width() / 2.0)
        .min(r.height() / 2.0)
        .max(0.0);
    r.inflate(-inset, -inset)
}

/// An ellipse touching the sides of `r`, as four curves.
fn ellipse(r: PdfBox) -> Path<PdfSpace> {
    // How far the control points of a quarter circle are from its ends, for
    // a radius of 1.
    const K: f64 = 0.552_284_749_8;
    let c = r.center();
    let (rx, ry) = (r.width() / 2.0, r.height() / 2.0);
    let p = |x: f64, y: f64| PdfPoint::new(c.x + x * rx, c.y + y * ry);
    Path {
        start: p(1.0, 0.0),
        segments: vec![
            Segment::Curve(p(1.0, K), p(K, 1.0), p(0.0, 1.0)),
            Segment::Curve(p(-K, 1.0), p(-1.0, K), p(-1.0, 0.0)),
            Segment::Curve(p(-1.0, -K), p(-K, -1.0), p(0.0, -1.0)),
            Segment::Curve(p(K, -1.0), p(1.0, -K), p(1.0, 0.0)),
        ],
        closed: true,
    }
}

/// A smooth curve through the points of a freehand stroke (a Catmull-Rom
/// spline, as Bézier curves); a single point becomes a dot. `None` without
/// points.
fn smooth(points: &[PdfPoint]) -> Option<Path<PdfSpace>> {
    let (&start, rest) = points.split_first()?;
    let segments = match rest {
        // A line of no length: with round ends, a dot.
        [] => vec![Segment::Line(start)],
        [end] => vec![Segment::Line(*end)],
        _ => (0..points.len() - 1)
            .map(|i| {
                let before = points[i.saturating_sub(1)];
                let (from, to) = (points[i], points[i + 1]);
                let after = points[(i + 2).min(points.len() - 1)];
                Segment::Curve(from + (to - before) / 6.0, to - (after - from) / 6.0, to)
            })
            .collect(),
    };
    Some(Path {
        start,
        segments,
        closed: false,
    })
}

/// A line along the text of `quad` (bottom left, bottom right, top right,
/// top left in the text's direction), `height` of the way up from its
/// bottom edge.
fn line_across(quad: &PdfQuad, height: f64, color: Color, opacity: f32) -> Shape {
    let [bl, br, tr, tl] = quad.points;
    let from = bl + (tl - bl) * height;
    let to = br + (tr - br) * height;
    let line_height = (tl - bl).length().max((tr - br).length());
    Shape::Stroke {
        from,
        to,
        width: (line_height / 14.0).max(0.5),
        color,
        opacity,
    }
}

/// An annotation's look as a PDF appearance stream: a form whose content
/// is in page coordinates, so its bounding box is the annotation's
/// rectangle and it needs no matrix.
#[derive(Clone, Debug, PartialEq)]
pub struct AppearanceStream {
    /// The form's `/BBox`, and the annotation's `/Rect` to go with it: the
    /// shapes' bounds, which can reach past the annotation's own rectangle
    /// (the width of a line).
    pub bbox: PdfBox,
    /// The content stream. It selects the graphics state `/GS0`, …, one
    /// per entry of `states`.
    pub content: Vec<u8>,
    /// The graphics states the content uses.
    pub states: Vec<GraphicsState>,
}

/// An `ExtGState`: how what follows is painted over what is beneath.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GraphicsState {
    pub opacity: f32,
    pub multiply: bool,
}

/// The appearance stream for `annotation`, or `None` where [`shapes`]
/// gives none.
pub fn appearance(annotation: &Annotation) -> Option<AppearanceStream> {
    let shapes = shapes(annotation)?;
    let mut bbox: Option<PdfBox> = None;
    let mut grow = |b: PdfBox| bbox = Some(bbox.map_or(b, |a| a.union(&b)));
    let mut states: Vec<GraphicsState> = Vec::new();
    let mut content = String::new();
    let rgb = |c: Color| format!("{} {} {}", n(c.r.into()), n(c.g.into()), n(c.b.into()));
    let xy = |p: PdfPoint| format!("{} {}", n(p.x), n(p.y));
    for shape in &shapes {
        let (opacity, multiply) = match *shape {
            Shape::Fill {
                opacity, multiply, ..
            } => (opacity, multiply),
            Shape::Stroke { opacity, .. } | Shape::Path { opacity, .. } => (opacity, false),
        };
        let state = GraphicsState { opacity, multiply };
        let index = states.iter().position(|s| *s == state).unwrap_or_else(|| {
            states.push(state);
            states.len() - 1
        });
        content.push_str(&format!("/GS{index} gs\n"));
        match shape {
            Shape::Fill { quad, color, .. } => {
                grow(quad.bounds());
                let [a, b, c, d] = quad.points;
                content.push_str(&format!(
                    "{} rg\n{} m\n{} l\n{} l\n{} l\nh f\n",
                    rgb(*color),
                    xy(a),
                    xy(b),
                    xy(c),
                    xy(d),
                ));
            }
            Shape::Stroke {
                from,
                to,
                width,
                color,
                ..
            } => {
                grow(PdfBox::from_points([*from, *to]).inflate(*width, *width));
                content.push_str(&format!(
                    "{} RG\n{} w\n{} m\n{} l\nS\n",
                    rgb(*color),
                    n(*width),
                    xy(*from),
                    xy(*to),
                ));
            }
            Shape::Path {
                path, fill, stroke, ..
            } => {
                // A round pen reaches half its width beyond the outline, no
                // further: the area of a rectangle or ellipse must come out
                // as it went in, or it would grow with every save.
                let margin = stroke.map_or(0.0, |(_, width)| width / 2.0);
                grow(PdfBox::from_points(path.points()).inflate(margin, margin));
                if let Some(fill) = fill {
                    content.push_str(&format!("{} rg\n", rgb(*fill)));
                }
                if let Some((color, width)) = stroke {
                    // Round ends and corners.
                    content.push_str(&format!("{} RG\n{} w\n1 J\n1 j\n", rgb(*color), n(*width)));
                }
                content.push_str(&format!("{} m\n", xy(path.start)));
                for segment in &path.segments {
                    match *segment {
                        Segment::Line(p) => content.push_str(&format!("{} l\n", xy(p))),
                        Segment::Curve(a, b, p) => {
                            content.push_str(&format!("{} {} {} c\n", xy(a), xy(b), xy(p)));
                        }
                    }
                }
                if path.closed {
                    content.push_str("h\n");
                }
                content.push_str(match (fill.is_some(), stroke.is_some()) {
                    (true, true) => "B\n",
                    (true, false) => "f\n",
                    (false, true) => "S\n",
                    (false, false) => "n\n",
                });
            }
        }
    }
    Some(AppearanceStream {
        bbox: bbox?,
        content: content.into_bytes(),
        states,
    })
}

/// A number that says "this appearance stream was written here": kept in
/// the annotation (`/SpectoLook`) next to the stream, and compared with the
/// stream when the file is read. As long as they agree, the look is the
/// one [`appearance`] made, and nothing is lost by making it anew. Another
/// program that changes the look changes the stream, and the two no longer
/// agree. (FNV-1a over the stream's content.)
pub fn look_mark(content: &[u8]) -> i64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in content {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash as i64
}

/// A number as content streams want it: no exponent, no needless digits.
fn n(v: f64) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-0" {
        "0".to_owned()
    } else {
        s.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::n;

    #[test]
    fn numbers() {
        assert_eq!(n(1.0), "1");
        assert_eq!(n(0.5), "0.5");
        assert_eq!(n(-0.0001), "0");
        assert_eq!(n(100.0), "100");
        assert_eq!(n(12.3456), "12.346");
        assert_eq!(n(1e-9), "0");
    }
}
