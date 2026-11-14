use std::sync::Arc;

use hayro::hayro_interpret::util::TransformExt;
use hayro::hayro_interpret::{InterpreterSettings, RenderContent};
use hayro::hayro_syntax::{DecryptionError, LoadPdfError, Pdf};
use hayro::kurbo::Affine;
use hayro::vello_cpu::color::AlphaColor;
use hayro::vello_cpu::color::palette::css::TRANSPARENT;
use hayro::vello_cpu::{Pixmap, RasterizerSettings, RenderContext, Resources, TargetInit};
use hayro::{RenderCache, RenderSettings, render_into};

use crate::{
    Bitmap, Content, Document, OpenError, PageSize, PixelRect, RenderError, RenderRequest,
};

/// vello_cpu addresses pixels with `u16`.
const MAX_DIMENSION: u32 = u16::MAX as u32;

/// A PDF opened with the pure-Rust hayro renderer.
pub struct HayroDocument {
    pdf: Pdf,
    settings: InterpreterSettings,
}

impl HayroDocument {
    pub fn open(data: Vec<u8>, password: Option<&str>) -> Result<Self, OpenError> {
        let pdf =
            Pdf::new_with_password(Arc::new(data), password.unwrap_or("")).map_err(
                |e| match e {
                    LoadPdfError::Decryption(DecryptionError::PasswordProtected) => {
                        OpenError::PasswordRequired
                    }
                    LoadPdfError::Decryption(DecryptionError::UnsupportedAlgorithm) => {
                        OpenError::UnsupportedEncryption
                    }
                    LoadPdfError::Decryption(_) => OpenError::InvalidEncryption,
                    LoadPdfError::Invalid => OpenError::Invalid,
                },
            )?;
        Ok(Self {
            pdf,
            // Defaults use hayro's embedded standard fonts and CMaps.
            // TODO: resolve fallback fonts via fontconfig for better CJK coverage.
            settings: InterpreterSettings::default(),
        })
    }

    pub fn pdf(&self) -> &Pdf {
        &self.pdf
    }
}

impl Document for HayroDocument {
    fn page_count(&self) -> usize {
        self.pdf.pages().len()
    }

    fn page_size(&self, page: usize) -> Option<PageSize> {
        let (width, height) = self.pdf.pages().get(page)?.render_dimensions();
        Some(PageSize { width, height })
    }

    fn render(&self, req: &RenderRequest) -> Result<Bitmap, RenderError> {
        let page = self
            .pdf
            .pages()
            .get(req.page)
            .ok_or(RenderError::PageOutOfRange(req.page))?;
        let (pw, ph) = page.render_dimensions();
        let (full_w, full_h) = PageSize {
            width: pw,
            height: ph,
        }
        .pixels(req.scale);

        let region = req.region.unwrap_or(PixelRect {
            x: 0,
            y: 0,
            width: full_w,
            height: full_h,
        });
        if region.width == 0
            || region.height == 0
            || region.x + region.width > full_w
            || region.y + region.height > full_h
        {
            return Err(RenderError::RegionOutOfBounds);
        }
        if region.width > MAX_DIMENSION || region.height > MAX_DIMENSION {
            return Err(RenderError::TooLarge(region.width, region.height));
        }

        // Scale non-uniformly by a hair so the page exactly fills the rounded
        // pixel dimensions; tiles then line up seamlessly with full renders.
        let sx = full_w as f64 / pw as f64;
        let sy = full_h as f64 / ph as f64;
        let transform = Affine::translate((-(region.x as f64), -(region.y as f64)))
            * Affine::scale_non_uniform(sx, sy)
            * page.initial_transform(true).to_kurbo();

        let mut settings = self.settings.clone();
        settings.render_content = match req.content {
            Content::PageAndAnnotations => RenderContent::all(),
            Content::PageOnly => RenderContent::PAGE,
        };

        let (w, h) = (region.width as u16, region.height as u16);
        let mut ctx = RenderContext::new(w, h);
        // TODO: keep a RenderCache per document across calls. It borrows the
        // `Pdf`, so that needs a self-referential owner or a per-thread cache.
        let cache = RenderCache::new();
        render_into(
            page,
            &cache,
            &settings,
            &RenderSettings::default(),
            &mut ctx,
            transform,
        );
        ctx.flush();

        let background = req
            .background
            .map(|[r, g, b, a]| AlphaColor::from_rgba8(r, g, b, a))
            .unwrap_or(TRANSPARENT);
        let mut pixmap = Pixmap::new(w, h);
        ctx.render_with(
            &mut pixmap,
            &mut Resources::default(),
            RasterizerSettings {
                target_init: TargetInit::Clear(background),
                ..Default::default()
            },
        );

        Ok(Bitmap {
            width: region.width,
            height: region.height,
            data: pixmap.data_as_u8_slice().to_vec(),
        })
    }
}
