//! The document's annotations: those of its files (read when a page is
//! first asked for) as the edits have left them.
//!
//! For now a frontend shows the annotations of the files as part of the
//! page (the renderer draws them, from their appearance streams) and draws
//! only those made here itself, from [`Session::overlay`]. So only those
//! can be picked and deleted yet.

use std::sync::Arc;

use specto_annot::{Annotation, Color, Date, MarkupStyle, Shape, shapes};
use specto_doc::{AnnotId, SourceId};
use specto_geom::ViewPoint;
use specto_view::TextPages;

use crate::Session;

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

    /// The annotations made here on the page at `position`: the ones a
    /// frontend draws itself (see the module documentation).
    fn own_annotations(&mut self, position: usize) -> Vec<(AnnotId, Arc<Annotation>)> {
        let mut all = self.annotations(position);
        all.retain(|(id, _)| matches!(id, AnnotId::New(_)));
        all
    }

    /// Whether there is anything for [`overlay`](Self::overlay) to give, on
    /// any page: a cheap question to ask before drawing.
    pub fn has_overlay(&self) -> bool {
        self.edit.has_annotation_edits()
    }

    /// What to draw over the page at `position`, bottom to top: the
    /// annotations made here, where they are on screen now.
    pub fn overlay(&mut self, position: usize) -> Vec<ViewShape> {
        if !self.has_overlay() {
            return Vec::new();
        }
        let annotations = self.own_annotations(position);
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
    /// picked: the position of its page, and which it is.
    pub fn annotation_at(&mut self, p: ViewPoint) -> Option<(usize, AnnotId)> {
        let (position, at) = self.viewer.hit_test(p)?;
        let view_page = self.viewer.pages().get(position)?;
        let pdf = view_page.displayed().page_to_pdf().transform_point(at);
        self.own_annotations(position)
            .into_iter()
            .rev()
            .find(|(_, a)| a.flags.is_visible() && a.contains(pdf))
            .map(|(id, _)| (position, id))
    }

    /// The annotation that is picked, if it is still there.
    fn picked_one(&mut self) -> Option<(usize, AnnotId, Arc<Annotation>)> {
        let (position, id) = self.picked?;
        let (_, a) = self
            .own_annotations(position)
            .into_iter()
            .find(|(i, _)| *i == id)?;
        Some((position, id, a))
    }

    /// The corners of the box around the picked annotation on screen, and
    /// the position of its page.
    pub fn picked_outline(&mut self) -> Option<(usize, [ViewPoint; 4])> {
        let (position, _, a) = self.picked_one()?;
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
    /// one.
    pub fn remove_picked_annotation(&mut self) -> bool {
        let Some((_, id, a)) = self.picked_one() else {
            return false;
        };
        self.picked = None;
        self.edit.remove_annotation(id, &a)
    }
}

/// The time now, in UTC.
fn now() -> Date {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    Date::from_unix(secs)
}
