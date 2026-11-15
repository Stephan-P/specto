use specto_annot::{AnnotKind, AnnotType, Appearance, Color, Flags, MarkupStyle, Origin, import};
use specto_geom::{PdfBox, PdfPoint};
use specto_source::SourceDocument;

/// Builds a PDF from numbered object bodies (`objects[i]` becomes object `i + 1`), with object 1 as
/// the catalog. The xref is left out, and hayro rebuilds it.
fn pdf(objects: &[&str]) -> SourceDocument {
    let mut out = String::from("%PDF-1.7\n");
    for (i, body) in objects.iter().enumerate() {
        out.push_str(&format!("{} 0 obj\n{body}\nendobj\n", i + 1));
    }
    out.push_str(&format!(
        "trailer << /Root 1 0 R /Size {} >>\n%%EOF\n",
        objects.len() + 1
    ));
    SourceDocument::open(out.into_bytes(), None).expect("the test PDF opens")
}

fn sample() -> SourceDocument {
    pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 600 800] >>",
        // 3: page 1. A link, a highlight, a number (not an annotation), an underline written
        // straight into the array, ink, a note with its popup, a square, a line, a form field.
        "<< /Type /Page /Parent 2 0 R /Annots [5 0 R 6 0 R 42
            << /Subtype /Underline /Rect [10 10 50 20] /QuadPoints [10 20 50 20 10 10 50 10] >>
            7 0 R 8 0 R 9 0 R 10 0 R 11 0 R 12 0 R] >>",
        "<< /Type /Page /Parent 2 0 R >>",
        "<< /Subtype /Link /Rect [0 0 10 10] /A << /S /URI /URI (https://example.org/) >> >>",
        // 6: a highlight over two lines, quads in the usual Z order, with everything set.
        "<< /Type /Annot /Subtype /Highlight /Rect [100 700 300 730]
            /QuadPoints [100 730 300 730 100 715 300 715  100 715 200 715 100 700 200 700]
            /C [1 0.9 0] /CA 0.5 /T (Ada) /Contents <FEFF004E006F00740065> /NM (h1)
            /M (D:20240305142210+01'00') /F 4 /AP << /N 13 0 R >> >>",
        // 7: ink, two strokes, the second with a stray number.
        "<< /Subtype /Ink /Rect [0 0 100 100] /InkList [[10 10 20 30 40 20] [50 50 60 60 7]]
            /C [0 0 1] /BS << /W 2.5 >> >>",
        // 8: a note: not a kind with parameters here.
        "<< /Subtype /Text /Rect [400 400 420 420] /Contents (Check this) /Popup 9 0 R /F 68 >>",
        "<< /Subtype /Popup /Rect [420 300 600 400] /Parent 8 0 R >>",
        // 10: a square, its rectangle given backwards, gray outline, CMYK fill.
        "<< /Subtype /Square /Rect [300 200 200 100] /C [0.5] /IC [0 1 1 0] /Border [0 0 3] >>",
        "<< /Subtype /Line /Rect [0 0 50 50] /L [5 5 45 45] >>",
        "<< /Subtype /Widget /FT /Tx /Rect [0 0 50 50] >>",
        "<< /Type /XObject /Subtype /Form /BBox [0 0 200 30] /Length 0 >>\nstream\n\nendstream",
    ])
}

#[test]
fn imports_markup_and_leaves_the_rest() {
    let doc = sample();
    let annots = import(&doc, 0);
    let types: Vec<AnnotType> = annots.iter().map(|a| a.annot_type()).collect();
    assert_eq!(
        types,
        [
            AnnotType::Highlight,
            AnnotType::Underline,
            AnnotType::Ink,
            AnnotType::Other,
            AnnotType::Square,
            AnnotType::Line,
        ]
    );
    // Positions count every entry of the array, also the link and the number.
    let origins: Vec<Option<Origin>> = annots.iter().map(|a| a.origin).collect();
    let at = |index, object: Option<u32>| {
        Some(Origin {
            index,
            object: object.map(|n| (n, 0)),
        })
    };
    assert_eq!(
        origins,
        [
            at(1, Some(6)),
            at(3, None),
            at(4, Some(7)),
            at(5, Some(8)),
            at(7, Some(10)),
            at(8, Some(11)),
        ]
    );
    assert!(import(&doc, 1).is_empty());
    assert!(import(&doc, 2).is_empty());
}

#[test]
fn reads_what_a_highlight_has() {
    let annots = import(&sample(), 0);
    let h = &annots[0];
    assert_eq!(
        h.rect,
        PdfBox::new(PdfPoint::new(100.0, 700.0), PdfPoint::new(300.0, 730.0))
    );
    assert_eq!(h.color, Some(Color::new(1.0, 0.9, 0.0)));
    assert_eq!(h.opacity, 0.5);
    assert_eq!(h.author.as_deref(), Some("Ada"));
    assert_eq!(h.contents.as_deref(), Some("Note"));
    assert_eq!(h.name.as_deref(), Some("h1"));
    assert_eq!(
        h.modified.map(|d| d.to_string()).as_deref(),
        Some("2024-03-05T14:22:10+01:00")
    );
    assert_eq!(h.flags, Flags(Flags::PRINT));
    assert!(h.flags.is_visible() && h.flags.is_editable());
    assert_eq!(h.appearance, Appearance::Original);
    let AnnotKind::TextMarkup { style, quads } = &h.kind else {
        panic!("not text markup: {:?}", h.kind);
    };
    assert_eq!(*style, MarkupStyle::Highlight);
    assert_eq!(quads.len(), 2);
    // Each quad covers its line, whatever order its corners came in.
    assert_eq!(
        quads[0].bounds(),
        PdfBox::new(PdfPoint::new(100.0, 715.0), PdfPoint::new(300.0, 730.0))
    );
    assert!(quads[0].contains(PdfPoint::new(250.0, 720.0)));
    assert!(quads[1].contains(PdfPoint::new(150.0, 707.0)));
    // On the marked text, not just within the rectangle around it.
    assert!(h.contains(PdfPoint::new(250.0, 720.0)));
    assert!(!h.contains(PdfPoint::new(250.0, 707.0)));
}

#[test]
fn reads_the_other_kinds() {
    let annots = import(&sample(), 0);
    let underline = &annots[1];
    assert_eq!(underline.appearance, Appearance::Missing);
    assert_eq!(underline.color, None);
    assert_eq!(underline.opacity, 1.0);

    let ink = &annots[2];
    assert_eq!(ink.line_width, 2.5);
    let AnnotKind::Ink { strokes } = &ink.kind else {
        panic!("not ink: {:?}", ink.kind);
    };
    assert_eq!(strokes.len(), 2);
    assert_eq!(strokes[0].len(), 3);
    assert_eq!(
        strokes[1],
        [PdfPoint::new(50.0, 50.0), PdfPoint::new(60.0, 60.0)]
    );

    let note = &annots[3];
    assert_eq!(
        note.kind,
        AnnotKind::Other {
            subtype: "Text".to_owned()
        }
    );
    assert_eq!(note.contents.as_deref(), Some("Check this"));
    assert!(!note.flags.is_editable());
    assert!(note.contains(PdfPoint::new(410.0, 410.0)));

    let square = &annots[4];
    assert_eq!(
        square.rect,
        PdfBox::new(PdfPoint::new(200.0, 100.0), PdfPoint::new(300.0, 200.0))
    );
    assert_eq!(square.color, Some(Color::new(0.5, 0.5, 0.5)));
    assert_eq!(square.line_width, 3.0);
    assert_eq!(
        square.kind,
        AnnotKind::Square {
            fill: Some(Color::new(1.0, 0.0, 0.0))
        }
    );

    assert_eq!(
        annots[5].kind,
        AnnotKind::Line {
            from: PdfPoint::new(5.0, 5.0),
            to: PdfPoint::new(45.0, 45.0)
        }
    );
}

#[test]
fn broken_annotations_are_skipped_or_kept_as_they_are() {
    let doc = pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 600 800] >>",
        "<< /Type /Page /Parent 2 0 R /Annots [4 0 R 5 0 R 6 0 R 7 0 R 99 0 R] >>",
        // No rectangle: nowhere to show it.
        "<< /Subtype /Highlight /QuadPoints [0 0 1 0 0 1 1 1] >>",
        // A highlight without quads is shown as the file has it.
        "<< /Subtype /Highlight /Rect [0 0 10 10] /QuadPoints [1 2 3] >>",
        // No subtype.
        "<< /Rect [0 0 10 10] >>",
        // Numbers out of range.
        "<< /Subtype /Circle /Rect [0 0 10 10] /CA 7 /C [2 -1 0.5] /BS << /W -3 >> >>",
    ]);
    let annots = import(&doc, 0);
    assert_eq!(annots.len(), 2);
    assert_eq!(
        annots[0].kind,
        AnnotKind::Other {
            subtype: "Highlight".to_owned()
        }
    );
    assert_eq!(annots[1].opacity, 1.0);
    assert_eq!(annots[1].color, Some(Color::new(1.0, 0.0, 0.5)));
    assert_eq!(annots[1].line_width, 1.0);
}
