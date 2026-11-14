use pdfrs_geom::PdfPoint;
use pdfrs_source::SourceDocument;
use pdfrs_text::{Extractor, TextPage};

/// A one-page PDF (600 × 800 pt) with Helvetica as /F1 and `content`.
fn pdf(content: &str) -> Vec<u8> {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 600 800] >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /Contents 4 0 R
            /Resources << /Font << /F1 5 0 R >> >> >>"
            .to_string(),
        format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_string(),
    ];
    let mut out = String::from("%PDF-1.7\n");
    for (i, body) in objects.iter().enumerate() {
        out.push_str(&format!("{} 0 obj\n{body}\nendobj\n", i + 1));
    }
    out.push_str("trailer << /Root 1 0 R /Size 6 >>\n%%EOF\n");
    out.into_bytes()
}

fn text_page(content: &str) -> TextPage {
    let doc = SourceDocument::open(pdf(content), None).unwrap();
    Extractor::new(doc.pdf()).page(0).unwrap()
}

#[test]
fn lines_and_words() {
    let page =
        text_page("BT /F1 12 Tf 72 700 Td (Hello World) Tj 0 -14 Td (Second line here) Tj ET");
    assert_eq!(page.text(), "Hello World\nSecond line here");
    assert_eq!(page.lines.len(), 2);
    assert_eq!(page.words.len(), 5);
    // Characters sit where the text was drawn: "H" starts at x = 72 on the
    // baseline at y = 700.
    let h = &page.chars[0];
    assert_eq!(h.text, "H");
    assert!((h.origin().x - 72.0).abs() < 0.01);
    assert!((h.size - 12.0).abs() < 0.01);
    assert!(h.quad.contains(PdfPoint::new(74.0, 703.0)));
}

#[test]
fn kerning_joins_and_gaps_split() {
    // -60/1000 em of kerning keeps "Hel" and "lo" together; a 400/1000 em
    // gap without a space character still separates words.
    let page = text_page("BT /F1 12 Tf 72 700 Td [(Hel) -60 (lo) -400 (World)] TJ ET");
    assert_eq!(page.text(), "Hello World");
}

#[test]
fn superscripts_stay_on_the_line() {
    let page = text_page(
        "BT /F1 12 Tf 72 700 Td (E = mc) Tj /F1 8 Tf 5 Ts (2) Tj 0 Ts /F1 12 Tf ( holds) Tj ET",
    );
    assert_eq!(page.lines.len(), 1, "{:?}", page.text());
    assert_eq!(page.text(), "E = mc2 holds");
}

#[test]
fn rotated_text() {
    // Rotated 90° counter-clockwise, reading upwards.
    let page = text_page("BT /F1 12 Tf 0 1 -1 0 300 100 Tm (Going up) Tj ET");
    assert_eq!(page.text(), "Going up");
    let g = &page.chars[0];
    let d = g.direction();
    assert!(d.x.abs() < 1e-6 && (d.y - 1.0).abs() < 1e-6, "{d:?}");
}

#[test]
fn lookups_and_ranges() {
    let page = text_page("BT /F1 12 Tf 72 700 Td (one two) Tj 0 -14 Td (three) Tj ET");
    // "two" starts at char 4 ("one" + space).
    let t = page.chars[4].quad.bounds().center();
    let c = page.char_at(t).unwrap();
    assert_eq!(c, 4);
    assert_eq!(page.word_of(c).unwrap().chars, 4..7);
    let line = page.line_of(c).unwrap();
    assert_eq!(page.text_range(line.chars.clone()), "one two");
    // A range spanning the line break and part of a word.
    assert_eq!(page.text_range(5..10), "wo\nthr");
    assert!(page.char_at(PdfPoint::new(10.0, 10.0)).is_none());
    assert!(page.word_of(3).is_none(), "the space belongs to no word");
}

#[test]
fn positions_for_selection() {
    // "one two" at y = 700, "three" at y = 686; Helvetica 12 pt.
    let page = text_page("BT /F1 12 Tf 72 700 Td (one two) Tj 0 -14 Td (three) Tj ET");
    let mid = |c: usize| page.chars[c].quad.bounds().center();
    let left_half = |c: usize| {
        let b = page.chars[c].quad.bounds();
        PdfPoint::new(b.min.x + b.width() * 0.25, b.center().y)
    };
    let right_half = |c: usize| {
        let b = page.chars[c].quad.bounds();
        PdfPoint::new(b.max.x - b.width() * 0.25, b.center().y)
    };
    // On a character: before or after it, by half.
    assert_eq!(page.position_at(left_half(4)), 4);
    assert_eq!(page.position_at(right_half(4)), 5);
    // Left and right margins: the line's start and end.
    assert_eq!(page.position_at(PdfPoint::new(20.0, 703.0)), 0);
    assert_eq!(page.position_at(PdfPoint::new(500.0, 703.0)), 7);
    assert_eq!(page.position_at(PdfPoint::new(500.0, 689.0)), 12);
    // Slightly above the first line still counts as that line.
    let above = PdfPoint::new(mid(1).x, 720.0);
    assert!((1..=2).contains(&page.position_at(above)));
    // Nowhere near any text, an empty page has position 0.
    assert_eq!(text_page("").position_at(PdfPoint::new(1.0, 1.0)), 0);

    assert_eq!(page.word_range(5), 4..7);
    assert_eq!(page.word_range(3), 3..4, "a space is its own unit");
    assert_eq!(page.line_range(9), 7..12);
}

#[test]
fn positions_in_two_columns() {
    // Left column at x = 72, right column at x = 320 with its baseline 3 pt
    // higher. A pointer in the gutter, right of the left column's first
    // line and just below the right column's line, belongs to the left one.
    let page = text_page(
        "BT /F1 12 Tf 72 700 Td (left one) Tj 0 -14 Td (left two) Tj ET \
         BT /F1 12 Tf 320 703 Td (right one) Tj ET",
    );
    let at = page.position_at(PdfPoint::new(150.0, 699.0));
    let line = page.line_of(at.saturating_sub(1)).unwrap();
    assert_eq!(
        page.text_range(line.chars.clone()),
        "left one",
        "position {at}"
    );
    // Level with the right column and close to it: the right column.
    let at = page.position_at(PdfPoint::new(310.0, 706.0));
    assert_eq!(page.text_range(page.line_range(at)), "right one");
}

#[test]
fn selection_quads_per_line() {
    let page = text_page("BT /F1 12 Tf 72 700 Td (one two) Tj 0 -14 Td (three) Tj ET");
    // "two" and "th": one quad on each line.
    let quads = page.selection_quads(4..9);
    assert_eq!(quads.len(), 2);
    let (a, b) = (quads[0].bounds(), quads[1].bounds());
    assert!((a.min.x - page.chars[4].origin().x).abs() < 0.01);
    assert!((a.max.x - page.chars[6].end().x).abs() < 0.01);
    assert!(
        a.min.y > b.max.y - 1.0,
        "the first line is above the second"
    );
    assert!((b.min.x - 72.0).abs() < 0.01);
    // Both lines are as tall as their text.
    assert!((a.height() - 12.0).abs() < 0.01 && (b.height() - 12.0).abs() < 0.01);
    assert!(page.selection_quads(3..3).is_empty());
}

#[test]
fn selection_follows_rotated_text() {
    // Reading upwards: the quad is tall and narrow, and positions run along y.
    let page = text_page("BT /F1 12 Tf 0 1 -1 0 300 100 Tm (Going up) Tj ET");
    let q = page.selection_quads(0..8)[0].bounds();
    assert!(q.height() > 3.0 * q.width(), "{q:?}");
    let x = q.center().x;
    assert_eq!(page.position_at(PdfPoint::new(x, 90.0)), 0);
    assert_eq!(page.position_at(PdfPoint::new(x, q.max.y + 10.0)), 8);
}

#[test]
fn empty_page() {
    let page = text_page("0 0 1 rg 0 0 100 100 re f");
    assert!(page.chars.is_empty() && page.text().is_empty());
}

/// A 10 pt character `c` occupying x..x+6 on the baseline y = 100.
fn ch(c: &str, x: f64) -> pdfrs_text::TextChar {
    use pdfrs_geom::PdfQuad;
    let p = |x: f64, y: f64| PdfPoint::new(x, y);
    pdfrs_text::TextChar {
        text: c.into(),
        quad: PdfQuad::new([p(x, 98.0), p(x + 6.0, 98.0), p(x + 6.0, 108.0), p(x, 108.0)]),
        size: 10.0,
    }
}

/// Lays out `glyphs` left to right, 6 pt apart, with a space glyph between words.
fn left_to_right(glyphs: &[&str]) -> Vec<pdfrs_text::TextChar> {
    glyphs
        .iter()
        .enumerate()
        .map(|(i, g)| ch(g, 100.0 + 6.0 * i as f64))
        .collect()
}

#[test]
fn rtl_in_visual_order_comes_out_logical() {
    // "שלום עולם" ("hello world") drawn left to right, as most generators
    // do: on the page it reads (from the left) "םלוע םולש".
    let visual = ["ם", "ל", "ו", "ע", " ", "ם", "ו", "ל", "ש"];
    let page = TextPage::from_chars(left_to_right(&visual));
    assert_eq!(page.text(), "שלום עולם");
}

#[test]
fn rtl_in_logical_order_stays_logical() {
    // The same text drawn right to left (logical order), glyph by glyph.
    let logical = ["ש", "ל", "ו", "ם", " ", "ע", "ו", "ל", "ם"];
    let chars = logical
        .iter()
        .enumerate()
        .map(|(i, g)| ch(g, 200.0 - 6.0 * i as f64))
        .collect();
    let page = TextPage::from_chars(chars);
    assert_eq!(page.lines.len(), 1, "logical RTL must not break per glyph");
    assert_eq!(page.text(), "שלום עולם");
}

#[test]
fn rtl_line_with_a_number() {
    // "פרק 15" ("chapter 15") in visual order: "15 קרפ" from the left.
    let visual = ["1", "5", " ", "ק", "ר", "פ"];
    let page = TextPage::from_chars(left_to_right(&visual));
    assert_eq!(page.text(), "פרק 15");
}

/// Extracts the first pages of every corpus file: no panics.
/// `cargo test -p pdfrs-text --release -- --ignored --nocapture`
#[test]
#[ignore = "needs corpus/files (run corpus/fetch.sh)"]
fn corpus_extraction_does_not_panic() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/files");
    let mut stack = vec![dir];
    let (mut files, mut chars, mut panics) = (0, 0, Vec::new());
    let start = std::time::Instant::now();
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            if p.extension().is_none_or(|e| e != "pdf") {
                continue;
            }
            let Ok(doc) = SourceDocument::open(std::fs::read(&p).unwrap(), None) else {
                continue;
            };
            files += 1;
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let ex = Extractor::new(doc.pdf());
                (0..doc.page_count().min(3))
                    .filter_map(|i| ex.page(i))
                    .map(|t| {
                        let _ = t.text();
                        t.chars.len()
                    })
                    .sum::<usize>()
            }));
            match r {
                Ok(n) => chars += n,
                Err(_) => panics.push(p.display().to_string()),
            }
        }
    }
    println!(
        "{files} files, {chars} characters in {:.1?}",
        start.elapsed()
    );
    assert!(panics.is_empty(), "panicked on: {panics:#?}");
}
