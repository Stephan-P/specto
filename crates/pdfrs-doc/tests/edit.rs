use pdfrs_doc::{Edit, EditDoc, PageRef, Rotation, SourceId, UNDO_LIMIT};

const A: SourceId = SourceId(0);
const B: SourceId = SourceId(1);

/// The source page numbers, in order.
fn order(doc: &EditDoc) -> Vec<u32> {
    doc.pages().iter().map(|p| p.page).collect()
}

#[test]
fn rotating_turns_pages_and_wraps() {
    let mut doc = EditDoc::from_source(A, 3);
    assert!(!doc.is_modified());
    assert!(doc.rotate(&[0, 2, 2, 7], Rotation::Cw90));
    assert_eq!(doc.undo_edit(), Some(Edit::Rotate { pages: 2 }));
    assert!(doc.is_modified());
    doc.rotate(&[0], Rotation::Cw270);
    let r: Vec<Rotation> = doc.pages().iter().map(|p| p.rotation).collect();
    assert_eq!(r, [Rotation::None, Rotation::None, Rotation::Cw90]);
    assert!(!doc.rotate(&[], Rotation::Cw90));
    assert!(!doc.rotate(&[1], Rotation::None));
}

#[test]
fn deleting_keeps_at_least_one_page() {
    let mut doc = EditDoc::from_source(A, 4);
    assert!(doc.delete(&[3, 1]));
    assert_eq!(order(&doc), [0, 2]);
    assert!(!doc.delete(&[0, 1]), "can't delete every page");
    assert_eq!(doc.undo_edit(), Some(Edit::Delete { pages: 2 }));
}

#[test]
fn moving_keeps_the_moved_pages_in_order() {
    let mut doc = EditDoc::from_source(A, 6);
    // To the front.
    assert_eq!(doc.move_pages(&[4, 2], 0), Some(0..2));
    assert_eq!(order(&doc), [2, 4, 0, 1, 3, 5]);
    doc.undo();
    // Forwards: before page 5, so after 3 and 4.
    assert_eq!(doc.move_pages(&[0, 1], 5), Some(3..5));
    assert_eq!(order(&doc), [2, 3, 4, 0, 1, 5]);
    doc.undo();
    // To the end.
    assert_eq!(doc.move_pages(&[1], 99), Some(5..6));
    assert_eq!(order(&doc), [0, 2, 3, 4, 5, 1]);
    doc.undo();
    // Onto itself: no change, nothing to undo.
    assert_eq!(doc.move_pages(&[2, 3], 2), None);
    assert_eq!(doc.move_pages(&[2, 3], 4), None);
    assert_eq!(doc.undo_edit(), None);
    assert!(!doc.is_modified());
}

#[test]
fn inserting_pages_from_another_file() {
    let mut doc = EditDoc::from_source(A, 2);
    let other: Vec<PageRef> = (0..3).map(|i| PageRef::new(B, i)).collect();
    assert_eq!(doc.insert(1, other), Some(1..4));
    let sources: Vec<SourceId> = doc.pages().iter().map(|p| p.source).collect();
    assert_eq!(sources, [A, B, B, B, A]);
    assert_eq!(doc.sources(), [A, B]);
    doc.delete(&[0, 4]);
    assert_eq!(doc.sources(), [B]);
    assert_eq!(doc.insert(0, Vec::new()), None);
}

#[test]
fn undo_and_redo() {
    let mut doc = EditDoc::from_source(A, 3);
    let original = doc.pages().to_vec();
    doc.delete(&[0]);
    let deleted = doc.pages().to_vec();
    doc.rotate(&[0], Rotation::Cw180);
    assert_eq!(doc.undo(), Some(Edit::Rotate { pages: 1 }));
    assert_eq!(doc.pages(), deleted);
    assert_eq!(doc.redo_edit(), Some(Edit::Rotate { pages: 1 }));
    assert_eq!(doc.undo(), Some(Edit::Delete { pages: 1 }));
    assert_eq!(doc.pages(), original);
    assert!(!doc.is_modified(), "back where it was opened");
    assert_eq!(doc.undo(), None);
    assert_eq!(doc.redo(), Some(Edit::Delete { pages: 1 }));
    assert!(doc.is_modified());
    // A new edit drops what could be redone.
    doc.move_pages(&[1], 0);
    assert_eq!(doc.redo_edit(), None);
    assert_eq!(doc.redo(), None);
}

#[test]
fn modified_is_relative_to_the_last_save() {
    let mut doc = EditDoc::from_source(A, 3);
    doc.rotate(&[0], Rotation::Cw90);
    doc.mark_saved();
    assert!(!doc.is_modified());
    doc.undo();
    assert!(doc.is_modified(), "the saved file has the rotation");
    doc.redo();
    assert!(!doc.is_modified());
    doc.undo();
    // A different edit from the same starting point isn't the saved state.
    doc.rotate(&[0], Rotation::Cw90);
    assert!(doc.is_modified());
}

#[test]
fn edits_during_a_save_stay_unsaved() {
    let mut doc = EditDoc::from_source(A, 3);
    doc.rotate(&[0], Rotation::Cw90);
    let saving = doc.version();
    doc.rotate(&[1], Rotation::Cw90);
    doc.mark_saved_version(saving);
    assert!(doc.is_modified());
    doc.undo();
    assert!(!doc.is_modified());
}

#[test]
fn history_is_limited() {
    let mut doc = EditDoc::from_source(A, 2);
    for _ in 0..UNDO_LIMIT + 10 {
        doc.rotate(&[0], Rotation::Cw90);
    }
    let mut undone = 0;
    while doc.undo().is_some() {
        undone += 1;
    }
    assert_eq!(undone, UNDO_LIMIT);
    assert!(doc.is_modified(), "the oldest edits can't be undone");
}

/// A small deterministic generator, so failures can be replayed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }

    fn some(&mut self, n: usize) -> Vec<usize> {
        (0..1 + self.below(4)).map(|_| self.below(n + 1)).collect()
    }
}

#[test]
fn random_edits_then_undoing_all_restores_the_original() {
    for seed in 1..=200u64 {
        let mut rng = Rng(seed);
        let mut doc = EditDoc::from_source(A, 1 + rng.below(8) as u32);
        let original = doc.pages().to_vec();
        let mut states = vec![original.clone()];
        for _ in 0..rng.below(30) {
            let n = doc.page_count();
            let changed = match rng.below(4) {
                0 => doc.rotate(
                    &rng.some(n),
                    Rotation::from_quarter_turns(rng.below(4) as i64),
                ),
                1 => doc.delete(&rng.some(n)),
                2 => {
                    let which = rng.some(n);
                    doc.move_pages(&which, rng.below(n + 1)).is_some()
                }
                _ => {
                    let pages = (0..rng.below(3) as u32)
                        .map(|i| PageRef::new(B, i))
                        .collect();
                    doc.insert(rng.below(n + 1), pages).is_some()
                }
            };
            assert!(doc.page_count() >= 1, "seed {seed}");
            if changed {
                states.push(doc.pages().to_vec());
            } else {
                assert_eq!(doc.pages(), states.last().unwrap(), "seed {seed}");
            }
        }
        // Undo walks back through every state, redo forward again.
        for state in states.iter().rev().skip(1) {
            assert!(doc.undo().is_some(), "seed {seed}");
            assert_eq!(doc.pages(), state, "seed {seed}");
        }
        assert_eq!(doc.undo(), None, "seed {seed}");
        assert!(!doc.is_modified(), "seed {seed}");
        for state in states.iter().skip(1) {
            assert!(doc.redo().is_some(), "seed {seed}");
            assert_eq!(doc.pages(), state, "seed {seed}");
        }
    }
}
