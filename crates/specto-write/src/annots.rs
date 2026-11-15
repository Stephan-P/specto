//! Writing annotations: what was added, changed and deleted on a page.

use std::collections::HashSet;

use lopdf::{Dictionary, Document, Object, ObjectId, Stream, dictionary};
use specto_annot::{AnnotKind, Annotation, Appearance, Color, MarkupStyle, appearance, look_mark};
use specto_doc::PageAnnotChanges;

use crate::{WriteError, text_string};

/// Applies `changes` to the `/Annots` of the page `page_id`, in `doc` as
/// loaded: object numbers are still the file's.
///
/// What is deleted or changed is named by where the import found it: its
/// index among the entries of the array, and its object number. If lopdf
/// has something else there (the two parsers read a damaged file
/// differently), nothing is written rather than the wrong annotation
/// changed.
pub(crate) fn apply(
    doc: &mut Document,
    page_id: ObjectId,
    changes: &PageAnnotChanges,
) -> Result<(), WriteError> {
    let page = doc.get_dictionary(page_id).map_err(WriteError::Read)?;
    let entries: Vec<Object> = match page.get(b"Annots") {
        Ok(Object::Array(a)) => a.clone(),
        Ok(Object::Reference(r)) => doc
            .get_object(*r)
            .and_then(Object::as_array)
            .cloned()
            .unwrap_or_default(),
        _ => Vec::new(),
    };

    let origins = changes
        .removed
        .iter()
        .chain(changes.replaced.iter().map(|(o, _)| o));
    for origin in origins {
        let same = match (entries.get(origin.index as usize), origin.object) {
            (Some(Object::Reference(id)), Some(object)) => *id == object,
            (Some(Object::Dictionary(_)), None) => true,
            _ => false,
        };
        if !same {
            return Err(WriteError::Unsupported);
        }
    }
    let removed: HashSet<u32> = changes.removed.iter().map(|o| o.index).collect();

    // The objects that go, and with them what hangs on them: their popups
    // and the replies to them.
    let mut gone: HashSet<ObjectId> = removed
        .iter()
        .filter_map(|&i| entries.get(i as usize)?.as_reference().ok())
        .collect();
    loop {
        let more: Vec<ObjectId> = entries
            .iter()
            .filter_map(|e| e.as_reference().ok())
            .filter(|id| !gone.contains(id))
            .filter(|id| {
                doc.get_dictionary(*id).is_ok_and(|d| {
                    [&b"Parent"[..], b"IRT"].iter().any(|key| {
                        d.get(key)
                            .and_then(Object::as_reference)
                            .is_ok_and(|r| gone.contains(&r))
                    })
                })
            })
            .collect();
        if more.is_empty() {
            break;
        }
        gone.extend(more);
    }

    let mut kept = Vec::with_capacity(entries.len() + changes.added.len());
    for (index, entry) in entries.into_iter().enumerate() {
        let index = index as u32;
        if removed.contains(&index) {
            continue;
        }
        if entry.as_reference().is_ok_and(|id| gone.contains(&id)) {
            continue;
        }
        let replacement = changes.replaced.iter().find(|(o, _)| o.index == index);
        let Some((_, annotation)) = replacement else {
            kept.push(entry);
            continue;
        };
        // The same object with its new values, so that what refers to it
        // (a popup, replies) still does, and what isn't known here stays.
        match entry {
            Object::Reference(id) => {
                let old = doc.get_dictionary(id).cloned().unwrap_or_default();
                let new = dictionary_of(doc, annotation, old, page_id);
                doc.objects.insert(id, Object::Dictionary(new));
                kept.push(Object::Reference(id));
            }
            Object::Dictionary(old) => {
                let new = dictionary_of(doc, annotation, old, page_id);
                kept.push(Object::Dictionary(new));
            }
            other => kept.push(other),
        }
    }
    for annotation in &changes.added {
        let dict = dictionary_of(doc, annotation, Dictionary::new(), page_id);
        kept.push(Object::Reference(doc.add_object(dict)));
    }

    let page = doc.get_dictionary_mut(page_id).map_err(WriteError::Read)?;
    if kept.is_empty() {
        page.remove(b"Annots");
    } else {
        page.set("Annots", kept);
    }
    Ok(())
}

/// The annotation dictionary for `a`, on top of `base` (what the file had
/// for it, or nothing).
fn dictionary_of(
    doc: &mut Document,
    a: &Annotation,
    base: Dictionary,
    page: ObjectId,
) -> Dictionary {
    let mut d = base;
    d.set("Type", "Annot");
    d.set("P", page);
    d.set("F", i64::from(a.flags.0));
    let subtype: &[u8] = match &a.kind {
        AnnotKind::TextMarkup { style, .. } => match style {
            MarkupStyle::Highlight => b"Highlight",
            MarkupStyle::Underline => b"Underline",
            MarkupStyle::StrikeOut => b"StrikeOut",
            MarkupStyle::Squiggly => b"Squiggly",
        },
        AnnotKind::Ink { .. } => b"Ink",
        AnnotKind::Square { .. } => b"Square",
        AnnotKind::Circle { .. } => b"Circle",
        AnnotKind::Line { .. } => b"Line",
        AnnotKind::Stamp(_) => b"Stamp",
        AnnotKind::Other { subtype } => subtype.as_bytes(),
    };
    d.set("Subtype", Object::Name(subtype.to_vec()));
    let mut rect = a.rect;

    match &a.kind {
        AnnotKind::TextMarkup { quads, .. } => {
            // In a Z (top left, top right, bottom left, bottom right), as
            // every program writes them, whatever the standard says.
            let numbers = quads
                .iter()
                .flat_map(|q| [q.points[3], q.points[2], q.points[0], q.points[1]])
                .flat_map(|p| [real(p.x), real(p.y)])
                .collect::<Vec<_>>();
            d.set("QuadPoints", numbers);
        }
        AnnotKind::Ink { strokes } => {
            let list = strokes
                .iter()
                .map(|s| Object::Array(s.iter().flat_map(|p| [real(p.x), real(p.y)]).collect()))
                .collect::<Vec<_>>();
            d.set("InkList", list);
        }
        AnnotKind::Square { fill } | AnnotKind::Circle { fill } => match fill {
            Some(c) => d.set("IC", color(*c)),
            None => {
                d.remove(b"IC");
            }
        },
        AnnotKind::Line { from, to } => {
            d.set(
                "L",
                vec![real(from.x), real(from.y), real(to.x), real(to.y)],
            );
        }
        AnnotKind::Stamp(_) | AnnotKind::Other { .. } => {}
    }
    match a.color {
        Some(c) => d.set("C", color(c)),
        None => {
            d.remove(b"C");
        }
    }
    if a.opacity < 1.0 {
        d.set("CA", a.opacity);
    } else {
        d.remove(b"CA");
    }
    if !matches!(
        a.kind,
        AnnotKind::TextMarkup { .. } | AnnotKind::Other { .. }
    ) {
        d.set("BS", dictionary! { "W" => real(a.line_width) });
        d.remove(b"Border");
    }
    for (key, value) in [("T", &a.author), ("Contents", &a.contents), ("NM", &a.name)] {
        match value {
            Some(v) => d.set(key, text_string(v)),
            None => {
                d.remove(key.as_bytes());
            }
        }
    }
    match a.modified {
        Some(m) => d.set("M", Object::string_literal(pdf_date(m))),
        None => {
            d.remove(b"M");
        }
    }

    // Its look for other programs: as it was where nothing changed, made
    // anew where something did, and none where nothing here makes one
    // (readers then draw it from the values above, if they do).
    if a.appearance == Appearance::Generated {
        match appearance(a) {
            Some(ap) => {
                rect = ap.bbox;
                let states: Dictionary = ap
                    .states
                    .iter()
                    .enumerate()
                    .map(|(i, s)| {
                        let mut gs = dictionary! {
                            "Type" => "ExtGState",
                            "CA" => s.opacity,
                            "ca" => s.opacity,
                        };
                        if s.multiply {
                            gs.set("BM", "Multiply");
                        }
                        (format!("GS{i}"), Object::Dictionary(gs))
                    })
                    .collect();
                let form = dictionary! {
                    "Type" => "XObject",
                    "Subtype" => "Form",
                    "BBox" => vec![
                        real(rect.min.x), real(rect.min.y), real(rect.max.x), real(rect.max.y),
                    ],
                    "Resources" => dictionary! { "ExtGState" => states },
                };
                // Marked as written here, so that it can be made anew
                // another day without wondering what else it drew.
                d.set("SpectoLook", look_mark(&ap.content));
                let id = doc.add_object(Stream::new(form, ap.content));
                d.set("AP", dictionary! { "N" => id });
                d.remove(b"AS");
            }
            None => {
                d.remove(b"AP");
                d.remove(b"AS");
                d.remove(b"SpectoLook");
            }
        }
    }
    d.set(
        "Rect",
        vec![
            real(rect.min.x),
            real(rect.min.y),
            real(rect.max.x),
            real(rect.max.y),
        ],
    );
    d
}

fn real(v: f64) -> Object {
    // Three decimals are a thousandth of a point; f32 is what lopdf holds.
    Object::Real(((v * 1000.0).round() / 1000.0) as f32)
}

fn color(c: Color) -> Object {
    Object::Array(vec![c.r.into(), c.g.into(), c.b.into()])
}

/// `D:YYYYMMDDHHmmSS+HH'mm'`.
fn pdf_date(d: specto_annot::Date) -> String {
    let offset = d.utc_offset_minutes;
    format!(
        "D:{:04}{:02}{:02}{:02}{:02}{:02}{}{:02}'{:02}'",
        d.year,
        d.month,
        d.day,
        d.hour,
        d.minute,
        d.second,
        if offset < 0 { '-' } else { '+' },
        offset.unsigned_abs() / 60,
        offset.unsigned_abs() % 60,
    )
}
