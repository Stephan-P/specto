//! Annotations through a save: written, read back with our own import,
//! and rendered with hayro, which shows a file's annotations only through
//! their appearance streams, as many other programs do.

use std::collections::BTreeMap;
use std::process::Command;
use std::sync::Arc;

use lopdf::encryption::crypt_filters::{Aes256CryptFilter, CryptFilter};
use lopdf::{
    Document, EncryptionState, EncryptionVersion, Object, Permissions, Stream, dictionary,
};
use specto_annot::{AnnotKind, AnnotType, Annotation, Appearance, Color, MarkupStyle, import};
use specto_doc::{EditDoc, PageRef, SourceId};
use specto_geom::{PdfBox, PdfPoint, PdfQuad};
use specto_render::{Content, Document as _, HayroDocument, RenderRequest};
use specto_source::{PdfData, SourceDocument};
use specto_write::{AnnotChanges, Source, write_edited};

const MAIN: SourceId = SourceId(0);
const YELLOW: Color = Color::new(1.0, 0.9, 0.0);
const RED: Color = Color::new(0.9, 0.1, 0.1);

/// Two pages of 600 × 800 with a black bar at x 100–300, y 700–730 (text,
/// as far as a highlight is concerned). Page 1 has, in `/Annots`: a link,
/// a square with an appearance (a green box at x 400–500, y 100–200), its
/// popup, and a reply to it.
fn fixture() -> Vec<u8> {
    let mut doc = Document::with_version("1.7");
    let pages = doc.new_object_id();
    let mut kids = Vec::new();
    for _ in 0..2 {
        let content = b"0 g 100 700 200 30 re f".to_vec();
        let contents = doc.add_object(Stream::new(dictionary! {}, content));
        kids.push(doc.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages, "Contents" => contents,
        }));
    }
    let link = doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Link",
        "Rect" => vec![0.into(), 0.into(), 50.into(), 50.into()],
        "Dest" => vec![kids[1].into(), "Fit".into()],
    });
    let look = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form",
            "BBox" => vec![400.into(), 100.into(), 500.into(), 200.into()],
        },
        b"0 1 0 rg 400 100 100 100 re f".to_vec(),
    ));
    let square = doc.new_object_id();
    let popup = doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Popup", "Parent" => square,
        "Rect" => vec![500.into(), 100.into(), 600.into(), 200.into()],
    });
    doc.objects.insert(
        square,
        dictionary! {
            "Type" => "Annot", "Subtype" => "Square", "Popup" => popup,
            "Rect" => vec![400.into(), 100.into(), 500.into(), 200.into()],
            "C" => vec![0.into(), 1.into(), 0.into()],
            "Contents" => Object::string_literal("A box"),
            "Vendor" => Object::string_literal("kept"),
            "AP" => dictionary! { "N" => look },
        }
        .into(),
    );
    let reply = doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Text", "IRT" => square,
        "Rect" => vec![400.into(), 100.into(), 420.into(), 120.into()],
        "Contents" => Object::string_literal("Agreed"),
    });
    doc.get_dictionary_mut(kids[0]).unwrap().set(
        "Annots",
        vec![link.into(), square.into(), popup.into(), reply.into()],
    );
    doc.objects.insert(
        pages,
        dictionary! {
            "Type" => "Pages", "Count" => 2,
            "Kids" => kids.iter().map(|&k| Object::Reference(k)).collect::<Vec<_>>(),
            "MediaBox" => vec![0.into(), 0.into(), 600.into(), 800.into()],
            "Resources" => dictionary! {},
        }
        .into(),
    );
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages });
    doc.trailer.set("Root", catalog);
    doc.trailer.set(
        "ID",
        vec![
            Object::string_literal(b"0123456789abcdef".to_vec()),
            Object::string_literal(b"0123456789abcdef".to_vec()),
        ],
    );
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    out
}

fn encrypted(data: &[u8], password: &str) -> Vec<u8> {
    let mut doc = Document::load_mem(data).unwrap();
    let filter: Arc<dyn CryptFilter> = Arc::new(Aes256CryptFilter);
    let key = [7u8; 32];
    let state = EncryptionState::try_from(EncryptionVersion::V5 {
        encrypt_metadata: true,
        crypt_filters: BTreeMap::from([(b"StdCF".to_vec(), filter)]),
        file_encryption_key: &key,
        stream_filter: b"StdCF".to_vec(),
        string_filter: b"StdCF".to_vec(),
        owner_password: "owner",
        user_password: password,
        permissions: Permissions::all(),
    })
    .unwrap();
    doc.encrypt(&state).unwrap();
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    out
}

/// A quad over x `x0`–`x1` of the bar, bottom left first.
fn over_bar(x0: f64, x1: f64) -> PdfQuad {
    PdfQuad::new([
        PdfPoint::new(x0, 700.0),
        PdfPoint::new(x1, 700.0),
        PdfPoint::new(x1, 730.0),
        PdfPoint::new(x0, 730.0),
    ])
}

fn markup(style: MarkupStyle, x0: f64, x1: f64, color: Color) -> Annotation {
    Annotation::text_markup(style, vec![over_bar(x0, x1)], color).unwrap()
}

/// What saving does with `edit`'s annotations.
fn changes(edit: &EditDoc) -> AnnotChanges {
    edit.pages()
        .iter()
        .map(|p| {
            (
                (p.source, p.page),
                edit.annotation_changes(p.source, p.page),
            )
        })
        .collect()
}

fn write(data: &[u8], password: Option<&str>, edit: &EditDoc) -> Vec<u8> {
    let data = PdfData::from(data.to_vec());
    let source = Source {
        id: MAIN,
        data: &data,
        password,
    };
    let out = write_edited(&[source], edit.pages(), &changes(edit), true).expect("it saves");
    qpdf_check(&out, password);
    out
}

/// The colour at PDF point (`x`, `y`) of `page`, rendered at one pixel a
/// point, with or without the annotations.
fn pixel(
    data: &[u8],
    password: Option<&str>,
    page: usize,
    x: u32,
    y: u32,
    annots: bool,
) -> [u8; 3] {
    let doc = HayroDocument::open(data.to_vec(), password).expect("the file opens");
    let mut req = RenderRequest::page(page, 1.0);
    if !annots {
        req.content = Content::PageOnly;
    }
    let bitmap = doc.render(&req).unwrap();
    let at = ((799 - y) * bitmap.width + x) as usize * 4;
    [bitmap.data[at], bitmap.data[at + 1], bitmap.data[at + 2]]
}

fn imported(data: &[u8], password: Option<&str>, page: usize) -> Vec<Annotation> {
    let doc = SourceDocument::open(data.to_vec(), password).expect("the file opens");
    import(&doc, page)
}

fn imported_arcs(data: &[u8], page: usize) -> Vec<Arc<Annotation>> {
    imported(data, None, page)
        .into_iter()
        .map(Arc::new)
        .collect()
}

/// `qpdf --check` passes, if qpdf is installed.
fn qpdf_check(data: &[u8], password: Option<&str>) {
    let path = std::env::temp_dir().join(format!(
        "specto-annots-{}-{:x}.pdf",
        std::process::id(),
        data.len() ^ data.iter().take(4096).map(|&b| b as usize).sum::<usize>()
    ));
    std::fs::write(&path, data).unwrap();
    let mut cmd = Command::new("qpdf");
    if let Some(p) = password {
        cmd.arg(format!("--password={p}"));
    }
    let result = cmd.arg("--check").arg(&path).output();
    std::fs::remove_file(&path).unwrap();
    let Ok(out) = result else {
        eprintln!("qpdf not installed; skipping the check");
        return;
    };
    assert!(
        out.status.success() && out.stderr.is_empty(),
        "qpdf --check: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn new_markup_is_saved_and_shows() {
    let original = fixture();
    let mut edit = EditDoc::from_source(MAIN, 2);
    let mut highlight = markup(MarkupStyle::Highlight, 90.0, 200.0, YELLOW);
    highlight.contents = Some("Größe".to_owned());
    highlight.modified = specto_annot::Date::parse(b"D:20261115190000+01'00'");
    edit.add_annotation(MAIN, 1, highlight);
    edit.add_annotation(MAIN, 1, markup(MarkupStyle::Underline, 350.0, 450.0, RED));
    edit.add_annotation(MAIN, 1, markup(MarkupStyle::StrikeOut, 350.0, 450.0, RED));
    let out = write(&original, None, &edit);

    // Read back: what was put in, now as annotations of the file.
    let annots = imported(&out, None, 1);
    let types: Vec<AnnotType> = annots.iter().map(Annotation::annot_type).collect();
    assert_eq!(
        types,
        [
            AnnotType::Highlight,
            AnnotType::Underline,
            AnnotType::StrikeOut
        ]
    );
    let h = &annots[0];
    assert_eq!(h.color, Some(YELLOW));
    assert_eq!(h.contents.as_deref(), Some("Größe"));
    assert_eq!(
        h.modified.map(|d| d.to_string()).as_deref(),
        Some("2026-11-15T19:00:00+01:00")
    );
    assert_eq!(h.appearance, Appearance::Original);
    assert!(h.flags.is_visible());
    assert_eq!(
        h.rect,
        PdfBox::new(PdfPoint::new(90.0, 700.0), PdfPoint::new(200.0, 730.0))
    );
    let AnnotKind::TextMarkup { quads, .. } = &h.kind else {
        panic!("not text markup");
    };
    assert_eq!(quads, &[over_bar(90.0, 200.0)]);
    // Page 1 keeps what it had.
    assert_eq!(imported(&out, None, 0).len(), 2);

    // Rendered: yellow beside the bar, and the bar still black under the
    // highlight (it darkens, it doesn't cover). Nothing without annotations.
    let at = |x, y, annots| pixel(&out, None, 1, x, y, annots);
    let [r, g, b] = at(95, 715, true);
    assert!(r > 240 && g > 215 && b < 30, "beside the bar: {r} {g} {b}");
    assert_eq!(at(150, 715, true), [0, 0, 0]);
    assert_eq!(at(95, 715, false), [255, 255, 255]);
    assert_eq!(at(250, 715, true), [0, 0, 0]);
    // The underline near the bottom of its quad, the strike-out further up,
    // white between and above.
    let red = |[r, g, b]: [u8; 3]| r > 200 && g < 60 && b < 60;
    assert!(
        red(at(400, 702, true)),
        "underline: {:?}",
        at(400, 702, true)
    );
    assert!(
        red(at(400, 712, true)),
        "strike-out: {:?}",
        at(400, 712, true)
    );
    assert_eq!(at(400, 707, true), [255, 255, 255]);
    assert_eq!(at(400, 722, true), [255, 255, 255]);
}

#[test]
fn deleting_one_of_the_files() {
    let original = fixture();
    let on_page = imported_arcs(&original, 0);
    assert_eq!(on_page.len(), 2, "the square and the reply to it");
    assert_eq!(pixel(&original, None, 0, 450, 150, true), [0, 255, 0]);

    let mut edit = EditDoc::from_source(MAIN, 2);
    let (id, square) = edit.annotations(MAIN, 0, &on_page)[0].clone();
    assert!(edit.remove_annotation(id, &square));
    let out = write(&original, None, &edit);

    // Gone, with its popup and the reply; the link stays.
    assert!(imported(&out, None, 0).is_empty());
    assert_eq!(pixel(&out, None, 0, 450, 150, true), [255, 255, 255]);
    let doc = SourceDocument::open(out.clone(), None).unwrap();
    assert_eq!(doc.links(0).len(), 1);
    let written = Document::load_mem(&out).unwrap();
    let page = written.get_pages()[&1];
    let annots = written
        .get_dictionary(page)
        .unwrap()
        .get(b"Annots")
        .unwrap();
    assert_eq!(annots.as_array().unwrap().len(), 1);
}

#[test]
fn changing_one_of_the_files() {
    let original = fixture();
    let on_page = imported_arcs(&original, 0);
    let mut edit = EditDoc::from_source(MAIN, 2);
    let (id, square) = edit.annotations(MAIN, 0, &on_page)[0].clone();
    let mut changed = (*square).clone();
    changed.contents = Some("Another box".to_owned());
    changed.color = Some(RED);
    assert!(edit.change_annotation(id, changed));
    let out = write(&original, None, &edit);

    let annots = imported(&out, None, 0);
    assert_eq!(annots.len(), 2);
    assert_eq!(annots[0].contents.as_deref(), Some("Another box"));
    assert_eq!(annots[0].color, Some(RED));
    // Only its values changed, so it looks as it did, and is the same
    // object: its popup and the reply still belong to it, and what this
    // program doesn't know about is kept.
    assert_eq!(annots[0].appearance, Appearance::Original);
    assert_eq!(pixel(&out, None, 0, 450, 150, true), [0, 255, 0]);
    let written = Document::load_mem(&out).unwrap();
    let page = written.get_pages()[&1];
    let entries = written
        .get_dictionary(page)
        .unwrap()
        .get(b"Annots")
        .unwrap();
    let entries = entries.as_array().unwrap();
    assert_eq!(entries.len(), 4);
    let square_id = entries[1].as_reference().unwrap();
    let dict = written.get_dictionary(square_id).unwrap();
    assert_eq!(
        dict.get(b"Popup").unwrap().as_reference().unwrap(),
        entries[2].as_reference().unwrap()
    );
    assert!(dict.has(b"Vendor"));
    let reply = written
        .get_dictionary(entries[3].as_reference().unwrap())
        .unwrap();
    assert_eq!(
        reply.get(b"IRT").unwrap().as_reference().unwrap(),
        square_id
    );
}

#[test]
fn encrypted_and_on_a_page_used_twice() {
    let original = encrypted(&fixture(), "secret");
    let mut edit = EditDoc::from_source(MAIN, 2);
    let mut highlight = markup(MarkupStyle::Highlight, 90.0, 200.0, YELLOW);
    highlight.contents = Some("A note".to_owned());
    edit.add_annotation(MAIN, 1, highlight);
    // Page 2 a second time, at the end.
    edit.insert(2, vec![PageRef::new(MAIN, 1)]);
    let out = write(&original, Some("secret"), &edit);

    assert!(
        SourceDocument::open(out.clone(), None).is_err(),
        "still protected"
    );
    for page in [1, 2] {
        let annots = imported(&out, Some("secret"), page);
        assert_eq!(annots.len(), 1, "page {page}");
        assert_eq!(annots[0].contents.as_deref(), Some("A note"));
        let [r, g, b] = pixel(&out, Some("secret"), page, 95, 715, true);
        assert!(r > 240 && g > 215 && b < 30, "page {page}: {r} {g} {b}");
    }
}
