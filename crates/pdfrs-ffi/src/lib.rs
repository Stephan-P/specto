//! UniFFI surface for the Android app.
//!
//! M0 spike scope: open a PDF from bytes and rasterize whole pages. The real
//! surface (`Session`, `Viewer`, commands, events) comes in M6; see
//! `docs/ARCHITECTURE.md` ("Android").

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};

use pdfrs_render::{
    Bitmap, Document, HayroDocument, OpenError, PageGeometry, RenderError, RenderRequest,
};

#[cfg(target_os = "android")]
mod android_bitmap;

uniffi::setup_scaffolding!();

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum PdfError {
    #[error("the document is password protected")]
    PasswordRequired,
    #[error("the document uses unsupported or invalid encryption")]
    UnsupportedEncryption,
    #[error("not a valid PDF file")]
    Invalid,
    #[error("page index {page} out of range")]
    PageOutOfRange { page: u32 },
    #[error("requested bitmap of {width}×{height} px exceeds the size limit")]
    TooLarge { width: u32, height: u32 },
    #[error("the renderer panicked")]
    RendererPanicked,
}

impl From<OpenError> for PdfError {
    fn from(e: OpenError) -> Self {
        match e {
            OpenError::PasswordRequired => Self::PasswordRequired,
            OpenError::UnsupportedEncryption | OpenError::InvalidEncryption => {
                Self::UnsupportedEncryption
            }
            OpenError::Invalid => Self::Invalid,
        }
    }
}

/// Page size in PDF points as displayed (CropBox, with /Rotate applied).
#[derive(Clone, Copy, Debug, PartialEq, uniffi::Record)]
pub struct PageSize {
    pub width: f32,
    pub height: f32,
}

/// Size of a rendered page bitmap in device pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct PixelSize {
    pub width: u32,
    pub height: u32,
}

/// Premultiplied RGBA8, rows top to bottom, no padding. This is the in-memory
/// layout of `Bitmap.Config.ARGB_8888`, so it can go straight into
/// `Bitmap.copyPixelsFromBuffer`.
#[derive(Clone, Debug, uniffi::Record)]
pub struct RenderedPage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

thread_local! {
    /// This thread's opened copy of each live [`PdfDocument`], by id.
    static OPENED: RefCell<HashMap<u64, (Weak<()>, HayroDocument)>> = RefCell::default();
}

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

/// `HayroDocument` is not `Send` (its render cache uses `Rc`), but UniFFI
/// objects are called from any thread. So this holds the shared file bytes,
/// and every thread that renders opens (and keeps) its own copy, the same
/// approach as `pdfrs_render::scheduler`.
#[derive(uniffi::Object)]
pub struct PdfDocument {
    id: u64,
    data: Arc<[u8]>,
    password: Option<String>,
    pages: Vec<PageGeometry>,
    /// Per-thread copies hold a `Weak` to this and are dropped once it's gone.
    alive: Arc<()>,
}

impl PdfDocument {
    fn render(&self, page: u32, scale: f32) -> Result<Bitmap, PdfError> {
        let request = RenderRequest::page(page as usize, f64::from(scale));
        OPENED.with_borrow_mut(|opened| {
            // Other threads can't reach into this map, so closed documents are
            // cleaned up lazily here.
            opened.retain(|_, (alive, _)| alive.strong_count() > 0);
            let doc = match opened.entry(self.id) {
                std::collections::hash_map::Entry::Occupied(e) => &e.into_mut().1,
                std::collections::hash_map::Entry::Vacant(e) => {
                    let doc = HayroDocument::open(self.data.clone(), self.password.as_deref())?;
                    &e.insert((Arc::downgrade(&self.alive), doc)).1
                }
            };
            doc.render(&request).map_err(|e| match e {
                RenderError::PageOutOfRange(_) | RenderError::RegionOutOfBounds => {
                    PdfError::PageOutOfRange { page }
                }
                RenderError::TooLarge(width, height) => PdfError::TooLarge { width, height },
                RenderError::Panicked => PdfError::RendererPanicked,
            })
        })
    }
}

#[uniffi::export]
impl PdfDocument {
    #[uniffi::constructor]
    pub fn open(data: Vec<u8>, password: Option<String>) -> Result<Arc<Self>, PdfError> {
        let data: Arc<[u8]> = data.into();
        let doc = HayroDocument::open(data.clone(), password.as_deref())?;
        let pages = (0..doc.page_count())
            .filter_map(|page| doc.page_geometry(page))
            .collect();
        Ok(Arc::new(Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            data,
            password,
            pages,
            alive: Arc::new(()),
        }))
    }

    pub fn page_count(&self) -> u32 {
        self.pages.len() as u32
    }

    pub fn page_size(&self, page: u32) -> Option<PageSize> {
        let size = self.pages.get(page as usize)?.size();
        Some(PageSize {
            width: size.width as f32,
            height: size.height as f32,
        })
    }

    /// Dimensions of the bitmap that rendering `page` at `scale` produces.
    pub fn pixel_size(&self, page: u32, scale: f32) -> Option<PixelSize> {
        let size = self.pages.get(page as usize)?.pixel_size(f64::from(scale));
        Some(PixelSize {
            width: size.width,
            height: size.height,
        })
    }

    /// Address of this object for `NativeBitmap.renderInto` (the JNI bitmap
    /// path). Only valid while the foreign side keeps this object alive.
    pub fn raw_handle(&self) -> u64 {
        std::ptr::from_ref(self) as u64
    }

    /// Render a whole page on a white background.
    /// `scale` is device pixels per PDF point.
    pub fn render_page(&self, page: u32, scale: f32) -> Result<RenderedPage, PdfError> {
        let bitmap = self.render(page, scale)?;
        Ok(RenderedPage {
            width: bitmap.width,
            height: bitmap.height,
            pixels: bitmap.data,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One 200×100 pt page with a red triangle in the lower-right half.
    fn sample_pdf() -> Vec<u8> {
        let content = "1 0 0 rg 0 0 m 200 0 l 200 100 l f";
        format!(
            "%PDF-1.7\n\
             1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n\
             2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 100] >> endobj\n\
             3 0 obj << /Type /Page /Parent 2 0 R /Contents 4 0 R >> endobj\n\
             4 0 obj << /Length {} >> stream\n{}\nendstream endobj\n\
             trailer << /Root 1 0 R /Size 5 >>\n%%EOF\n",
            content.len(),
            content,
        )
        .into_bytes()
    }

    #[test]
    fn open_and_render() {
        let doc = PdfDocument::open(sample_pdf(), None).unwrap();
        assert_eq!(doc.page_count(), 1);
        assert_eq!(
            doc.page_size(0),
            Some(PageSize {
                width: 200.0,
                height: 100.0
            })
        );
        assert_eq!(doc.page_size(1), None);

        assert_eq!(
            doc.pixel_size(0, 2.0),
            Some(PixelSize {
                width: 400,
                height: 200
            })
        );
        let page = doc.render_page(0, 2.0).unwrap();
        assert_eq!((page.width, page.height), (400, 200));
        assert_eq!(page.pixels.len(), 400 * 200 * 4);
        let px = |x: usize, y: usize| &page.pixels[(y * 400 + x) * 4..][..4];
        assert_eq!(px(390, 190), [255, 0, 0, 255]);
        assert_eq!(px(200, 5), [255, 255, 255, 255]);
    }

    #[test]
    fn errors() {
        assert!(matches!(
            PdfDocument::open(b"not a pdf".to_vec(), None),
            Err(PdfError::Invalid)
        ));
        let doc = PdfDocument::open(sample_pdf(), None).unwrap();
        assert!(matches!(
            doc.render_page(7, 1.0),
            Err(PdfError::PageOutOfRange { page: 7 })
        ));
    }

    #[test]
    fn renders_from_any_thread() {
        let doc = PdfDocument::open(sample_pdf(), None).unwrap();
        let here = doc.render_page(0, 1.0).unwrap();
        let there =
            std::thread::scope(|s| s.spawn(|| doc.render_page(0, 1.0).unwrap()).join().unwrap());
        assert_eq!(here.pixels, there.pixels);
    }
}
