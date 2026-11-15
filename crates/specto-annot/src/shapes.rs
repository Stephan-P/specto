//! What an annotation looks like, as a few simple shapes.
//!
//! Annotations are drawn in two places: on screen, by each app's toolkit,
//! and in the saved file, as an appearance stream that other programs
//! show. Both are made from the [`Shape`]s here, so that what is saved is
//! what was on screen.

use specto_geom::{PdfBox, PdfPoint, PdfQuad};

use crate::{AnnotKind, Annotation, Color, MarkupStyle};

/// One piece of an annotation's look, in the coordinates of its page of the
/// file.
#[derive(Clone, Copy, Debug, PartialEq)]
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
        _ => None,
    }
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
    for shape in &shapes {
        let (opacity, multiply) = match *shape {
            Shape::Fill {
                opacity, multiply, ..
            } => (opacity, multiply),
            Shape::Stroke { opacity, .. } => (opacity, false),
        };
        let state = GraphicsState { opacity, multiply };
        let index = states.iter().position(|s| *s == state).unwrap_or_else(|| {
            states.push(state);
            states.len() - 1
        });
        content.push_str(&format!("/GS{index} gs\n"));
        match *shape {
            Shape::Fill { quad, color, .. } => {
                grow(quad.bounds());
                let [a, b, c, d] = quad.points;
                content.push_str(&format!(
                    "{} {} {} rg\n{} {} m\n{} {} l\n{} {} l\n{} {} l\nh f\n",
                    n(color.r.into()),
                    n(color.g.into()),
                    n(color.b.into()),
                    n(a.x),
                    n(a.y),
                    n(b.x),
                    n(b.y),
                    n(c.x),
                    n(c.y),
                    n(d.x),
                    n(d.y),
                ));
            }
            Shape::Stroke {
                from,
                to,
                width,
                color,
                ..
            } => {
                grow(PdfBox::from_points([from, to]).inflate(width, width));
                content.push_str(&format!(
                    "{} {} {} RG\n{} w\n{} {} m\n{} {} l\nS\n",
                    n(color.r.into()),
                    n(color.g.into()),
                    n(color.b.into()),
                    n(width),
                    n(from.x),
                    n(from.y),
                    n(to.x),
                    n(to.y),
                ));
            }
        }
    }
    Some(AppearanceStream {
        bbox: bbox?,
        content: content.into_bytes(),
        states,
    })
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
