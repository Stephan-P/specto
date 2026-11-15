use std::sync::Arc;

use specto_annot::{AnnotKind, Annotation, Color, MarkupStyle, Origin};
use specto_doc::{AnnotId, AnnotType, Edit, EditDoc, PageAnnotChanges, Rotation, SourceId};
use specto_geom::{PdfBox, PdfPoint, PdfQuad};

const A: SourceId = SourceId(0);
const B: SourceId = SourceId(1);

fn highlight(x: f64) -> Annotation {
    let quad = PdfQuad::new([
        PdfPoint::new(x, 0.0),
        PdfPoint::new(x + 10.0, 0.0),
        PdfPoint::new(x + 10.0, 5.0),
        PdfPoint::new(x, 5.0),
    ]);
    Annotation::text_markup(
        MarkupStyle::Highlight,
        vec![quad],
        Color::new(1.0, 1.0, 0.0),
    )
    .expect("one quad")
}

/// What a file has on a page: annotations at these entries of `/Annots`.
fn imported(indices: &[u32]) -> Vec<Arc<Annotation>> {
    indices
        .iter()
        .map(|&index| {
            let rect = PdfBox::new(PdfPoint::new(0.0, 0.0), PdfPoint::new(1.0, 1.0));
            let mut a = Annotation::new(AnnotKind::Square { fill: None }, rect);
            a.origin = Some(Origin {
                index,
                object: None,
            });
            Arc::new(a)
        })
        .collect()
}

fn file(page: u32, index: u32) -> AnnotId {
    AnnotId::File {
        source: A,
        page,
        index,
    }
}

fn ids(doc: &EditDoc, page: u32, imported: &[Arc<Annotation>]) -> Vec<AnnotId> {
    doc.annotations(A, page, imported)
        .into_iter()
        .map(|(id, _)| id)
        .collect()
}

#[test]
fn a_document_starts_with_what_its_files_have() {
    let doc = EditDoc::from_source(A, 3);
    let on_page = imported(&[1, 4]);
    assert_eq!(ids(&doc, 0, &on_page), [file(0, 1), file(0, 4)]);
    assert!(!doc.has_annotation_edits());
    assert!(doc.annotation_changes(A, 0).is_empty());
}

#[test]
fn adding_removing_and_changing() {
    let mut doc = EditDoc::from_source(A, 3);
    let on_page = imported(&[1, 4]);

    let new = doc.add_annotation(A, 0, highlight(10.0));
    assert_eq!(
        doc.undo_edit(),
        Some(Edit::AddAnnotation(AnnotType::Highlight))
    );
    assert!(doc.is_modified());
    let other_page = doc.add_annotation(A, 1, highlight(20.0));
    assert_ne!(new, other_page);
    // New ones lie on top of the file's, and only on their own page.
    assert_eq!(ids(&doc, 0, &on_page), [file(0, 1), file(0, 4), new]);
    assert_eq!(ids(&doc, 1, &[]), [other_page]);
    assert_eq!(ids(&doc, 2, &[]), []);
    assert!(doc.annotations(B, 0, &[]).is_empty());

    // Deleting one of the file's.
    assert!(doc.remove_annotation(file(0, 1), &on_page[0]));
    assert_eq!(
        doc.undo_edit(),
        Some(Edit::RemoveAnnotation(AnnotType::Square))
    );
    assert!(
        !doc.remove_annotation(file(0, 1), &on_page[0]),
        "deleted twice"
    );
    assert_eq!(ids(&doc, 0, &on_page), [file(0, 4), new]);

    // Changing one of the file's, and a new one.
    let mut moved = (*on_page[1]).clone();
    moved.opacity = 0.5;
    assert!(doc.change_annotation(file(0, 4), moved.clone()));
    assert!(doc.change_annotation(new, highlight(30.0)));
    assert_eq!(
        doc.undo_edit(),
        Some(Edit::ChangeAnnotation(AnnotType::Highlight))
    );
    let now = doc.annotations(A, 0, &on_page);
    assert_eq!(now[0].1.opacity, 0.5);
    assert_eq!(*now[1].1, highlight(30.0));

    assert_eq!(
        doc.annotation_changes(A, 0),
        PageAnnotChanges {
            removed: vec![1],
            replaced: vec![(4, Arc::new(moved))],
            added: vec![Arc::new(highlight(30.0))],
        }
    );
    assert_eq!(doc.annotation_changes(A, 1).added.len(), 1);
    assert!(doc.annotation_changes(B, 0).is_empty());

    // Deleting a new one leaves nothing to save for it.
    assert!(doc.remove_annotation(other_page, &highlight(20.0)));
    assert!(!doc.remove_annotation(other_page, &highlight(20.0)));
    assert!(!doc.change_annotation(other_page, highlight(20.0)));
    assert!(doc.annotation_changes(A, 1).is_empty());
}

#[test]
fn undo_goes_through_page_and_annotation_edits_in_order() {
    let mut doc = EditDoc::from_source(A, 3);
    let on_page = imported(&[0]);
    let first = doc.add_annotation(A, 0, highlight(10.0));
    doc.rotate(&[0], Rotation::Cw90);
    doc.remove_annotation(file(0, 0), &on_page[0]);
    doc.delete(&[1]);
    assert_eq!(ids(&doc, 0, &on_page), [first]);

    assert_eq!(doc.undo(), Some(Edit::Delete { pages: 1 }));
    assert_eq!(ids(&doc, 0, &on_page), [first]);
    assert_eq!(doc.undo(), Some(Edit::RemoveAnnotation(AnnotType::Square)));
    assert_eq!(ids(&doc, 0, &on_page), [file(0, 0), first]);
    assert_eq!(doc.pages()[0].rotation, Rotation::Cw90);
    assert_eq!(doc.undo(), Some(Edit::Rotate { pages: 1 }));
    assert_eq!(doc.undo(), Some(Edit::AddAnnotation(AnnotType::Highlight)));
    assert_eq!(ids(&doc, 0, &on_page), [file(0, 0)]);
    assert!(!doc.is_modified() && !doc.has_annotation_edits());
    assert_eq!(doc.undo(), None);

    for _ in 0..4 {
        doc.redo();
    }
    assert_eq!(ids(&doc, 0, &on_page), [first]);
    assert_eq!(doc.page_count(), 2);

    // A new annotation after undoing one doesn't get the old one's name.
    doc.undo();
    doc.undo();
    doc.undo();
    doc.undo();
    let second = doc.add_annotation(A, 0, highlight(40.0));
    assert_ne!(first, second);
}

#[test]
fn annotations_stay_with_their_page() {
    let mut doc = EditDoc::from_source(A, 3);
    let new = doc.add_annotation(A, 2, highlight(10.0));
    doc.move_pages(&[2], 0);
    doc.rotate(&[0], Rotation::Cw180);
    assert_eq!(ids(&doc, 2, &[]), [new]);
    // The page deleted and back.
    doc.delete(&[0]);
    assert_eq!(doc.annotation_changes(A, 2).added.len(), 1);
    doc.undo();
    assert_eq!(ids(&doc, 2, &[]), [new]);
}

#[test]
fn saved_and_modified() {
    let mut doc = EditDoc::from_source(A, 1);
    doc.add_annotation(A, 0, highlight(10.0));
    doc.mark_saved();
    assert!(!doc.is_modified());
    // Still there to be written the next time.
    assert!(doc.has_annotation_edits());
    doc.undo();
    assert!(doc.is_modified());
    doc.redo();
    assert!(!doc.is_modified());
}
