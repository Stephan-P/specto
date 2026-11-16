//! Reading the fields of a form.

use std::collections::{HashMap, HashSet};

use specto_annot::Color;
use specto_geom::{PdfBox, PdfPoint};
use specto_source::hayro_syntax::object::{
    Array, Dict, MaybeRef, Name, Object, ObjectIdentifier, Stream, String as PdfString,
};
use specto_source::{SourceDocument, decode_text};

use crate::{ChoiceOption, Field, FieldId, FieldKind, FieldValue, TextLook, Widget};

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
const COMBO: u32 = 1 << 17;
const EDIT: u32 = 1 << 18;
const MULTI_SELECT: u32 = 1 << 21;
const COMB: u32 = 1 << 24;

/// Options of a choice field beyond this many are ignored.
const MAX_OPTIONS: usize = 10_000;

/// What a field takes from those above it in the tree.
#[derive(Clone, Default)]
struct Inherited {
    name: String,
    kind: Option<Vec<u8>>,
    flags: Option<u32>,
    look: Option<String>,
    alignment: Option<u8>,
    /// The form leaves showing what its fields hold to the reader (`/NeedAppearances`).
    unshown: bool,
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
        unshown: form.get::<bool>(b"NeedAppearances").unwrap_or(false),
        ..Default::default()
    };
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for (node, id) in children(&roots) {
        walk(&node, id, &top, 0, &pages, &mut seen, &mut out);
    }
    order_by_place(source, &mut out);
    out
}

/// Puts the widgets of the pages that ask for it (`/Tabs`) in the order of their places: `R`
/// for rows from the top, each from the left, and `C` for columns from the left, each from the
/// top. Other pages keep the order of their annotations.
fn order_by_place(source: &SourceDocument, fields: &mut [Field]) {
    for (page, p) in source.pdf().pages().iter().enumerate() {
        let columns = match p.raw().get::<Name<'_>>(b"Tabs").as_deref() {
            Some(b"R") => false,
            Some(b"C") => true,
            _ => continue,
        };
        let mut on_page: Vec<&mut Widget> = fields
            .iter_mut()
            .flat_map(|f| f.widgets.iter_mut())
            .filter(|w| w.page == page)
            .collect();
        // Along the rows (or columns): where a widget starts and ends across them, and where
        // it is along them. Seen from the top left, both grow.
        let across = |w: &Widget| match columns {
            false => (-w.rect.max.y, -w.rect.min.y, w.rect.min.x),
            true => (w.rect.min.x, w.rect.max.x, -w.rect.max.y),
        };
        on_page.sort_by(|a, b| {
            across(a)
                .0
                .total_cmp(&across(b).0)
                .then(a.order.cmp(&b.order))
        });
        // A widget whose middle is within the first one of a row belongs to that row.
        let mut rows: Vec<(f64, f64, Vec<&mut Widget>)> = Vec::new();
        for w in on_page {
            let (start, end, _) = across(w);
            let middle = (start + end) / 2.0;
            match rows.last_mut() {
                Some((from, to, row)) if (*from..=*to).contains(&middle) => row.push(w),
                _ => rows.push((start, end, vec![w])),
            }
        }
        let mut order = 0;
        for (_, _, mut row) in rows {
            row.sort_by(|a, b| {
                across(a)
                    .2
                    .total_cmp(&across(b).2)
                    .then(a.order.cmp(&b.order))
            });
            for w in row {
                w.order = order;
                order += 1;
            }
        }
    }
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
fn widget_pages(source: &SourceDocument) -> HashMap<ObjectIdentifier, (usize, usize)> {
    let mut map = HashMap::new();
    for (page, p) in source.pdf().pages().iter().enumerate() {
        let Some(annots) = p.raw().get::<Array<'_>>(b"Annots") else {
            continue;
        };
        for (order, entry) in annots.raw_iter().take(MAX_FIELDS).enumerate() {
            if let MaybeRef::Ref(r) = entry {
                map.entry(ObjectIdentifier::from(r))
                    .or_insert((page, order));
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
    pages: &HashMap<ObjectIdentifier, (usize, usize)>,
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
        unshown: above.unshown,
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
            // Only with a length, and not for several lines or passwords.
            comb: flags & COMB != 0
                && flags & (MULTILINE | PASSWORD) == 0
                && node
                    .get::<u32>(b"MaxLen")
                    .is_some_and(|m| (1..=1000).contains(&m)),
        },
        b"Btn" if flags & PUSH_BUTTON != 0 => FieldKind::Button,
        b"Btn" if flags & RADIO != 0 => FieldKind::Radio,
        b"Btn" => FieldKind::Checkbox,
        b"Ch" => FieldKind::Choice {
            options: options(node),
            list: flags & COMBO == 0,
            editable: flags & COMBO != 0 && flags & EDIT != 0,
            multiple: flags & MULTI_SELECT != 0,
        },
        b"Sig" => FieldKind::Signature,
        _ => FieldKind::Other,
    };
    let mut widgets: Vec<Widget> = widget_kids
        .iter()
        .filter(|(k, _)| is_widget(k) || k.contains_key(b"Rect"))
        .filter_map(|(k, kid_id)| widget(k, *kid_id, pages))
        .collect();
    // Without kids, the field is its own widget.
    let own = widgets.is_empty() && (is_widget(node) || node.contains_key(b"Rect"));
    if own {
        widgets.extend(widget(node, id, pages));
    }
    // What it holds, and what a reset has it hold: the name of the state that is on, for
    // buttons, and text for text fields.
    let held = |key: &[u8]| {
        let state = node
            .get::<Name<'_>>(key)
            .map(|n| String::from_utf8_lossy(n.as_ref()).into_owned())
            .filter(|s| s != "Off");
        match &kind {
            FieldKind::Text { .. } => Some(FieldValue::Text(text(node, key).unwrap_or_default())),
            FieldKind::Checkbox => Some(FieldValue::Check(state.is_some())),
            FieldKind::Radio => Some(FieldValue::Choice(state)),
            FieldKind::Choice {
                multiple: false, ..
            } => Some(FieldValue::Text(text(node, key).unwrap_or_default())),
            FieldKind::Choice { options, .. } => {
                Some(FieldValue::Several(several(node, key, options)))
            }
            _ => None,
        }
    };
    let (value, default) = (held(b"V"), held(b"DV"));
    let unshown = here.unshown
        && match &value {
            Some(FieldValue::Text(t)) => !t.is_empty(),
            Some(FieldValue::Several(all)) => !all.is_empty(),
            _ => false,
        }
        && if own {
            !shows_text(node)
        } else {
            widget_kids.iter().any(|(k, _)| !shows_text(k))
        };
    out.push(Field {
        id,
        name: here.name.clone(),
        label: text(node, b"TU").filter(|t| !t.is_empty()),
        kind,
        read_only: flags & READ_ONLY != 0,
        required: flags & REQUIRED != 0,
        value,
        default,
        unshown,
        look: text_look(here.look.as_deref(), here.alignment),
        widgets,
    });
}

/// What is chosen in a list that takes several (under `key`, one text or an array of them):
/// those of `options` it names, in their order.
fn several(node: &Dict<'_>, key: &[u8], options: &[ChoiceOption]) -> Vec<String> {
    let held: Vec<String> = match node.get::<Array<'_>>(key) {
        Some(all) => all
            .iter::<PdfString<'_>>()
            .take(MAX_OPTIONS)
            .map(|s| decode_text(s.as_bytes()))
            .collect(),
        None => text(node, key).into_iter().collect(),
    };
    options
        .iter()
        .filter(|o| held.contains(&o.value))
        .map(|o| o.value.clone())
        .collect()
}

/// What a choice field offers (`/Opt`): each a text, or a value and what to show for it.
fn options(node: &Dict<'_>) -> Vec<ChoiceOption> {
    let Some(opt) = node.get::<Array<'_>>(b"Opt") else {
        return Vec::new();
    };
    let string = |o: Object<'_>| match o {
        Object::String(s) => Some(decode_text(s.as_bytes())),
        _ => None,
    };
    opt.iter::<Object<'_>>()
        .take(MAX_OPTIONS)
        .filter_map(|entry| match entry {
            Object::Array(pair) => {
                let mut parts = pair.iter::<Object<'_>>();
                let value = string(parts.next()?)?;
                let label = parts
                    .next()
                    .and_then(string)
                    .unwrap_or_else(|| value.clone());
                Some(ChoiceOption { value, label })
            }
            other => {
                let value = string(other)?;
                Some(ChoiceOption {
                    label: value.clone(),
                    value,
                })
            }
        })
        .collect()
}

/// The widget in `dict`, if it has an area and is on a page.
/// Whether the look a widget has in the file (`/AP /N`) has text in it.
fn shows_text(widget: &Dict<'_>) -> bool {
    let Some(content) = widget
        .get::<Dict<'_>>(b"AP")
        .and_then(|ap| ap.get::<Stream<'_>>(b"N"))
        .and_then(|look| look.decoded().ok())
    else {
        return false;
    };
    // An operator that shows text: `Tj`, `TJ`, `'` or `"`, after a string or an array.
    content.windows(3).any(|w| {
        matches!(w[0], b')' | b'>' | b']' | b' ' | b'\n' | b'\r')
            && (matches!(&w[1..], b"Tj" | b"TJ") || matches!(w[1], b'\'' | b'"'))
    })
}

fn widget(
    dict: &Dict<'_>,
    id: FieldId,
    pages: &HashMap<ObjectIdentifier, (usize, usize)>,
) -> Option<Widget> {
    let numbers: Vec<f64> = dict
        .get::<Array<'_>>(b"Rect")?
        .iter::<f64>()
        .filter(|n| n.is_finite())
        .collect();
    let [x0, y0, x1, y1] = numbers[..] else {
        return None;
    };
    let (page, order) = *pages.get(&ObjectIdentifier::new(id.0, i32::from(id.1)))?;
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
        order,
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
