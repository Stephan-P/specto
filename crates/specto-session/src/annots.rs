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

use specto_annot::{
    AnnotKind, Annotation, Appearance, Color, Date, MarkupStyle, Origin, Path, Shape, shapes,
};
use specto_doc::{AnnotId, PageAnnotChanges, PageRef, SourceId};
use specto_geom::{PdfBox, PdfPoint, ViewPoint, ViewSpace};
use specto_render::scheduler::Scheduler;
use specto_render::tile::DocId;
use specto_view::TextPages;
use specto_write::WriteError;

use crate::{FilePage, Session, view_pages};

/// A page rendered from a copy of itself with other annotations than the
/// file has.
pub(crate) struct Variant {
    /// The annotations of the file the copy leaves out.
    hidden: Vec<Origin>,
    /// The annotations the copy has in addition: text boxes made or
    /// changed here, which only the renderer can draw.
    baked: Vec<Arc<Annotation>>,
    /// The copy, in the scheduler: a document of one page.
    pub(crate) doc_id: DocId,
}

/// A variant that is written but not opened yet: its page, the annotations
/// it leaves out and has in addition, and the file.
type WrittenVariant = (FilePage, Vec<Origin>, Vec<Arc<Annotation>>, Vec<u8>);

/// Whether `annotation` is one made or changed here that the frontend
/// can't draw, so that the renderer must, from a copy of the page with it
/// in: a text box.
fn bakes(annotation: &Annotation) -> bool {
    annotation.appearance == Appearance::Generated
        && shapes(annotation).is_none()
        && specto_annot::appearance(annotation).is_some()
}

/// Whether the frontend draws `annotation` (see the module documentation)
/// rather than the renderer.
fn floats(annotation: &Annotation) -> bool {
    matches!(
        annotation.appearance,
        Appearance::Missing | Appearance::Generated
    ) && shapes(annotation).is_some()
}

/// A piece of an annotation's look where it is on screen, in viewport
/// coordinates (see [`Shape`]).
#[derive(Clone, Debug, PartialEq)]
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
    /// An outline, filled, drawn with a round pen of a width, or both.
    Path {
        path: Path<ViewSpace>,
        fill: Option<Color>,
        stroke: Option<(Color, f64)>,
        opacity: f32,
    },
}

/// What of an annotation a drag takes hold of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grip {
    /// The whole of it, to move it.
    Move,
    /// A corner of its frame (0 to 3, around it), to resize it: the corner
    /// across stays where it is.
    Corner(usize),
    /// An end of a line (0 or 1), to put it elsewhere.
    End(usize),
}

/// An annotation being moved or resized: the document has it as it was
/// until the drag ends.
#[derive(Clone, Debug)]
pub(crate) struct Adjust {
    id: AnnotId,
    /// The position of its page.
    position: usize,
    grip: Grip,
    /// Where on the page the drag started.
    from: PdfPoint,
    original: Arc<Annotation>,
    /// As it would be if the drag ended now.
    current: Annotation,
}

/// How near a handle the pointer must be to take it, in pixels.
const GRIP_REACH: f64 = 9.0;

/// The corners of `frame`, around it.
fn corners(frame: PdfBox) -> [PdfPoint; 4] {
    [
        frame.min,
        PdfPoint::new(frame.max.x, frame.min.y),
        frame.max,
        PdfPoint::new(frame.min.x, frame.max.y),
    ]
}

/// The handles of `a`: the ends of a line, else the corners of its frame.
fn grips(a: &Annotation) -> Vec<(Grip, PdfPoint)> {
    if !a.is_resizable() {
        return Vec::new();
    }
    match a.kind {
        AnnotKind::Line { from, to } => vec![(Grip::End(0), from), (Grip::End(1), to)],
        _ => corners(a.frame())
            .into_iter()
            .enumerate()
            .map(|(i, c)| (Grip::Corner(i), c))
            .collect(),
    }
}

/// What a drag of the pointer draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DraftKind {
    /// A freehand stroke along the pointer's way.
    Ink,
    /// A rectangle, an ellipse in it, or a line, from where the drag
    /// started to where the pointer is.
    Square,
    Circle,
    Line,
}

/// An annotation being drawn: not in the document until the drag ends.
#[derive(Clone, Debug)]
pub(crate) struct Draft {
    kind: DraftKind,
    /// The position of the page it is on.
    position: usize,
    color: Color,
    line_width: f64,
    /// Where the pointer has been, on that page; for all but ink only the
    /// first and the last count.
    points: Vec<PdfPoint>,
    /// How far apart points of a stroke are kept, in points: about a pixel.
    step: f64,
}

impl Draft {
    /// The annotation as it would be if the drag ended now.
    fn annotation(&self) -> Option<Annotation> {
        let (&from, &to) = (self.points.first()?, self.points.last()?);
        let half = self.line_width / 2.0;
        let around = |points: &[PdfPoint]| PdfBox::from_points(points).inflate(half, half);
        let (kind, rect) = match self.kind {
            DraftKind::Ink => (
                AnnotKind::Ink {
                    strokes: vec![self.points.clone()],
                },
                around(&self.points),
            ),
            DraftKind::Square => (AnnotKind::Square { fill: None }, around(&[from, to])),
            DraftKind::Circle => (AnnotKind::Circle { fill: None }, around(&[from, to])),
            DraftKind::Line => (AnnotKind::Line { from, to }, around(&[from, to])),
        };
        let mut a = Annotation::new(kind, rect);
        a.color = Some(self.color);
        a.line_width = self.line_width;
        Some(a)
    }

    /// Whether the drag went anywhere: a click with a shape tool draws
    /// nothing, a click with the pen a dot.
    fn is_worth_keeping(&self) -> bool {
        match self.kind {
            DraftKind::Ink => !self.points.is_empty(),
            _ => match (self.points.first(), self.points.last()) {
                (Some(a), Some(b)) => (*b - *a).length() >= 2.0 * self.step,
                _ => false,
            },
        }
    }
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

    /// Every annotation of the document that is shown, in the order of its
    /// pages: the position of its page, which it is, and the annotation.
    /// This reads the annotations of every page of the files, once.
    pub fn all_annotations(&mut self) -> Vec<(usize, AnnotId, Arc<Annotation>)> {
        (0..self.edit.page_count())
            .flat_map(|position| {
                self.annotations(position)
                    .into_iter()
                    .filter(|(_, a)| a.flags.is_visible())
                    .map(move |(id, a)| (position, id, a))
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// Picks the annotation `id` on the page at `position` and scrolls to
    /// it. Returns whether it is there.
    pub fn pick_and_show(&mut self, position: usize, id: AnnotId) -> bool {
        let Some((_, a)) = self
            .annotations(position)
            .into_iter()
            .find(|(i, _)| *i == id)
        else {
            return false;
        };
        self.picked = Some((position, id));
        self.selection = None;
        self.viewer.reveal(position, a.rect);
        true
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
        removed.extend(
            replaced
                .iter()
                .filter(|(_, a)| floats(a) || bakes(a))
                .map(|(o, _)| *o),
        );
        // And one that is being dragged about.
        if let Some((lifted, origin)) = self.lifted
            && lifted == (source, page)
            && !removed.contains(&origin)
        {
            removed.push(origin);
        }
        removed.sort_by_key(|o| o.index);
        removed
    }

    /// The annotations page `page` of `source` must be rendered with that
    /// the file doesn't have: text boxes made or changed here.
    fn baked(&self, source: SourceId, page: u32) -> Vec<Arc<Annotation>> {
        let changes = self.edit.annotation_changes(source, page);
        changes
            .replaced
            .into_iter()
            .map(|(_, a)| a)
            .chain(changes.added)
            .filter(|a| bakes(a))
            .collect()
    }

    /// Page `page` of `source` as a file of its own, without the
    /// annotations `hidden` and with `baked`.
    fn variant_file(
        &self,
        (source, page): FilePage,
        hidden: &[Origin],
        baked: &[Arc<Annotation>],
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
            added: baked.to_vec(),
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
    fn sync_variants_with(&mut self, scheduler: &Scheduler, mut ready: Option<WrittenVariant>) {
        let wanted: HashMap<FilePage, (Vec<Origin>, Vec<Arc<Annotation>>)> = self
            .edit
            .pages_with_annotation_edits()
            .into_iter()
            .chain(self.lifted.map(|(page, _)| page))
            .map(|(source, page)| {
                let spec = (self.hidden(source, page), self.baked(source, page));
                ((source, page), spec)
            })
            .filter(|(_, (hidden, baked))| !hidden.is_empty() || !baked.is_empty())
            .collect();
        let mut changed = false;
        let stale: Vec<FilePage> = self
            .variants
            .iter()
            .filter(|(key, v)| {
                wanted
                    .get(key)
                    .is_none_or(|(hidden, baked)| *hidden != v.hidden || *baked != v.baked)
            })
            .map(|(key, _)| *key)
            .collect();
        for key in stale {
            if let Some(v) = self.variants.remove(&key) {
                scheduler.close(v.doc_id);
                changed = true;
            }
        }
        for (key, (hidden, baked)) in wanted {
            if self.variants.contains_key(&key) {
                continue;
            }
            let file = match ready.take_if(|(k, h, b, _)| *k == key && *h == hidden && *b == baked)
            {
                Some((_, _, _, file)) => Ok(file),
                None => self.variant_file(key, &hidden, &baked),
            };
            let opened = file.ok().and_then(|f| scheduler.open(f, None).ok());
            if let Some(opened) = opened {
                let doc_id = opened.id;
                self.variants.insert(
                    key,
                    Variant {
                        hidden,
                        baked,
                        doc_id,
                    },
                );
                changed = true;
            }
        }
        if changed {
            let pages = view_pages(&self.edit, &self.sources, &self.variants);
            self.viewer.set_page_sources(pages);
        }
    }

    /// The annotations of the page at `position` that the frontend draws,
    /// the one being drawn there included.
    fn floating(&mut self, position: usize) -> Vec<Arc<Annotation>> {
        let draft = self
            .draft
            .as_ref()
            .filter(|d| d.position == position)
            .and_then(Draft::annotation)
            .map(Arc::new);
        // The one being moved or resized is drawn as it would become,
        // whoever draws it otherwise.
        let adjusted = self
            .adjust
            .as_ref()
            .filter(|a| a.position == position)
            .map(|a| (a.id, Arc::new(a.current.clone())));
        let mut all: Vec<Arc<Annotation>> = self
            .annotations(position)
            .into_iter()
            .filter(|(id, a)| {
                adjusted.as_ref().is_some_and(|(i, _)| i == id)
                    || (a.flags.is_visible() && floats(a))
            })
            .map(|(id, a)| match &adjusted {
                Some((i, current)) if *i == id => current.clone(),
                _ => a,
            })
            .collect();
        all.extend(draft);
        all
    }

    /// The annotation that could be moved or resized by a drag from `p`
    /// (viewport coordinates), and what of it the drag would take hold of:
    /// a handle of the picked one, the picked one itself, or another one
    /// under `p` that can be moved.
    fn grip_under(&mut self, p: ViewPoint) -> Option<(usize, AnnotId, Arc<Annotation>, Grip)> {
        if let Some((position, id)) = self.picked
            && let Some((_, a)) = self.picked_one()
            && a.is_adjustable()
        {
            let near = grips(&a).into_iter().find(|(_, at)| {
                self.viewer
                    .pdf_to_view(position, *at)
                    .is_some_and(|v| (v - p).length() <= GRIP_REACH)
            });
            if let Some((grip, _)) = near {
                return Some((position, id, a, grip));
            }
        }
        let (position, id) = self.annotation_at(p)?;
        let (_, a) = self
            .annotations(position)
            .into_iter()
            .find(|(i, _)| *i == id)?;
        a.is_adjustable().then_some((position, id, a, Grip::Move))
    }

    /// What a drag from `p` would take hold of, if anything: for choosing
    /// the pointer's shape, and whether a drag is to move something rather
    /// than select text.
    pub fn grip_at(&mut self, p: ViewPoint) -> Option<Grip> {
        self.grip_under(p).map(|(_, _, _, grip)| grip)
    }

    /// Where the handles of the picked annotation are on screen, if it can
    /// be resized.
    pub fn picked_handles(&mut self) -> Vec<ViewPoint> {
        let Some((position, _)) = self.picked else {
            return Vec::new();
        };
        let Some((_, a)) = self.picked_one().filter(|(_, a)| a.is_adjustable()) else {
            return Vec::new();
        };
        // While it is dragged, where it is now.
        let shown = match &self.adjust {
            Some(adjust) => grips(&adjust.current),
            None => grips(&a),
        };
        shown
            .into_iter()
            .filter_map(|(_, at)| self.viewer.pdf_to_view(position, at))
            .collect()
    }

    /// Starts moving or resizing what [`grip_at`](Self::grip_at) finds at
    /// `p`, which becomes the picked annotation. Returns whether there was
    /// something. Follow with [`adjust_to`](Self::adjust_to) as the pointer
    /// moves and [`finish_adjust`](Self::finish_adjust) when the button is
    /// let go. Errors as
    /// [`remove_picked_annotation`](Self::remove_picked_annotation).
    pub fn begin_adjust(
        &mut self,
        scheduler: &Scheduler,
        p: ViewPoint,
    ) -> Result<bool, WriteError> {
        let Some((position, id, a, grip)) = self.grip_under(p) else {
            return Ok(false);
        };
        let Some(from) = self.viewer.view_to_pdf(position, p) else {
            return Ok(false);
        };
        // One the renderer draws must be out of the page while it is
        // dragged about.
        let ready = self.page_without(id, &a)?;
        if let (Some((page, _, _, _)), Some(origin)) = (&ready, a.origin) {
            self.lifted = Some((*page, origin));
        }
        self.picked = Some((position, id));
        self.selection = None;
        self.adjust = Some(Adjust {
            id,
            position,
            grip,
            from,
            current: (*a).clone(),
            original: a,
        });
        self.sync_variants_with(scheduler, ready);
        Ok(true)
    }

    /// The pointer of a drag that moves or resizes is at `p` now. The
    /// pointer counts as far as its page goes.
    pub fn adjust_to(&mut self, p: ViewPoint) {
        let Some(adjust) = self.adjust.as_mut() else {
            return;
        };
        let (Some(page), Some(at)) = (
            self.viewer.pages().get(adjust.position),
            self.viewer.view_to_pdf(adjust.position, p),
        ) else {
            return;
        };
        let bounds = page.displayed().visible_box();
        let at = PdfPoint::new(
            at.x.clamp(bounds.min.x, bounds.max.x),
            at.y.clamp(bounds.min.y, bounds.max.y),
        );
        let original = &adjust.original;
        adjust.current = match adjust.grip {
            Grip::Move => {
                let by = at - adjust.from;
                original.transformed(|q| q + by)
            }
            Grip::Corner(i) => {
                let all = corners(original.frame());
                let (held, fixed) = (all[i % 4], all[(i + 2) % 4]);
                // A frame of no width (a straight stroke) keeps it.
                let scale = |held: f64, fixed: f64, at: f64| {
                    if (held - fixed).abs() > 1e-6 {
                        (at - fixed) / (held - fixed)
                    } else {
                        1.0
                    }
                };
                let (sx, sy) = (scale(held.x, fixed.x, at.x), scale(held.y, fixed.y, at.y));
                original.transformed(|q| {
                    PdfPoint::new(
                        fixed.x + (q.x - fixed.x) * sx,
                        fixed.y + (q.y - fixed.y) * sy,
                    )
                })
            }
            Grip::End(i) => {
                let mut moved = (**original).clone();
                if let AnnotKind::Line { from, to } = &mut moved.kind {
                    *(if i == 0 { from } else { to }) = at;
                }
                // Through `transformed` for the area and the look.
                moved.transformed(|q| q)
            }
        };
    }

    /// Whether something is being moved or resized.
    pub fn is_adjusting(&self) -> bool {
        self.adjust.is_some()
    }

    /// Gives up moving or resizing: the annotation is as it was.
    pub fn cancel_adjust(&mut self, scheduler: &Scheduler) {
        self.adjust = None;
        self.lifted = None;
        self.sync_variants(scheduler);
    }

    /// The drag has ended: the annotation stays as it is now. Returns
    /// whether it changed.
    pub fn finish_adjust(&mut self, scheduler: &Scheduler) -> bool {
        let Some(adjust) = self.adjust.take() else {
            return false;
        };
        let mut changed = false;
        let moved = adjust.current.kind != adjust.original.kind
            || adjust.current.rect != adjust.original.rect;
        if moved {
            let mut a = adjust.current;
            a.modified = Some(now());
            changed = self.edit.change_annotation(adjust.id, a);
        }
        // The edit keeps it out of the page from here on, or nothing does.
        self.lifted = None;
        self.sync_variants(scheduler);
        changed
    }

    /// Starts drawing at `p` (viewport coordinates) with a pen of
    /// `line_width` points in `color`. Returns whether it did: `p` must be
    /// on a page. Follow with [`extend_draft`](Self::extend_draft) as the
    /// pointer moves and [`finish_draft`](Self::finish_draft) when the
    /// button is let go; meanwhile [`overlay`](Self::overlay) shows what is
    /// drawn so far.
    pub fn begin_draft(
        &mut self,
        kind: DraftKind,
        color: Color,
        line_width: f64,
        p: ViewPoint,
    ) -> bool {
        let Some((position, _)) = self.viewer.hit_test(p) else {
            return false;
        };
        let (Some(at), Some(next)) = (
            self.viewer.view_to_pdf(position, p),
            self.viewer
                .view_to_pdf(position, p + specto_geom::ViewVector::new(1.0, 0.0)),
        ) else {
            return false;
        };
        self.picked = None;
        self.selection = None;
        self.draft = Some(Draft {
            kind,
            position,
            color,
            line_width,
            points: vec![at],
            step: (next - at).length().max(0.01),
        });
        true
    }

    /// The pointer of a drag that draws is at `p` now. The drawing stays on
    /// the page it started on.
    pub fn extend_draft(&mut self, p: ViewPoint) {
        let Some(draft) = self.draft.as_mut() else {
            return;
        };
        let Some(page) = self.viewer.pages().get(draft.position) else {
            return;
        };
        let Some(at) = self.viewer.view_to_pdf(draft.position, p) else {
            return;
        };
        let bounds = page.displayed().visible_box();
        let at = PdfPoint::new(
            at.x.clamp(bounds.min.x, bounds.max.x),
            at.y.clamp(bounds.min.y, bounds.max.y),
        );
        match draft.kind {
            // Not every twitch of the pointer: a point about every pixel.
            DraftKind::Ink => {
                if draft
                    .points
                    .last()
                    .is_none_or(|l| (at - *l).length() >= draft.step)
                {
                    draft.points.push(at);
                }
            }
            _ => {
                draft.points.truncate(1);
                draft.points.push(at);
            }
        }
    }

    /// Whether something is being drawn.
    pub fn is_drafting(&self) -> bool {
        self.draft.is_some()
    }

    /// Gives up what is being drawn.
    pub fn cancel_draft(&mut self) {
        self.draft = None;
    }

    /// The drag has ended: what was drawn becomes an annotation. Returns
    /// whether there is one (not for a shape of no size).
    pub fn finish_draft(&mut self) -> bool {
        let Some(mut draft) = self.draft.take() else {
            return false;
        };
        if !draft.is_worth_keeping() {
            return false;
        }
        if draft.kind == DraftKind::Ink {
            // The points a pointer leaves are far more than the stroke
            // needs: those within a third of a pixel of the line between
            // their neighbours go.
            draft.points = simplify(&draft.points, draft.step / 3.0);
        }
        let Some(mut a) = draft.annotation() else {
            return false;
        };
        let Some(&p) = self.edit.pages().get(draft.position) else {
            return false;
        };
        a.modified = Some(now());
        self.edit.add_annotation(p.source, p.page, a);
        true
    }

    /// What to draw over the page at `position`, bottom to top: the
    /// annotations the frontend draws, where they are on screen now.
    pub fn overlay(&mut self, position: usize) -> Vec<ViewShape> {
        let annotations = self.floating(position);
        let viewer = &self.viewer;
        on_screen(&annotations, |p| viewer.pdf_to_view(position, p))
    }

    /// Like [`overlay`](Self::overlay), for the page drawn `width` by
    /// `height` with its top left corner at the origin: a thumbnail.
    pub fn overlay_in(&mut self, position: usize, width: f64, height: f64) -> Vec<ViewShape> {
        let annotations = self.floating(position);
        let Some(page) = self.viewer.pages().get(position) else {
            return Vec::new();
        };
        let geometry = page.displayed();
        let (to_page, size) = (geometry.pdf_to_page(), geometry.size());
        on_screen(&annotations, |p| {
            let q = to_page.transform_point(p);
            Some(ViewPoint::new(
                q.x * width / size.width,
                q.y * height / size.height,
            ))
        })
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
        let (id, a) = self.picked_one()?;
        // While it is dragged about, where it is now.
        let r = match &self.adjust {
            Some(adjust) if adjust.id == id => adjust.current.rect,
            _ => a.rect,
        };
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

    /// For an edit to the annotation `a` after which the renderer must not
    /// draw it as it is any more (it is deleted, changed, or drawn by the
    /// frontend from then on): the copy of its page without it, written
    /// first, so that nothing changes if that can't be done. `None` if the
    /// renderer doesn't draw it now either.
    fn page_without(
        &self,
        id: AnnotId,
        a: &Arc<Annotation>,
    ) -> Result<Option<WrittenVariant>, WriteError> {
        let (AnnotId::File { source, page, .. }, Some(origin)) = (id, a.origin) else {
            return Ok(None);
        };
        if floats(a) {
            return Ok(None);
        }
        let mut hidden = self.hidden(source, page);
        if !hidden.contains(&origin) {
            hidden.push(origin);
        }
        hidden.sort_by_key(|o| o.index);
        let mut baked = self.baked(source, page);
        baked.retain(|b| !Arc::ptr_eq(b, a));
        let file = self.variant_file((source, page), &hidden, &baked)?;
        Ok(Some(((source, page), hidden, baked, file)))
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
        let ready = self.page_without(id, &a)?;
        self.picked = None;
        let removed = self.edit.remove_annotation(id, &a);
        if removed {
            self.sync_variants_with(scheduler, ready);
        }
        Ok(removed)
    }

    /// Gives the picked annotation the colour `color`; it stays picked.
    /// Returns whether it changed: not if nothing is picked, it has that
    /// colour already, or it is of a kind only its file knows how to draw.
    /// Errors as [`remove_picked_annotation`](Self::remove_picked_annotation).
    pub fn recolor_picked_annotation(
        &mut self,
        scheduler: &Scheduler,
        color: Color,
    ) -> Result<bool, WriteError> {
        let Some((id, a)) = self.picked_one() else {
            return Ok(false);
        };
        if a.color == Some(color) {
            return Ok(false);
        }
        // Not one whose look in the file may hold more than its values
        // say: a line can have arrowheads and a rectangle a dashed border,
        // which drawing them anew here would lose.
        if !a.is_redrawable() {
            return Ok(false);
        }
        let ready = self.page_without(id, &a)?;
        let mut changed = (*a).clone();
        changed.color = Some(color);
        changed.modified = Some(now());
        // Drawn from its values from now on, and given a new look when saved.
        changed.appearance = Appearance::Generated;
        let done = self.edit.change_annotation(id, changed);
        if done {
            self.sync_variants_with(scheduler, ready);
        }
        Ok(done)
    }

    /// Puts a note at `p` (viewport coordinates) in `color`, and picks it.
    /// Returns whether it did: `p` must be on a page.
    pub fn add_note(&mut self, p: ViewPoint, color: Color) -> bool {
        let Some((position, _)) = self.viewer.hit_test(p) else {
            return false;
        };
        let (Some(at), Some(&page)) = (
            self.viewer.view_to_pdf(position, p),
            self.edit.pages().get(position),
        ) else {
            return false;
        };
        let mut note = Annotation::note(at, color);
        note.modified = Some(now());
        let id = self.edit.add_annotation(page.source, page.page, note);
        self.selection = None;
        self.picked = Some((position, id));
        true
    }

    /// Puts a text box of `text` at `p` (viewport coordinates), in
    /// Helvetica of `size` points and `color`, and picks it. It is up to
    /// 220 points wide, less where the page ends. Returns whether it did:
    /// `p` must be on a page, and there must be text.
    pub fn add_text_box(
        &mut self,
        scheduler: &Scheduler,
        p: ViewPoint,
        text: &str,
        size: f64,
        color: Color,
    ) -> bool {
        if text.trim().is_empty() {
            return false;
        }
        let Some((position, _)) = self.viewer.hit_test(p) else {
            return false;
        };
        let (Some(at), Some(view_page), Some(&page)) = (
            self.viewer.view_to_pdf(position, p),
            self.viewer.pages().get(position),
            self.edit.pages().get(position),
        ) else {
            return false;
        };
        let room = view_page.displayed().visible_box().max.x - at.x - 4.0;
        let mut text_box = Annotation::text_box(at, room.clamp(40.0, 220.0), text, size, color);
        text_box.modified = Some(now());
        let id = self.edit.add_annotation(page.source, page.page, text_box);
        self.selection = None;
        self.picked = Some((position, id));
        self.sync_variants(scheduler);
        true
    }

    /// The text of the picked annotation, to be edited: what a text box
    /// shows, the text of a note, or the comment that goes with any other
    /// (empty if it has none). `None` if nothing is picked, or its text
    /// can't be changed here (a text box of another program, whose look
    /// wouldn't follow).
    pub fn picked_text(&mut self) -> Option<String> {
        let (_, a) = self.picked_one()?;
        text_is_editable(&a).then(|| a.contents.clone().unwrap_or_default())
    }

    /// Whether the picked annotation is a text box: its text is what it
    /// shows.
    pub fn picked_is_text_box(&mut self) -> bool {
        self.picked_one().is_some_and(|(_, a)| a.is_text_box())
    }

    /// Gives the picked annotation the text `text` (see
    /// [`picked_text`](Self::picked_text)). A text box left without text
    /// is deleted. Returns whether anything changed. Errors as
    /// [`remove_picked_annotation`](Self::remove_picked_annotation).
    pub fn set_picked_text(
        &mut self,
        scheduler: &Scheduler,
        text: &str,
    ) -> Result<bool, WriteError> {
        let Some((id, a)) = self.picked_one() else {
            return Ok(false);
        };
        if !text_is_editable(&a) || a.contents.as_deref().unwrap_or("") == text {
            return Ok(false);
        }
        if a.is_text_box() && text.trim().is_empty() {
            return self.remove_picked_annotation(scheduler);
        }
        let mut changed = (*a).clone();
        changed.contents = Some(text.to_owned()).filter(|t| !t.is_empty());
        changed.modified = Some(now());
        // A text box shows its text, so it gets a new look and its page a
        // new copy; for the others the text is beside what they show.
        let ready = if a.is_text_box() {
            changed.fit_text();
            self.page_without(id, &a)?
        } else {
            None
        };
        let done = self.edit.change_annotation(id, changed);
        if done {
            self.sync_variants_with(scheduler, ready);
        }
        Ok(done)
    }

    /// The colour of the picked annotation, if one is picked and has one.
    pub fn picked_color(&mut self) -> Option<Color> {
        self.picked_one()?.1.color
    }
}

/// Whether the text of `a` can be changed here: not that of a text box of
/// another program (its look, which shows the text, wouldn't follow), and
/// not where the file forbids changes.
fn text_is_editable(a: &Annotation) -> bool {
    let foreign_text_box = matches!(&a.kind, AnnotKind::Other { subtype } if subtype == "FreeText");
    a.flags.is_editable() && !foreign_text_box
}

/// The shapes of `annotations` where `at` puts their points.
fn on_screen(
    annotations: &[Arc<Annotation>],
    at: impl Fn(specto_geom::PdfPoint) -> Option<ViewPoint>,
) -> Vec<ViewShape> {
    let mut out = Vec::new();
    for a in annotations {
        for shape in shapes(a).unwrap_or_default() {
            let placed = match shape {
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
                Shape::Path {
                    path,
                    fill,
                    stroke,
                    opacity,
                } => (|| {
                    // How long a point is on screen, from two points one
                    // apart.
                    let one = path.start + specto_geom::PdfVector::new(1.0, 0.0);
                    let zoom = (at(one)? - at(path.start)?).length();
                    Some(ViewShape::Path {
                        path: path.map(&at)?,
                        fill,
                        stroke: stroke.map(|(color, width)| (color, width * zoom)),
                        opacity,
                    })
                })(),
            };
            out.extend(placed);
        }
    }
    out
}

/// `points` without those nearer than `tolerance` to the line between the
/// ones kept around them (Ramer, Douglas and Peucker's way); the first and
/// last stay.
fn simplify(points: &[PdfPoint], tolerance: f64) -> Vec<PdfPoint> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let mut keep = vec![false; points.len()];
    keep[0] = true;
    keep[points.len() - 1] = true;
    let mut spans = vec![(0, points.len() - 1)];
    while let Some((first, last)) = spans.pop() {
        let (a, b) = (points[first], points[last]);
        let ab = b - a;
        let length = ab.length();
        let far = (first + 1..last)
            .map(|i| {
                let ap = points[i] - a;
                let d = if length > 0.0 {
                    ab.cross(ap).abs() / length
                } else {
                    ap.length()
                };
                (i, d)
            })
            .max_by(|x, y| x.1.total_cmp(&y.1));
        if let Some((i, d)) = far
            && d > tolerance
        {
            keep[i] = true;
            spans.push((first, i));
            spans.push((i, last));
        }
    }
    points
        .iter()
        .zip(keep)
        .filter(|(_, k)| *k)
        .map(|(p, _)| *p)
        .collect()
}

/// The time now, in UTC.
fn now() -> Date {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    Date::from_unix(secs)
}
