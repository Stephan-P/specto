use pdfrs_render::{Content, Document, HayroDocument, OpenError, PixelRect, RenderRequest};

/// A one-page 200×100 pt PDF: a red diagonal triangle plus a blue square annotation.
/// Offsets in the xref are deliberately wrong; hayro recovers by scanning.
fn sample_pdf() -> Vec<u8> {
    let content = b"1 0 0 rg 0 0 m 200 0 l 200 100 l f";
    let ap = b"0 0 1 rg 0 0 20 20 re f";
    format!(
        "%PDF-1.7\n\
         1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n\
         2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 100] >> endobj\n\
         3 0 obj << /Type /Page /Parent 2 0 R /Contents 4 0 R /Annots [5 0 R] >> endobj\n\
         4 0 obj << /Length {} >> stream\n{}\nendstream endobj\n\
         5 0 obj << /Type /Annot /Subtype /Square /Rect [10 70 30 90] /AP << /N 6 0 R >> >> endobj\n\
         6 0 obj << /Type /XObject /Subtype /Form /BBox [0 0 20 20] /Length {} >> stream\n{}\nendstream endobj\n\
         trailer << /Root 1 0 R /Size 7 >>\n%%EOF\n",
        content.len(),
        std::str::from_utf8(content).unwrap(),
        ap.len(),
        std::str::from_utf8(ap).unwrap(),
    )
    .into_bytes()
}

fn pixel(b: &pdfrs_render::Bitmap, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * b.width + x) * 4) as usize;
    b.data[i..i + 4].try_into().unwrap()
}

#[test]
fn page_size_and_full_render() {
    let doc = HayroDocument::open(sample_pdf(), None).unwrap();
    assert_eq!(doc.page_count(), 1);
    let size = doc.page_size(0).unwrap();
    assert_eq!((size.width, size.height), (200.0, 100.0));

    let bmp = doc.render(&RenderRequest::page(0, 2.0)).unwrap();
    assert_eq!((bmp.width, bmp.height), size.pixels(2.0));
    // The triangle covers the lower-right half: bottom-right is red, top-centre is white.
    assert_eq!(pixel(&bmp, 390, 190), [255, 0, 0, 255]);
    assert_eq!(pixel(&bmp, 200, 10), [255, 255, 255, 255]);
}

#[test]
fn annotations_can_be_excluded() {
    let doc = HayroDocument::open(sample_pdf(), None).unwrap();
    // Annotation covers x 10..30, y 70..90 pt → pixels (40, 40) at scale 2 (y flipped).
    let mut req = RenderRequest::page(0, 2.0);
    assert_eq!(pixel(&doc.render(&req).unwrap(), 40, 40), [0, 0, 255, 255]);
    req.content = Content::PageOnly;
    assert_eq!(
        pixel(&doc.render(&req).unwrap(), 40, 40),
        [255, 255, 255, 255]
    );
}

#[test]
fn tiles_match_full_render() {
    let doc = HayroDocument::open(sample_pdf(), None).unwrap();
    let scale = 1.37; // deliberately odd
    let full = doc.render(&RenderRequest::page(0, scale)).unwrap();
    let tile_size = 64;
    for ty in (0..full.height).step_by(tile_size) {
        for tx in (0..full.width).step_by(tile_size) {
            let region = PixelRect {
                x: tx,
                y: ty,
                width: (tile_size as u32).min(full.width - tx),
                height: (tile_size as u32).min(full.height - ty),
            };
            let mut req = RenderRequest::page(0, scale);
            req.region = Some(region);
            let tile = doc.render(&req).unwrap();
            for y in 0..tile.height {
                for x in 0..tile.width {
                    let (a, b) = (pixel(&tile, x, y), pixel(&full, tx + x, ty + y));
                    let max = (0..4).map(|c| a[c].abs_diff(b[c])).max().unwrap();
                    assert!(max <= 2, "tile ({tx},{ty}) pixel ({x},{y}): {a:?} vs {b:?}");
                }
            }
        }
    }
}

#[test]
fn rejects_garbage_and_bad_regions() {
    assert!(matches!(
        HayroDocument::open(b"not a pdf".to_vec(), None),
        Err(OpenError::Invalid)
    ));
    let doc = HayroDocument::open(sample_pdf(), None).unwrap();
    let mut req = RenderRequest::page(0, 1.0);
    req.region = Some(PixelRect {
        x: 150,
        y: 0,
        width: 100,
        height: 10,
    });
    assert!(doc.render(&req).is_err());
    assert!(doc.render(&RenderRequest::page(1, 1.0)).is_err());
}
