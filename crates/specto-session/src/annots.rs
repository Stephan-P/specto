//! The document's annotations: those of its files (read when a page is
//! first asked for) as the edits have left them.
//!
//! Who draws what: the annotations of a file are part of the page as the
//! renderer draws it, from their appearance streams. The frontend draws,
//! from [`Session::overlay`], those that have no such look in the file:
//! the ones made here, ones changed here, and ones the file gives no
//! appearance for.
//!
//! The renderer draws all of a page's annotations or none. So when one of
//! a file is deleted (or changed so that the frontend draws it), its page
//! is rendered from a *variant* instead: a copy of that one page without
//! the annotation, written the way saving writes it and opened as a
//! document of its own. Text, links and everything else still come from
//! the file.

use std::collections::HashMap;
use std::sync::Arc;

use specto_annot::{Annotation, Appearance, Color, Date, MarkupStyle, Origin, Shape, shapes};
use specto_doc::{AnnotId, PageAnnotChanges, PageRef, SourceId};
use specto_geom::ViewPoint;
use specto_render::scheduler::Scheduler;
use specto_render::tile::DocId;
use specto_view::TextPages;
use specto_write::WriteError;

use crate::{FilePage, Session, view_pages};

/// A page rendered from a copy of itself without some of its annotations.
pub(crate) struct Variant {
    /// The annotations of the file the copy leaves out.
    hidden: Vec<Origin>,
    /// The copy, in the scheduler: a document of one page.
    pub(crate) doc_id: DocId,
}

/// Whether the frontend draws `annotation` (see the module documentation)
/// rather than the renderer.
fn floats(annotation: &Annotation) -> bool {
    annotation.appearance != Appearance::Original && shapes(annotation).is_some()
}

/// A piece of an annotation's look where it is on screen, in viewport
/// coordinates (see [`Shape`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ViewShape {
    Fill {
        quad: [ViewPoint; 4],
        color: Color,
        opacity: f32,
        multiply: bool,
    },
    Stroke {
        from: ViewPoint,
        to: ViewPoint,
        width: f64,
        color: Color,
        opacity: f32,
    },
}

impl Session {
    /// The annotations page `page` of file `source` has in the file.
    fn imported(&mut self, source: SourceId, page: u32) -> Arc<[Arc<Annotation>]> {
        let Some(s) = self.sources.get_mut(source.0 as usize) else {
            return Arc::default();
        };
        let doc = &s.doc;
        s.annots
            .entry(page)
            .or_insert_with(|| {
                specto_annot::import(doc, page as usize)
                    .into_iter()
                    .map(Arc::new)
                    .collect()
            })
            .clone()
    }

    /// The annotations of the page at position `position`, as edited:
    /// bottom to top.
    pub fn annotations(&mut self, position: usize) -> Vec<(AnnotId, Arc<Annotation>)> {
        let Some(&p) = self.edit.pages().get(position) else {
            return Vec::new();
        };
        let imported = self.imported(p.source, p.page);
        self.edit.annotations(p.source, p.page, &imported)
    }

    /// The annotations of the file that page `page` of `source` must be
    /// rendered without: the deleted ones, and changed ones the frontend
    /// draws.
    fn hidden(&self, source: SourceId, page: u32) -> Vec<Origin> {
        let PageAnnotChanges {
            mut removed,
            replaced,
            ..
        } = self.edit.annotation_changes(source, page);
        removed.extend(replaced.iter().filter(|(_, a)| floats(a)).map(|(o, _)| *o));
        removed.sort_by_key(|o| o.index);
        removed
    }

    /// Page `page` of `source` as a file of its own, without the
    /// annotations `hidden`.
    fn variant_file(
        &self,
        (source, page): FilePage,
        hidden: &[Origin],
    ) -> Result<Vec<u8>, WriteError> {
        let s = self
            .sources
            .get(source.0 as usize)
            .ok_or(WriteError::UnknownSource)?;
        let from = specto_write::Source {
            id: source,
            data: &s.data,
            password: s.password.as_deref(),
        };
        let changes = PageAnnotChanges {
            removed: hidden.to_vec(),
            ..Default::default()
        };
        // Not encrypted: it never leaves the program.
        specto_write::write_edited(
            &[from],
            &[PageRef::new(source, page)],
            &HashMap::from([((source, page), changes)]),
            false,
        )
    }

    /// Makes the variants what the edits ask for: a copy for every page
    /// with hidden annotations, none for the others. A page whose copy
    /// can't be written is shown from its file, with all its annotations.
    pub(crate) fn sync_variants(&mut self, scheduler: &Scheduler) {
        self.sync_variants_with(scheduler, None);
    }

    /// As [`sync_variants`](Self::sync_variants); `ready` is a copy that is
    /// written already: its page, what it leaves out, and the file.
    fn sync_variants_with(
        &mut self,
        scheduler: &Scheduler,
        mut ready: Option<(FilePage, Vec<Origin>, Vec<u8>)>,
    ) {
        let wanted: HashMap<FilePage, Vec<Origin>> = self
            .edit
            .pages_with_changed_annotations()
            .into_iter()
            .map(|(source, page)| ((source, page), self.hidden(source, page)))
            .filter(|(_, hidden)| !hidden.is_empty())
            .collect();
        let mut changed = false;
        let stale: Vec<FilePage> = self
            .variants
            .iter()
            .filter(|(key, v)| wanted.get(key) != Some(&v.hidden))
            .map(|(key, _)| *key)
            .collect();
        for key in stale {
            if let Some(v) = self.variants.remove(&key) {
                scheduler.close(v.doc_id);
                changed = true;
            }
        }
        for (key, hidden) in wanted {
            if self.variants.contains_key(&key) {
                continue;
            }
            let file = match ready.take_if(|(k, h, _)| *k == key && *h == hidden) {
                Some((_, _, file)) => Ok(file),
                None => self.variant_file(key, &hidden),
            };
            let opened = file.ok().and_then(|f| scheduler.open(f, None).ok());
            if let Some(opened) = opened {
                let doc_id = opened.id;
                self.variants.insert(key, Variant { hidden, doc_id });
                changed = true;
            }
        }
        if changed {
            let pages = view_pages(&self.edit, &self.sources, &self.variants);
            self.viewer.set_page_sources(pages);
        }
    }

    /// What to draw over the page at `position`, bottom to top: the
    /// annotations the frontend draws, where they are on screen now.
    pub fn overlay(&mut self, position: usize) -> Vec<ViewShape> {
        let mut annotations = self.annotations(position);
        annotations.retain(|(_, a)| a.flags.is_visible() && floats(a));
        let viewer = &self.viewer;
        let at = |p| viewer.pdf_to_view(position, p);
        let mut out = Vec::new();
        for (_, a) in annotations {
            for shape in shapes(&a).unwrap_or_default() {
                let on_screen = match shape {
                    Shape::Fill {
                        quad,
                        color,
                        opacity,
                        multiply,
                    } => (|| {
                        let [a, b, c, d] = quad.points;
                        Some(ViewShape::Fill {
                            quad: [at(a)?, at(b)?, at(c)?, at(d)?],
                            color,
                            opacity,
                            multiply,
                        })
                    })(),
                    Shape::Stroke {
                        from,
                        to,
                        width,
                        color,
                        opacity,
                    } => (|| {
                        let (a, b) = (at(from)?, at(to)?);
                        // How long a point is on screen, from the line itself.
                        let length = (to - from).length();
                        let zoom = if length > 0.0 {
                            (b - a).length() / length
                        } else {
                            1.0
                        };
                        Some(ViewShape::Stroke {
                            from: a,
                            to: b,
                            width: width * zoom,
                            color,
                            opacity,
                        })
                    })(),
                };
                out.extend(on_screen);
            }
        }
        out
    }

    /// Marks the selected text in `style` and `color`, as one edit: one
    /// annotation per page the selection touches. The selection is given
    /// up. Returns whether anything was marked.
    pub fn mark_selection(&mut self, style: MarkupStyle, color: Color) -> bool {
        let modified = Some(now());
        let mut new = Vec::new();
        {
            let pages = self.edit.pages().to_vec();
            let (_, mut texts, selection) = self.text();
            let Some(selection) = selection.as_ref().filter(|s| !s.is_empty()) else {
                return false;
            };
            let (start, end) = selection.range();
            for position in start.page..=end.page {
                let Some(&p) = pages.get(position) else {
                    break;
                };
                let Some(text) = texts.text_page(position) else {
                    continue;
                };
                let Some(range) = selection.page_range(position, text.chars.len()) else {
                    continue;
                };
                let contents = text.text_range(range.clone());
                let quads = text.selection_quads(range);
                let Some(mut a) = Annotation::text_markup(style, quads, color) else {
                    continue;
                };
                // The marked text, as other programs put it there too: it
                // is what a list of annotations shows.
                a.contents = Some(contents).filter(|c| !c.is_empty());
                a.modified = modified;
                new.push((p.source, p.page, a));
            }
        }
        if new.is_empty() {
            return false;
        }
        self.edit.add_annotations(new);
        self.selection = None;
        true
    }

    /// The topmost annotation under `p` (viewport coordinates) that can be
    /// picked: the position of its page, and which it is. Not ones the
    /// file says are locked or read-only.
    pub fn annotation_at(&mut self, p: ViewPoint) -> Option<(usize, AnnotId)> {
        let (position, at) = self.viewer.hit_test(p)?;
        let view_page = self.viewer.pages().get(position)?;
        let pdf = view_page.displayed().page_to_pdf().transform_point(at);
        self.annotations(position)
            .into_iter()
            .rev()
            .find(|(_, a)| a.flags.is_visible() && a.flags.is_editable() && a.contains(pdf))
            .map(|(id, _)| (position, id))
    }

    /// The annotation that is picked, if it is still there.
    fn picked_one(&mut self) -> Option<(AnnotId, Arc<Annotation>)> {
        let (position, id) = self.picked?;
        self.annotations(position)
            .into_iter()
            .find(|(i, _)| *i == id)
    }

    /// The corners of the box around the picked annotation on screen, and
    /// the position of its page.
    pub fn picked_outline(&mut self) -> Option<(usize, [ViewPoint; 4])> {
        let (position, _) = self.picked?;
        let (_, a) = self.picked_one()?;
        let r = a.rect;
        let at = |x, y| self.viewer.pdf_to_view(position, (x, y).into());
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

    /// Deletes the annotation that is picked. Returns whether there was
    /// one, or why it can't be deleted: for one of a file, its page must
    /// be written anew without it (see the module documentation), which
    /// fails for the files saving fails for. `scheduler` is the one the
    /// session was opened with.
    pub fn remove_picked_annotation(&mut self, scheduler: &Scheduler) -> Result<bool, WriteError> {
        let Some((id, a)) = self.picked_one() else {
            return Ok(false);
        };
        // The copy of the page first: if it can't be made, nothing changes.
        let mut ready = None;
        if let (AnnotId::File { source, page, .. }, Some(origin)) = (id, a.origin)
            && !floats(&a)
        {
            let mut hidden = self.hidden(source, page);
            hidden.push(origin);
            hidden.sort_by_key(|o| o.index);
            let file = self.variant_file((source, page), &hidden)?;
            ready = Some(((source, page), hidden, file));
        }
        self.picked = None;
        let removed = self.edit.remove_annotation(id, &a);
        if removed {
            self.sync_variants_with(scheduler, ready);
        }
        Ok(removed)
    }
}

/// The time now, in UTC.
fn now() -> Date {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    Date::from_unix(secs)
}
