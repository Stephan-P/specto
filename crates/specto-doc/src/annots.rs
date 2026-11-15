//! The annotations of a document being edited.
//!
//! The annotations a file has are read from it when wanted (see
//! `specto_annot::import`) and never change. What an [`EditDoc`] keeps is
//! only what the user did: which of those are deleted or changed, and the
//! new ones. That is small however many annotations the files have, costs
//! nothing for pages nobody looked at, and is what saving needs to know.
//!
//! Annotations belong to a page of a file, not to a place in the document:
//! they stay with their page when it is moved or rotated, and are back
//! when a deleted page is. Copies of one page share them.

use std::collections::BTreeMap;
use std::sync::Arc;

use specto_annot::Annotation;

use crate::{Edit, EditDoc, SourceId};

/// Names one annotation of a document.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AnnotId {
    /// One a file has: entry `index` of the `/Annots` of page `page` of
    /// `source` (an [`Origin`](specto_annot::Origin)'s index).
    File {
        source: SourceId,
        page: u32,
        index: u32,
    },
    /// One made while editing; the number is never used again.
    New(u64),
}

/// What was done to the annotations, relative to the files.
#[derive(Clone, Debug, Default)]
pub(crate) struct AnnotEdits {
    /// Annotations of the files that are deleted (`None`) or changed, by
    /// source, page and index.
    changed: BTreeMap<(SourceId, u32, u32), Option<Arc<Annotation>>>,
    /// New ones, in the order they were made (later ones lie on top).
    added: Vec<Added>,
}

#[derive(Clone, Debug)]
struct Added {
    id: u64,
    source: SourceId,
    page: u32,
    annotation: Arc<Annotation>,
}

/// What to do to the annotations of one page of a file when saving it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PageAnnotChanges {
    /// Entries of the page's `/Annots` to drop, by index.
    pub removed: Vec<u32>,
    /// Entries to write anew, by index.
    pub replaced: Vec<(u32, Arc<Annotation>)>,
    /// Annotations to append, in order.
    pub added: Vec<Arc<Annotation>>,
}

impl PageAnnotChanges {
    pub fn is_empty(&self) -> bool {
        self.removed.is_empty() && self.replaced.is_empty() && self.added.is_empty()
    }
}

impl EditDoc {
    /// The annotations of page `page` of `source` as edited, given the ones
    /// the file has there (`imported`, in the file's order): bottom to top.
    pub fn annotations(
        &self,
        source: SourceId,
        page: u32,
        imported: &[Arc<Annotation>],
    ) -> Vec<(AnnotId, Arc<Annotation>)> {
        let edits = &self.annots;
        let of_file = imported.iter().filter_map(|a| {
            let index = a.origin?.index;
            let id = AnnotId::File {
                source,
                page,
                index,
            };
            match edits.changed.get(&(source, page, index)) {
                None => Some((id, a.clone())),
                Some(None) => None,
                Some(Some(changed)) => Some((id, changed.clone())),
            }
        });
        let added = edits
            .added
            .iter()
            .filter(|a| a.source == source && a.page == page)
            .map(|a| (AnnotId::New(a.id), a.annotation.clone()));
        of_file.chain(added).collect()
    }

    /// Whether any annotation was added, changed or deleted (and not
    /// undone), whatever was saved since.
    pub fn has_annotation_edits(&self) -> bool {
        !self.annots.changed.is_empty() || !self.annots.added.is_empty()
    }

    /// What saving has to do to the annotations of page `page` of `source`.
    pub fn annotation_changes(&self, source: SourceId, page: u32) -> PageAnnotChanges {
        let mut out = PageAnnotChanges::default();
        let all = (source, page, 0)..=(source, page, u32::MAX);
        for (&(_, _, index), change) in self.annots.changed.range(all) {
            match change {
                None => out.removed.push(index),
                Some(a) => out.replaced.push((index, a.clone())),
            }
        }
        out.added = self
            .annots
            .added
            .iter()
            .filter(|a| a.source == source && a.page == page)
            .map(|a| a.annotation.clone())
            .collect();
        out
    }

    /// Puts a new annotation on page `page` of `source`, on top of the
    /// others.
    pub fn add_annotation(
        &mut self,
        source: SourceId,
        page: u32,
        annotation: Annotation,
    ) -> AnnotId {
        self.add_annotations(vec![(source, page, annotation)])[0]
    }

    /// Puts new annotations on their pages (source, page of it, annotation)
    /// as one edit, named after the first: marked text that runs over
    /// several pages is one annotation per page. Without any, nothing
    /// happens.
    pub fn add_annotations(
        &mut self,
        annotations: Vec<(SourceId, u32, Annotation)>,
    ) -> Vec<AnnotId> {
        let Some((_, _, first)) = annotations.first() else {
            return Vec::new();
        };
        self.record(Edit::AddAnnotation(first.annot_type()));
        let edits = Arc::make_mut(&mut self.annots);
        annotations
            .into_iter()
            .map(|(source, page, annotation)| {
                let id = self.next_annot;
                self.next_annot += 1;
                edits.added.push(Added {
                    id,
                    source,
                    page,
                    annotation: Arc::new(annotation),
                });
                AnnotId::New(id)
            })
            .collect()
    }

    /// Deletes the annotation `id`, which is `annotation` (as
    /// [`annotations`](Self::annotations) gave it). Returns whether there
    /// was something to delete.
    pub fn remove_annotation(&mut self, id: AnnotId, annotation: &Annotation) -> bool {
        if !self.has_annotation(id) {
            return false;
        }
        self.record(Edit::RemoveAnnotation(annotation.annot_type()));
        let edits = Arc::make_mut(&mut self.annots);
        match id {
            AnnotId::File {
                source,
                page,
                index,
            } => {
                edits.changed.insert((source, page, index), None);
            }
            AnnotId::New(n) => edits.added.retain(|a| a.id != n),
        }
        true
    }

    /// Replaces the annotation `id` with `annotation`: the same one,
    /// changed. Returns whether there was one to change.
    pub fn change_annotation(&mut self, id: AnnotId, annotation: Annotation) -> bool {
        if !self.has_annotation(id) {
            return false;
        }
        self.record(Edit::ChangeAnnotation(annotation.annot_type()));
        let annotation = Arc::new(annotation);
        let edits = Arc::make_mut(&mut self.annots);
        match id {
            AnnotId::File {
                source,
                page,
                index,
            } => {
                edits
                    .changed
                    .insert((source, page, index), Some(annotation));
            }
            AnnotId::New(n) => {
                if let Some(a) = edits.added.iter_mut().find(|a| a.id == n) {
                    a.annotation = annotation;
                }
            }
        }
        true
    }

    /// Whether `id` can be there: a new one that is, or one of a file that
    /// isn't deleted (whether the file has it, only the caller knows).
    fn has_annotation(&self, id: AnnotId) -> bool {
        match id {
            AnnotId::File {
                source,
                page,
                index,
            } => !matches!(self.annots.changed.get(&(source, page, index)), Some(None)),
            AnnotId::New(n) => self.annots.added.iter().any(|a| a.id == n),
        }
    }
}
