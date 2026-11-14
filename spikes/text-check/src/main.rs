//! M2 spike: is hayro's glyph → Unicode mapping good enough for a text layer?
//!
//!   text-check <file.pdf> [max-pages]
//!
//! Collects glyphs with a custom `Device`, builds words with simple gap and
//! baseline heuristics, and compares the words with `pdftotext -raw` (Poppler).
//! Prints one line: file, glyphs, glyphs without Unicode, our words, Poppler's
//! words, and recall (share of Poppler's words we also found). Throwaway code.

use std::collections::HashMap;
use std::process::Command;
use std::sync::Arc;

use anyhow::Result;
use hayro::hayro_interpret::font::GlyphRun;
use hayro::hayro_interpret::util::TransformExt;
use hayro::hayro_interpret::{
    BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image, ImageDrawProps,
    InterpreterCache, InterpreterSettings, SoftMask, interpret_page,
};
use hayro::hayro_syntax::Pdf;
use hayro::kurbo::{BezPath, Point, Rect, Shape};
use unicode_normalization::UnicodeNormalization;

/// A glyph in page space (points, y down).
struct TextGlyph {
    text: String,
    origin: Point,
    end: Point,
    /// Font size in page units (length of the em vector).
    size: f64,
}

#[derive(Default)]
struct TextDevice {
    glyphs: Vec<TextGlyph>,
    missing_unicode: usize,
}

impl<'a> Device<'a> for TextDevice {
    fn draw_path(&mut self, _: &BezPath, _: DrawProps<'a>, _: &DrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_image(&mut self, _: Image<'a, '_>, _: ImageDrawProps<'a>) {}
    fn pop_clip(&mut self) {}
    fn pop_transparency_group(&mut self) {}

    fn draw_glyph_run(&mut self, run: &GlyphRun<'_, 'a>, props: DrawProps<'a>, _: &DrawMode) {
        for g in run.glyphs() {
            let t = props.transform * g.transform();
            let text = match g.as_unicode() {
                Some(hayro::hayro_interpret::hayro_cmap::BfString::Char(c)) => c.to_string(),
                Some(hayro::hayro_interpret::hayro_cmap::BfString::String(s)) => s,
                None => {
                    self.missing_unicode += 1;
                    continue;
                }
            };
            // Glyph space is 1000 units per em.
            let advance = match &**g {
                hayro::hayro_interpret::font::Glyph::Outline(o) => o
                    .advance_width()
                    .map(f64::from)
                    .unwrap_or_else(|| o.outline().bounding_box().x1),
                hayro::hayro_interpret::font::Glyph::Type3(_) => 500.0,
            };
            let origin = t * Point::ZERO;
            let end = t * Point::new(advance, 0.0);
            let size = (t * Point::new(0.0, 1000.0) - origin).hypot();
            self.glyphs.push(TextGlyph {
                text,
                origin,
                end,
                size,
            });
        }
    }
}

/// Words in content order: a new word on a gap wider than 0.2 em, a jump
/// backwards, or a baseline change of more than half an em.
fn words(glyphs: &[TextGlyph]) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut prev: Option<&TextGlyph> = None;
    for g in glyphs {
        if let Some(p) = prev {
            let em = p.size.max(g.size).max(0.1);
            let gap = g.origin.x - p.end.x;
            let dy = (g.origin.y - p.origin.y).abs();
            if gap > 0.2 * em || gap < -0.5 * em || dy > 0.5 * em {
                words.push(std::mem::take(&mut current));
            }
        }
        for c in g.text.chars() {
            if c.is_whitespace() {
                words.push(std::mem::take(&mut current));
            } else {
                current.push(c);
            }
        }
        prev = Some(g);
    }
    words.push(current);
    words.into_iter().filter(|w| !w.is_empty()).collect()
}

/// Lowercased NFKC (expands ligatures like "ﬁ"), punctuation trimmed.
fn normalize(w: &str) -> String {
    w.nfkc()
        .collect::<String>()
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase()
}

fn counts(words: impl IntoIterator<Item = String>) -> HashMap<String, usize> {
    let mut m = HashMap::new();
    for w in words
        .into_iter()
        .map(|w| normalize(&w))
        .filter(|w| !w.is_empty())
    {
        *m.entry(w).or_insert(0) += 1;
    }
    m
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let path = &args[1];
    let max_pages: usize = args.get(2).map_or(Ok(3), |s| s.parse())?;
    let pdf =
        Pdf::new(Arc::new(std::fs::read(path)?)).map_err(|e| anyhow::anyhow!("open: {e:?}"))?;
    let cache = InterpreterCache::new();
    let settings = InterpreterSettings::default();
    let mut device = TextDevice::default();
    let pages = pdf.pages();
    let n = pages.len().min(max_pages);
    for page in pages.iter().take(n) {
        let (w, h) = page.render_dimensions();
        let mut ctx = Context::new(
            page.initial_transform(true).to_kurbo(),
            Rect::new(0.0, 0.0, w as f64, h as f64),
            &cache,
            page.xref(),
            settings.clone(),
        );
        interpret_page(page, &mut ctx, &mut device);
    }
    let ours = counts(words(&device.glyphs));

    let out = Command::new("pdftotext")
        .args([
            "-raw",
            "-enc",
            "UTF-8",
            "-f",
            "1",
            "-l",
            &n.to_string(),
            path,
            "-",
        ])
        .output()?;
    let theirs = counts(
        String::from_utf8_lossy(&out.stdout)
            .split_whitespace()
            .map(str::to_owned),
    );
    let total: usize = theirs.values().sum();
    let found: usize = theirs
        .iter()
        .map(|(w, c)| (*c).min(ours.get(w).copied().unwrap_or(0)))
        .sum();
    let ours_total: usize = ours.values().sum();
    let recall = if total == 0 {
        f64::NAN
    } else {
        found as f64 / total as f64
    };
    println!(
        "{}\t{}\t{}\t{}\t{}\t{:.3}",
        path.rsplit('/').next().unwrap_or(path),
        device.glyphs.len() + device.missing_unicode,
        device.missing_unicode,
        ours_total,
        total,
        recall
    );
    Ok(())
}
