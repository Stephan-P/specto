//! Reading the fields of a form.

use std::collections::{HashMap, HashSet};

use specto_annot::Color;
use specto_geom::{PdfBox, PdfPoint};
use specto_source::hayro_syntax::object::{
    Array, Dict, MaybeRef, Name, Object, ObjectIdentifier, String as PdfString,
};
use specto_source::{SourceDocument, decode_text};

use crate::{Field, FieldId, FieldKind, FieldValue, TextLook, Widget};

/// Fields beyond this many are ignored, and so are levels of the tree beyond `MAX_DEPTH`: a guard
/// against huge or cyclic trees in broken files.
const MAX_FIELDS: usize = 20_000;
const MAX_DEPTH: usize = 16;

// Field flags (`/Ff`), as bits from 1.
const READ_ONLY: u32 = 1;
const REQUIRED: u32 = 1 << 1;
const MULTILINE: u32 = 1 << 12;
const PASSWORD: u32 = 1 << 13;
const RADIO: u32 = 1 << 15;
const PUSH_BUTTON: u32 = 1 << 16;

/// What a field takes from those above it in the tree.
#[derive(Clone, Default)]
struct Inherited {
    name: String,
    kind: Option<Vec<u8>>,
    flags: Option<u32>,
    look: Option<String>,
    alignment: Option<u8>,
}

/// The fields of `source`'s form, in the order of the file's field tree. Empty if it has no form.
pub fn fields(source: &SourceDocument) -> Vec<Field> {
    let Some(form) = source
        .catalog()
        .and_then(|c| c.get::<Dict<'_>>(b"AcroForm"))
    else {
        return Vec::new();
    };
    let Some(roots) = form.get::<Array<'_>>(b"Fields") else {
        return Vec::new();
    };
    let pages = widget_pages(source);
    let top = Inherited {
        look: text(&form, b"DA"),
        alignment: form.get::<u32>(b"Q").map(|q| q.min(2) as u8),
        ..Default::default()
    };
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for (node, id) in children(&roots) {
        walk(&node, id, &top, 0, &pages, &mut seen, &mut out);
    }
    out
}

/// The dictionaries an array refers to, each with its object's number. Entries written straight
/// into the array have none and are left out: a field must be nameable.
fn children<'a>(array: &Array<'a>) -> Vec<(Dict<'a>, FieldId)> {
    array
        .raw_iter()
        .zip(array.iter::<Object<'a>>())
        .take(MAX_FIELDS)
        .filter_map(|(raw, object)| {
            let (MaybeRef::Ref(r), Object::Dict(dict)) = (raw, object) else {
                return None;
            };
            Some((
                dict,
                FieldId(r.obj_number, u16::try_from(r.gen_number).ok()?),
            ))
        })
        .collect()
}

/// Which page each annotation object is on, and where in that page's `/Annots`.
fn widget_pages(source: &SourceDocument) -> HashMap<ObjectIdentifier, usize> {
    let mut map = HashMap::new();
    for (page, p) in source.pdf().pages().iter().enumerate() {
        let Some(annots) = p.raw().get::<Array<'_>>(b"Annots") else {
            continue;
        };
        for entry in annots.raw_iter().take(MAX_FIELDS) {
            if let MaybeRef::Ref(r) = entry {
                map.entry(ObjectIdentifier::from(r)).or_insert(page);
            }
        }
    }
    map
}

fn text(dict: &Dict<'_>, key: &[u8]) -> Option<String> {
    dict.get::<PdfString<'_>>(key)
        .map(|s| decode_text(s.as_bytes()))
}

fn is_widget(dict: &Dict<'_>) -> bool {
    dict.get::<Name<'_>>(b"Subtype").as_deref() == Some(b"Widget")
}

#[allow(clippy::too_many_arguments)]
fn walk(
    node: &Dict<'_>,
    id: FieldId,
    above: &Inherited,
    depth: usize,
    pages: &HashMap<ObjectIdentifier, usize>,
    seen: &mut HashSet<FieldId>,
    out: &mut Vec<Field>,
) {
    if depth > MAX_DEPTH || out.len() >= MAX_FIELDS || !seen.insert(id) {
        return;
    }
    let own_name = text(node, b"T");
    let here = Inherited {
        name: match (&own_name, above.name.is_empty()) {
            (Some(n), true) => n.clone(),
            (Some(n), false) => format!("{}.{n}", above.name),
            (None, _) => above.name.clone(),
        },
        kind: node
            .get::<Name<'_>>(b"FT")
            .map(|n| n.as_ref().to_vec())
            .or_else(|| above.kind.clone()),
        flags: node.get::<u32>(b"Ff").or(above.flags),
        look: text(node, b"DA").or_else(|| above.look.clone()),
        alignment: node
            .get::<u32>(b"Q")
            .map(|q| q.min(2) as u8)
            .or(above.alignment),
    };
    let kids = node
        .get::<Array<'_>>(b"Kids")
        .map(|k| children(&k))
        .unwrap_or_default();
    // Fields below it: kids with names of their own. Kids without are its widgets.
    let (below, widget_kids): (Vec<_>, Vec<_>) =
        kids.into_iter().partition(|(k, _)| k.contains_key(b"T"));
    if !below.is_empty() {
        for (kid, kid_id) in below {
            walk(&kid, kid_id, &here, depth + 1, pages, seen, out);
        }
        return;
    }
    // A field with a value: one that says what kind it is, by itself or from above.
    let Some(kind_name) = &here.kind else {
        return;
    };
    let flags = here.flags.unwrap_or(0);
    let kind = match kind_name.as_slice() {
        b"Tx" => FieldKind::Text {
            multiline: flags & MULTILINE != 0,
            password: flags & PASSWORD != 0,
            max_length: node.get::<u32>(b"MaxLen"),
        },
        b"Btn" if flags & PUSH_BUTTON != 0 => FieldKind::Button,
        b"Btn" if flags & RADIO != 0 => FieldKind::Radio,
        b"Btn" => FieldKind::Checkbox,
        b"Ch" => FieldKind::Choice,
        b"Sig" => FieldKind::Signature,
        _ => FieldKind::Other,
    };
    let mut widgets: Vec<Widget> = widget_kids
        .iter()
        .filter(|(k, _)| is_widget(k) || k.contains_key(b"Rect"))
        .filter_map(|(k, kid_id)| widget(k, *kid_id, pages))
        .collect();
    // Without kids, the field is its own widget.
    if widgets.is_empty() && (is_widget(node) || node.contains_key(b"Rect")) {
        widgets.extend(widget(node, id, pages));
    }
    // What it holds: the name of the state that is on, for buttons, and text for text fields.
    let state = node
        .get::<Name<'_>>(b"V")
        .map(|n| String::from_utf8_lossy(n.as_ref()).into_owned())
        .filter(|s| s != "Off");
    let value = match kind {
        FieldKind::Text { .. } => Some(FieldValue::Text(text(node, b"V").unwrap_or_default())),
        FieldKind::Checkbox => Some(FieldValue::Check(state.is_some())),
        FieldKind::Radio => Some(FieldValue::Choice(state)),
        _ => None,
    };
    out.push(Field {
        id,
        name: here.name.clone(),
        label: text(node, b"TU").filter(|t| !t.is_empty()),
        kind,
        read_only: flags & READ_ONLY != 0,
        required: flags & REQUIRED != 0,
        value,
        look: text_look(here.look.as_deref(), here.alignment),
        widgets,
    });
}

/// The widget in `dict`, if it has an area and is on a page.
fn widget(
    dict: &Dict<'_>,
    id: FieldId,
    pages: &HashMap<ObjectIdentifier, usize>,
) -> Option<Widget> {
    let numbers: Vec<f64> = dict
        .get::<Array<'_>>(b"Rect")?
        .iter::<f64>()
        .filter(|n| n.is_finite())
        .collect();
    let [x0, y0, x1, y1] = numbers[..] else {
        return None;
    };
    let page = *pages.get(&ObjectIdentifier::new(id.0, i32::from(id.1)))?;
    // What its states are called: those it has a look for, other than "Off".
    let on_state = dict
        .get::<Dict<'_>>(b"AP")
        .and_then(|ap| ap.get::<Dict<'_>>(b"N"))
        .and_then(|states| {
            states
                .keys()
                .map(|k| String::from_utf8_lossy(k.as_ref()).into_owned())
                .find(|k| k != "Off")
        });
    let flags = dict.get::<u32>(b"F").unwrap_or(0);
    Some(Widget {
        page,
        rect: PdfBox::new(
            PdfPoint::new(x0.min(x1), y0.min(y1)),
            PdfPoint::new(x0.max(x1), y0.max(y1)),
        ),
        object: (id.0, id.1),
        on_state,
        // Hidden, or not to be viewed.
        hidden: flags & ((1 << 1) | (1 << 5)) != 0,
    })
}

/// How a text field sets its text, from its default appearance string: `/Font size Tf` and a
/// color (`g`, `rg` or `k`).
fn text_look(da: Option<&str>, alignment: Option<u8>) -> TextLook {
    let mut look = TextLook {
        alignment: alignment.unwrap_or(0),
        ..Default::default()
    };
    let Some(da) = da else {
        return look;
    };
    let words: Vec<&str> = da.split_ascii_whitespace().collect();
    let numbers = |at: usize, count: usize| -> Option<Vec<f32>> {
        let from = at.checked_sub(count)?;
        let n: Vec<f32> = words[from..at]
            .iter()
            .filter_map(|w| w.parse::<f32>().ok())
            .filter(|v| v.is_finite())
            .map(|v| v.clamp(0.0, 1.0))
            .collect();
        (n.len() == count).then_some(n)
    };
    for (at, word) in words.iter().enumerate() {
        match *word {
            "Tf" if at >= 2 => {
                if let Some(font) = words[at - 2].strip_prefix('/') {
                    look.font = font.to_owned();
                }
                if let Ok(size) = words[at - 1].parse::<f64>()
                    && (0.0..=1000.0).contains(&size)
                {
                    look.size = size;
                }
            }
            "g" => {
                if let Some(n) = numbers(at, 1) {
                    look.color = Color::new(n[0], n[0], n[0]);
                }
            }
            "rg" => {
                if let Some(n) = numbers(at, 3) {
                    look.color = Color::new(n[0], n[1], n[2]);
                }
            }
            "k" => {
                if let Some(n) = numbers(at, 4) {
                    look.color = Color::new(
                        (1.0 - n[0]) * (1.0 - n[3]),
                        (1.0 - n[1]) * (1.0 - n[3]),
                        (1.0 - n[2]) * (1.0 - n[3]),
                    );
                }
            }
            _ => {}
        }
    }
    look
}
