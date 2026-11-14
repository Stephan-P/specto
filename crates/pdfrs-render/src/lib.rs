//! Page rasterization for pdfrs.
//!
//! Everything engine-specific stays behind [`Document`] so that the rest of the
//! code base never sees hayro types. See `docs/ARCHITECTURE.md` ("Rendering pipeline").

mod hayro_backend;

pub use hayro_backend::HayroDocument;

/// Which parts of a page to draw.
///
/// The viewer renders page content and annotations separately: page tiles are
/// expensive and cached, while annotations come from the edit model and are
/// drawn as an overlay (see ARCHITECTURE.md, "Data model").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Content {
    /// Page content stream plus the annotations stored in the file.
    #[default]
    PageAndAnnotations,
    /// Only the page content stream.
    PageOnly,
}

/// A rectangle in device pixels, relative to the top-left of the full page bitmap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Parameters for rendering one page (or one tile of a page).
#[derive(Clone, Copy, Debug)]
pub struct RenderRequest {
    /// Zero-based page index.
    pub page: usize,
    /// Device pixels per PDF point (zoom × HiDPI scale factor).
    pub scale: f32,
    pub content: Content,
    /// Background as straight (non-premultiplied) RGBA. `None` → transparent.
    pub background: Option<[u8; 4]>,
    /// Only render this region of the scaled page. `None` → whole page.
    pub region: Option<PixelRect>,
}

impl RenderRequest {
    pub fn page(page: usize, scale: f32) -> Self {
        Self {
            page,
            scale,
            content: Content::default(),
            background: Some([255, 255, 255, 255]),
            region: None,
        }
    }
}

/// A rendered bitmap: premultiplied RGBA8, rows top to bottom, no padding.
#[derive(Clone, Debug)]
pub struct Bitmap {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

/// Size of a page in PDF points *as displayed*: CropBox, with /Rotate applied.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageSize {
    pub width: f32,
    pub height: f32,
}

impl PageSize {
    /// Pixel dimensions of the full page bitmap at `scale`.
    /// Must agree with what [`Document::render`] produces.
    pub fn pixels(&self, scale: f32) -> (u32, u32) {
        (
            pixel_extent(self.width, scale),
            pixel_extent(self.height, scale),
        )
    }
}

pub(crate) fn pixel_extent(points: f32, scale: f32) -> u32 {
    (points * scale).round().max(1.0) as u32
}

#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    #[error("the document is password protected")]
    PasswordRequired,
    #[error("the document uses an unsupported encryption scheme")]
    UnsupportedEncryption,
    #[error("the document's encryption dictionary is invalid")]
    InvalidEncryption,
    #[error("not a valid PDF file")]
    Invalid,
}

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("page index {0} out of range")]
    PageOutOfRange(usize),
    #[error("requested bitmap of {0}×{1} px exceeds the size limit")]
    TooLarge(u32, u32),
    #[error("region lies outside the page bitmap")]
    RegionOutOfBounds,
}

/// An opened, read-only PDF that can be rasterized.
pub trait Document {
    fn page_count(&self) -> usize;
    fn page_size(&self, page: usize) -> Option<PageSize>;
    fn render(&self, request: &RenderRequest) -> Result<Bitmap, RenderError>;
}
