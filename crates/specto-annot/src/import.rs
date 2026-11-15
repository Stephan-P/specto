//! Reading the annotations a file already has.

use specto_geom::{PdfBox, PdfPoint, PdfQuad};
use specto_source::hayro_syntax::object::{
    Array, Dict, MaybeRef, Name, Object, Stream, String as PdfString,
};
use specto_source::{Date, SourceDocument, decode_text};

use crate::{AnnotKind, Annotation, Appearance, Color, Flags, MarkupStyle, Origin, look_mark};

/// Entries of a page's `/Annots` beyond this many are ignored, as are
/// quads, strokes and points of one annotation: a guard against broken
/// files.
const MAX_ITEMS: usize = 10_000;

/// The markup annotations of page `page` of `source`, in the order the
/// file draws them (later ones on top). What isn't markup (links, form
/// fields, popups, media) is left out, and so is anything without a usable
/// `/Rect`.
pub fn import(source: &SourceDocument, page: usize) -> Vec<Annotation> {
    let Some(p) = source.pdf().pages().get(page) else {
        return Vec::new();
    };
    let Some(annots) = p.raw().get::<Array<'_>>(b"Annots") else {
        return Vec::new();
    };
    // The raw entries say which are references, and count the entries that
    // aren't dictionaries; the resolved ones are the dictionaries.
    annots
        .raw_iter()
        .zip(annots.iter::<Object<'_>>())
        .take(MAX_ITEMS)
        .enumerate()
        .filter_map(|(index, (raw, object))| {
            let Object::Dict(dict) = object else {
                return None;
            };
            let object = match raw {
                MaybeRef::Ref(r) => Some((r.obj_number, u16::try_from(r.gen_number).ok()?)),
                MaybeRef::NotRef(_) => None,
            };
            let origin = Origin {
                index: u32::try_from(index).ok()?,
                object,
            };
            annotation(&dict, origin)
        })
        .collect()
}

fn annotation(dict: &Dict<'_>, origin: Origin) -> Option<Annotation> {
    let subtype = dict.get::<Name<'_>>(b"Subtype")?;
    let subtype = subtype.as_ref();
    // Not markup: links and form fields have their own crates, a popup is
    // the window of another annotation, and the rest are media and marks
    // for printers.
    if matches!(
        subtype,
        b"Link"
            | b"Widget"
            | b"Popup"
            | b"Screen"
            | b"Movie"
            | b"Sound"
            | b"RichMedia"
            | b"3D"
            | b"PrinterMark"
            | b"TrapNet"
            | b"Watermark"
    ) {
        return None;
    }
    let rect = rect(dict)?;
    let color = color_of(dict, b"C");
    let fill = || color_of(dict, b"IC");
    let kind = match subtype {
        b"Highlight" => markup(dict, MarkupStyle::Highlight),
        b"Underline" => markup(dict, MarkupStyle::Underline),
        b"StrikeOut" => markup(dict, MarkupStyle::StrikeOut),
        b"Squiggly" => markup(dict, MarkupStyle::Squiggly),
        b"Ink" => ink(dict),
        b"Square" => Some(AnnotKind::Square { fill: fill() }),
        b"Circle" => Some(AnnotKind::Circle { fill: fill() }),
        b"Line" => line(dict),
        _ => None,
    }
    // One of ours without what it needs is still shown as the file has it.
    .unwrap_or_else(|| AnnotKind::Other {
        subtype: String::from_utf8_lossy(subtype).into_owned(),
    });
    let text = |key: &[u8]| {
        dict.get::<PdfString<'_>>(key)
            .map(|s| decode_text(s.as_bytes()))
            .filter(|s| !s.is_empty())
    };
    Some(Annotation {
        kind,
        rect,
        color,
        opacity: dict
            .get::<f32>(b"CA")
            .filter(|a| a.is_finite())
            .map_or(1.0, |a| a.clamp(0.0, 1.0)),
        line_width: line_width(dict),
        author: text(b"T"),
        contents: text(b"Contents"),
        modified: dict
            .get::<PdfString<'_>>(b"M")
            .and_then(|s| Date::parse(s.as_bytes())),
        name: text(b"NM"),
        flags: Flags(dict.get::<u32>(b"F").unwrap_or(0)),
        origin: Some(origin),
        appearance: appearance_of(dict),
    })
}

/// What the file gives the annotation as its normal appearance (`/AP
/// /N`): nothing, a look written here (the stream is the one `/SpectoLook`
/// marks, see [`look_mark`]), or one of unknown making. A dictionary of
/// looks by state is never one of ours.
fn appearance_of(dict: &Dict<'_>) -> Appearance {
    let Some(ap) = dict.get::<Dict<'_>>(b"AP") else {
        return Appearance::Missing;
    };
    if !ap.contains_key(b"N") {
        return Appearance::Missing;
    }
    let ours = (|| {
        let mark = dict.get::<i64>(b"SpectoLook")?;
        let content = ap.get::<Stream<'_>>(b"N")?.decoded().ok()?;
        Some(look_mark(&content) == mark)
    })();
    if ours == Some(true) {
        Appearance::Own
    } else {
        Appearance::Original
    }
}

/// The finite numbers of the array at `key`, up to a limit.
fn numbers(dict: &Dict<'_>, key: &[u8]) -> Option<Vec<f64>> {
    Some(numbers_of(&dict.get::<Array<'_>>(key)?))
}

fn numbers_of(array: &Array<'_>) -> Vec<f64> {
    array
        .iter::<f64>()
        .take(8 * MAX_ITEMS)
        .filter(|n| n.is_finite())
        .collect()
}

fn rect(dict: &Dict<'_>) -> Option<PdfBox> {
    let [x0, y0, x1, y1] = numbers(dict, b"Rect")?[..] else {
        return None;
    };
    Some(PdfBox::new(
        PdfPoint::new(x0.min(x1), y0.min(y1)),
        PdfPoint::new(x0.max(x1), y0.max(y1)),
    ))
}

/// `/C` and `/IC`: grey, RGB or CMYK by the number of components; an
/// empty array means none.
fn color_of(dict: &Dict<'_>, key: &[u8]) -> Option<Color> {
    let c: Vec<f32> = numbers(dict, key)?
        .iter()
        .map(|&v| (v as f32).clamp(0.0, 1.0))
        .collect();
    match c[..] {
        [g] => Some(Color::new(g, g, g)),
        [r, g, b] => Some(Color::new(r, g, b)),
        [c, m, y, k] => Some(Color::new(
            (1.0 - c) * (1.0 - k),
            (1.0 - m) * (1.0 - k),
            (1.0 - y) * (1.0 - k),
        )),
        _ => None,
    }
}

/// The width of lines: `/BS /W`, else the third number of `/Border`, else 1.
fn line_width(dict: &Dict<'_>) -> f64 {
    dict.get::<Dict<'_>>(b"BS")
        .and_then(|bs| bs.get::<f64>(b"W"))
        .or_else(|| numbers(dict, b"Border")?.get(2).copied())
        .filter(|w| w.is_finite() && *w >= 0.0)
        .unwrap_or(1.0)
}

/// `/QuadPoints`: eight numbers per quad.
fn markup(dict: &Dict<'_>, style: MarkupStyle) -> Option<AnnotKind> {
    let numbers = numbers(dict, b"QuadPoints")?;
    let (eights, _rest) = numbers.as_chunks::<8>();
    let quads: Vec<PdfQuad> = eights.iter().map(quad).collect();
    (!quads.is_empty()).then_some(AnnotKind::TextMarkup { style, quads })
}

/// The standard asks for the corners counterclockwise, and nearly every
/// program writes them in a Z (top left, top right, bottom left, bottom
/// right). Both are read here by going round the corners by angle, which
/// gives a proper quad for either, starting from the first corner given.
fn quad(n: &[f64; 8]) -> PdfQuad {
    let mut points = [0, 2, 4, 6].map(|i| PdfPoint::new(n[i], n[i + 1]));
    let cx = points.iter().map(|p| p.x).sum::<f64>() / 4.0;
    let cy = points.iter().map(|p| p.y).sum::<f64>() / 4.0;
    points.sort_by(|a, b| {
        let angle = |p: &PdfPoint| (p.y - cy).atan2(p.x - cx);
        angle(a).total_cmp(&angle(b))
    });
    PdfQuad::new(points)
}

/// `/InkList`: an array of strokes, each an array of x and y in turn.
fn ink(dict: &Dict<'_>) -> Option<AnnotKind> {
    let strokes: Vec<Vec<PdfPoint>> = dict
        .get::<Array<'_>>(b"InkList")?
        .iter::<Array<'_>>()
        .take(MAX_ITEMS)
        .map(|stroke| {
            let numbers = numbers_of(&stroke);
            let (pairs, _rest) = numbers.as_chunks::<2>();
            pairs.iter().map(|&[x, y]| PdfPoint::new(x, y)).collect()
        })
        .filter(|s: &Vec<PdfPoint>| !s.is_empty())
        .collect();
    (!strokes.is_empty()).then_some(AnnotKind::Ink { strokes })
}

/// `/L`: where the line starts and ends.
fn line(dict: &Dict<'_>) -> Option<AnnotKind> {
    let [x0, y0, x1, y1] = numbers(dict, b"L")?[..] else {
        return None;
    };
    Some(AnnotKind::Line {
        from: PdfPoint::new(x0, y0),
        to: PdfPoint::new(x1, y1),
    })
}
