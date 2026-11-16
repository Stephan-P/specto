//! The form of the document: the fields of the file that was opened, with what was filled in.
//!
//! Only that file's form counts. Pages taken in from other files bring their widgets along, but
//! the form they belong to isn't part of the document (saving keeps the catalog of the first
//! file).
//!
//! A field shows through its widgets' appearances, which the renderer draws with the page. So a
//! page with a widget of a field that was filled in is rendered from a copy of the page with the
//! value in it, as pages with changed annotations are (see `annots`).
//!
//! One field can have the keyboard: the one last clicked or moved to with Tab. The frontend shows
//! which, and passes on the keys: Tab for the next one, Space to tick it or to type into it.

use std::sync::Arc;

use specto_annot::{FontFamily, TextFont};
use specto_doc::{FieldId, FieldValue};
use specto_forms::{Field, FieldKind};
use specto_geom::{PdfPoint, ViewPoint};
use specto_render::scheduler::Scheduler;

use crate::{FilePage, MAIN, Session, TextBoxPlace};

/// What a click on a field did, or asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum FieldClick {
    /// A box was ticked or unticked, or a radio button chosen.
    Changed,
    /// The keyboard is on a box or a button now, and nothing else changed.
    Focused,
    /// A choice field, to choose from: what it offers, which of that it holds, and its corners
    /// on screen. Follow with [`choose_option`](Session::choose_option).
    Choose {
        field: FieldId,
        labels: Vec<String>,
        chosen: Option<usize>,
        corners: [ViewPoint; 4],
    },
    /// A text field, to be typed into: where it is on screen, how high (in points), whether it
    /// takes several lines, whether what is typed is to be hidden, where its text stands (0 for
    /// left, 1 for centered, 2 for right), and what it holds.
    Text {
        field: FieldId,
        place: TextBoxPlace,
        height: f64,
        multiline: bool,
        password: bool,
        alignment: u8,
        text: String,
    },
    /// A text field on a page that is shown turned, where it can't be typed into in place: its
    /// corners on screen, to ask for the text beside it. Follow with
    /// [`set_field_text`](Session::set_field_text).
    Edit {
        field: FieldId,
        multiline: bool,
        password: bool,
        text: String,
        corners: [ViewPoint; 4],
    },
}

/// What kind of field the pointer is over, for its shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldHover {
    Text,
    Button,
}

impl Session {
    /// The fields of the form, read from the file when first asked for.
    pub fn form_fields(&mut self) -> Arc<[Field]> {
        if self.form.is_none() {
            let fields = specto_forms::fields(&self.sources[0].doc);
            self.form = Some(fields.into());
        }
        self.form.clone().unwrap_or_default()
    }

    /// What field `field` holds: what was filled in, else what the file has.
    pub fn field_value(&mut self, field: FieldId) -> Option<FieldValue> {
        if let Some(filled) = self.edit.field_value(field) {
            return Some(filled.clone());
        }
        let fields = self.form_fields();
        fields.iter().find(|f| f.id == field)?.value.clone()
    }

    /// The field that can be filled in under `p` (viewport coordinates): which, which of its
    /// widgets, and the position of the page.
    fn field_under(&mut self, p: ViewPoint) -> Option<(Field, usize, usize)> {
        let (position, at) = self.viewer.hit_test(p)?;
        let page = *self.edit.pages().get(position)?;
        if page.source != MAIN {
            return None;
        }
        let view_page = self.viewer.pages().get(position)?;
        let pdf = view_page.displayed().page_to_pdf().transform_point(at);
        let fields = self.form_fields();
        fields.iter().filter(|f| f.is_fillable()).find_map(|f| {
            let widget = f.widgets.iter().position(|w| {
                !w.hidden && w.page == page.page as usize && w.rect.contains_inclusive(pdf)
            })?;
            Some((f.clone(), widget, position))
        })
    }

    /// What kind of field is under `p`, if one that can be filled in is.
    pub fn field_hover(&mut self, p: ViewPoint) -> Option<FieldHover> {
        let (field, _, _) = self.field_under(p)?;
        Some(match field.kind {
            FieldKind::Text { .. } => FieldHover::Text,
            _ => FieldHover::Button,
        })
    }

    /// A click at `p` (viewport coordinates) on a field: ticks or unticks a checkbox, chooses a
    /// radio button, or says where a text field is, to type into it (follow with
    /// [`begin_field_text`](Self::begin_field_text)). The field has the keyboard from then on.
    /// `None` if no field that can be filled in is there.
    pub fn click_field(&mut self, scheduler: &Scheduler, p: ViewPoint) -> Option<FieldClick> {
        let (field, widget, position) = self.field_under(p)?;
        self.field_focus = Some((field.id, widget));
        self.press(scheduler, &field, widget, position)
    }

    /// The field that has the keyboard.
    pub fn field_focus(&self) -> Option<FieldId> {
        self.field_focus.map(|(field, _)| field)
    }

    /// Takes the keyboard off the form. Returns whether a field had it.
    pub fn clear_field_focus(&mut self) -> bool {
        self.field_focus.take().is_some()
    }

    /// Where the fields that can be filled in are, in the order Tab goes through them: by page as
    /// the document has them now, and on a page as the file lists them. Each with the position of
    /// its page.
    fn field_stops(&mut self) -> Vec<(FieldId, usize, usize)> {
        let fields = self.form_fields();
        let mut stops: Vec<(usize, usize, FieldId, usize)> = fields
            .iter()
            .filter(|f| f.is_fillable())
            .flat_map(|f| f.widgets.iter().enumerate().map(move |(i, w)| (f.id, i, w)))
            .filter(|(_, _, w)| !w.hidden)
            .filter_map(|(id, i, w)| Some((self.position_of(MAIN, w.page)?, w.order, id, i)))
            .collect();
        stops.sort_unstable();
        stops
            .into_iter()
            .map(|(position, _, id, i)| (id, i, position))
            .collect()
    }

    /// Moves the keyboard to the next field (the one before, if not `forward`), from the last to
    /// the first and the other way, and scrolls to it. Without a field that has the keyboard, to
    /// the first or the last. A text field is to be typed into, as after a click. `None` if the
    /// document has no field that can be filled in.
    pub fn tab_field(&mut self, scheduler: &Scheduler, forward: bool) -> Option<FieldClick> {
        let stops = self.field_stops();
        let count = stops.len();
        let at = self
            .field_focus
            .and_then(|(field, widget)| stops.iter().position(|s| (s.0, s.1) == (field, widget)));
        let next = match (at, forward) {
            (Some(at), true) => (at + 1) % count.max(1),
            (Some(at), false) => (at + count - 1) % count.max(1),
            (None, true) => 0,
            (None, false) => count.saturating_sub(1),
        };
        let (id, widget, position) = *stops.get(next)?;
        let fields = self.form_fields();
        let field = fields.iter().find(|f| f.id == id)?.clone();
        self.field_focus = Some((id, widget));
        self.viewer.reveal(position, field.widgets[widget].rect);
        match field.kind {
            FieldKind::Text { .. } => self.press(scheduler, &field, widget, position),
            _ => Some(FieldClick::Focused),
        }
    }

    /// Space or Enter on the field that has the keyboard: as a click on it.
    pub fn press_focused_field(&mut self, scheduler: &Scheduler) -> Option<FieldClick> {
        let (id, widget) = self.field_focus?;
        let (_, _, position) = *self
            .field_stops()
            .iter()
            .find(|s| (s.0, s.1) == (id, widget))?;
        let fields = self.form_fields();
        let field = fields.iter().find(|f| f.id == id)?.clone();
        self.press(scheduler, &field, widget, position)
    }

    /// The corners of the field that has the keyboard, on screen, and the position of its page.
    /// Not while it is typed into: the frontend's text field shows where it is then.
    pub fn focused_field_outline(&mut self) -> Option<(usize, [ViewPoint; 4])> {
        let (id, widget) = self.field_focus?;
        if self.typing_field == Some(id) {
            return None;
        }
        let fields = self.form_fields();
        let w = fields.iter().find(|f| f.id == id)?.widgets.get(widget)?;
        let position = self.position_of(MAIN, w.page)?;
        let r = w.rect;
        let at = |x, y| self.viewer.pdf_to_view(position, PdfPoint::new(x, y));
        Some((
            position,
            [
                at(r.min.x, r.min.y)?,
                at(r.max.x, r.min.y)?,
                at(r.max.x, r.max.y)?,
                at(r.min.x, r.max.y)?,
            ],
        ))
    }

    /// What a click on widget `widget` of `field`, on the page at `position`, does.
    fn press(
        &mut self,
        scheduler: &Scheduler,
        field: &Field,
        widget: usize,
        position: usize,
    ) -> Option<FieldClick> {
        let now = self.field_value(field.id);
        let w = &field.widgets[widget];
        match field.kind {
            FieldKind::Checkbox => {
                let on = now == Some(FieldValue::Check(true));
                self.fill_field(scheduler, field.id, FieldValue::Check(!on));
                Some(FieldClick::Changed)
            }
            FieldKind::Radio => {
                self.fill_field(scheduler, field.id, FieldValue::Choice(w.on_state.clone()));
                Some(FieldClick::Changed)
            }
            FieldKind::Text {
                multiline,
                password,
                ..
            } => {
                let text = match now {
                    Some(FieldValue::Text(t)) => t,
                    _ => String::new(),
                };
                let geometry = self.viewer.pages().get(position)?.displayed();
                if geometry.rotation() != specto_geom::Rotation::None {
                    let r = w.rect;
                    let at = |x, y| self.viewer.pdf_to_view(position, PdfPoint::new(x, y));
                    return Some(FieldClick::Edit {
                        field: field.id,
                        multiline,
                        password,
                        text,
                        corners: [
                            at(r.min.x, r.min.y)?,
                            at(r.max.x, r.min.y)?,
                            at(r.max.x, r.max.y)?,
                            at(r.min.x, r.max.y)?,
                        ],
                    });
                }
                let at = PdfPoint::new(w.rect.min.x, w.rect.max.y);
                let height = w.rect.height();
                // As the saved look sets it: the size the field asks for, or what fits.
                let size = if field.look.size > 0.0 {
                    field.look.size
                } else if multiline {
                    12.0
                } else {
                    ((height - 4.0) / 1.15).clamp(4.0, 12.0)
                };
                let place = TextBoxPlace {
                    position,
                    at,
                    corner: self.viewer.pdf_to_view(position, at)?,
                    width: w.rect.width(),
                    zoom: self.zoom_of_page(position)?,
                    size,
                    font: font_called(&field.look.font),
                    color: field.look.color,
                };
                Some(FieldClick::Text {
                    field: field.id,
                    place,
                    height,
                    multiline,
                    password,
                    alignment: field.look.alignment,
                    text,
                })
            }
            FieldKind::Choice { ref options, .. } => {
                let held = match now {
                    Some(FieldValue::Text(t)) => t,
                    _ => String::new(),
                };
                let r = w.rect;
                let at = |x, y| self.viewer.pdf_to_view(position, PdfPoint::new(x, y));
                Some(FieldClick::Choose {
                    field: field.id,
                    labels: options.iter().map(|o| o.label.clone()).collect(),
                    chosen: options.iter().position(|o| o.value == held),
                    corners: [
                        at(r.min.x, r.min.y)?,
                        at(r.max.x, r.min.y)?,
                        at(r.max.x, r.max.y)?,
                        at(r.min.x, r.max.y)?,
                    ],
                })
            }
            _ => None,
        }
    }

    /// Chooses option `index` of the choice field `field`. Returns whether that changed what it
    /// holds.
    pub fn choose_option(&mut self, scheduler: &Scheduler, field: FieldId, index: usize) -> bool {
        let fields = self.form_fields();
        let value = fields.iter().find(|f| f.id == field).and_then(|f| {
            let FieldKind::Choice { options, .. } = &f.kind else {
                return None;
            };
            Some(options.get(index)?.value.clone())
        });
        match value {
            Some(value) => self.fill_field(scheduler, field, FieldValue::Text(value)),
            None => false,
        }
    }

    /// Fills `value` into `field`. Returns whether that changed what it holds.
    pub fn fill_field(&mut self, scheduler: &Scheduler, field: FieldId, value: FieldValue) -> bool {
        let fields = self.form_fields();
        let Some(in_file) = fields.iter().find(|f| f.id == field) else {
            return false;
        };
        let changed = self.edit.fill_field(field, value, in_file.value.as_ref());
        if changed {
            self.sync_variants(scheduler);
        }
        changed
    }

    /// Starts typing into the text field `field`: the renderer shows it empty until
    /// [`end_field_text`](Self::end_field_text), so the frontend's text field stands in its
    /// place.
    pub fn begin_field_text(&mut self, scheduler: &Scheduler, field: FieldId) {
        self.typing_field = Some(field);
        self.sync_variants(scheduler);
    }

    /// Ends typing into a text field: with `text` it holds that now, without it stays as it was.
    /// Returns whether it changed.
    pub fn end_field_text(&mut self, scheduler: &Scheduler, text: Option<&str>) -> bool {
        let Some(field) = self.typing_field.take() else {
            return false;
        };
        let changed = match text {
            Some(text) => {
                let text = self.fitting(field, text);
                self.fill_field(scheduler, field, FieldValue::Text(text))
            }
            None => false,
        };
        // Shown with its value again, if nothing else did that.
        if !changed {
            self.sync_variants(scheduler);
        }
        changed
    }

    /// Fills `text` into the text field `field`, as far as the field takes it. Returns whether
    /// that changed what it holds.
    pub fn set_field_text(&mut self, scheduler: &Scheduler, field: FieldId, text: &str) -> bool {
        let text = self.fitting(field, text);
        self.fill_field(scheduler, field, FieldValue::Text(text))
    }

    /// `text` as the text field `field` takes it: on one line unless the field has several, and no
    /// longer than the field allows.
    fn fitting(&mut self, field: FieldId, text: &str) -> String {
        let fields = self.form_fields();
        let kind = fields.iter().find(|f| f.id == field).map(|f| &f.kind);
        let Some(FieldKind::Text {
            multiline,
            max_length,
            ..
        }) = kind
        else {
            return text.to_owned();
        };
        let text = if *multiline {
            text.to_owned()
        } else {
            text.split(['\n', '\r']).collect::<Vec<_>>().join(" ")
        };
        match max_length {
            Some(most) => text.chars().take(*most as usize).collect(),
            None => text,
        }
    }

    /// What the form's fields with a widget on page `page` of `source` hold other than in the
    /// file: what a copy of the page must be written with. A field being typed into counts as
    /// empty.
    pub(crate) fn page_form(&mut self, (source, page): FilePage) -> Vec<(FieldId, FieldValue)> {
        if source != MAIN {
            return Vec::new();
        }
        let mut values = self.edit.form_values();
        if let Some(typing) = self.typing_field {
            values.retain(|(id, _)| *id != typing);
            values.push((typing, FieldValue::Text(String::new())));
        }
        if values.is_empty() {
            return values;
        }
        let fields = self.form_fields();
        values.retain(|(id, _)| {
            fields
                .iter()
                .find(|f| f.id == *id)
                .is_some_and(|f| f.widgets.iter().any(|w| w.page == page as usize))
        });
        values.sort_by_key(|(id, _)| *id);
        values
    }

    /// The pages of the main file that show a field holding something other than in the file.
    pub(crate) fn pages_with_filled_fields(&mut self) -> Vec<FilePage> {
        let mut ids: Vec<FieldId> = self.edit.form_values().iter().map(|(id, _)| *id).collect();
        ids.extend(self.typing_field);
        if ids.is_empty() {
            return Vec::new();
        }
        let fields = self.form_fields();
        let mut pages: Vec<FilePage> = fields
            .iter()
            .filter(|f| ids.contains(&f.id))
            .flat_map(|f| f.widgets.iter().map(|w| (MAIN, w.page as u32)))
            .collect();
        pages.sort_unstable();
        pages.dedup();
        pages
    }
}

/// The font a form's font resource most likely is, by its name: for the frontend's text field,
/// which should look like the saved text.
fn font_called(name: &str) -> TextFont {
    if let Some(font) = TextFont::from_resource_name(name) {
        return font;
    }
    let lower = name.to_lowercase();
    TextFont {
        family: if lower.contains("times") || lower.contains("serif") && !lower.contains("sans") {
            FontFamily::Serif
        } else if lower.contains("cour") || lower.contains("mono") {
            FontFamily::Mono
        } else {
            FontFamily::Sans
        },
        bold: lower.contains("bold"),
        italic: lower.contains("italic") || lower.contains("oblique"),
    }
}
