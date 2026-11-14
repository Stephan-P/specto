//! The text layer: the characters on a page with their positions, grouped
//! into words and lines in reading order. Selection, copy and search are
//! built on this.
//!
//! Glyphs come from hayro's interpreter through a custom [`Device`]; hayro
//! maps them to Unicode (`ToUnicode`, glyph names). Positions are in **PDF
//! space** of the page, which doesn't change when the user rotates a page;
//! viewers convert with the page's geometry.
//!
//! Reading order is the order of the content stream (like `pdftotext -raw`),
//! which is what most generators produce. Lines are built from consecutive
//! glyphs that share a direction and baseline; words split at spaces and gaps.

mod bidi;
mod group;

use std::ops::Range;

use hayro::hayro_interpret::font::{Glyph, GlyphRun};
use hayro::hayro_interpret::hayro_cmap::BfString;
use hayro::hayro_interpret::{
    BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image, ImageDrawProps,
    InterpreterCache, InterpreterSettings, RenderContent, SoftMask, interpret_page,
};
use hayro::hayro_syntax::Pdf;
use hayro::kurbo::{Affine, BezPath, Point, Rect, Shape, Vec2};
use pdfrs_geom::{PdfPoint, PdfQuad, PdfVector};

/// Glyph space is 1000 units per em (hayro's convention).
const EM: f64 = 1000.0;
/// Approximate ascent and descent, as fractions of the em.
const ASCENT: f64 = 0.8;
const DESCENT: f64 = -0.2;

/// One character (or ligature, e.g. "ﬁ" → "fi") on the page.
#[derive(Clone, Debug, PartialEq)]
pub struct TextChar {
    /// Usually one character. Empty if the glyph has no known Unicode value;
    /// it still takes part in geometry (selection), but not in text.
    pub text: String,
    /// The glyph's box: baseline to ascent, origin to advance.
    pub quad: PdfQuad,
    /// Font size: length of the em vector in PDF units.
    pub size: f64,
}

impl TextChar {
    /// Unit vector along the baseline (reading direction).
    pub fn direction(&self) -> PdfVector {
        let v = self.quad.points[1] - self.quad.points[0];
        let len = v.length();
        if len > 1e-9 {
            v / len
        } else {
            PdfVector::new(1.0, 0.0)
        }
    }

    pub fn origin(&self) -> PdfPoint {
        self.quad.points[0]
    }

    pub fn end(&self) -> PdfPoint {
        self.quad.points[1]
    }
}

/// A word: a run of characters without whitespace or wide gaps.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Word {
    pub chars: Range<usize>,
}

/// A line: words on one baseline in one direction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub words: Range<usize>,
    pub chars: Range<usize>,
}

/// The text layer of one page.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextPage {
    pub chars: Vec<TextChar>,
    pub words: Vec<Word>,
    pub lines: Vec<Line>,
}

impl TextPage {
    /// Builds words and lines from characters in content order.
    pub fn from_chars(chars: Vec<TextChar>) -> Self {
        let (words, lines) = group::group(&chars);
        Self {
            chars,
            words,
            lines,
        }
    }

    /// All text: words separated by spaces, lines by newlines.
    pub fn text(&self) -> String {
        self.text_range(0..self.chars.len())
    }

    /// The text of characters `range`, with spaces between words and
    /// newlines between lines, as copied from a selection. Right-to-left
    /// text comes out in logical order.
    pub fn text_range(&self, range: Range<usize>) -> String {
        let mut out = String::new();
        for line in &self.lines {
            // (text, position along the line) of each word in range.
            let mut words: Vec<(String, f64)> = Vec::new();
            let dir = self.chars[line.chars.start].direction();
            for w in &self.words[line.words.clone()] {
                let start = w.chars.start.max(range.start);
                let end = w.chars.end.min(range.end);
                if start >= end {
                    continue;
                }
                let chars = &self.chars[start..end];
                let mut text: String = chars.iter().map(|c| c.text.as_str()).collect();
                if text.is_empty() {
                    continue;
                }
                // RTL drawn left to right (visual order): reverse.
                let along = |c: &TextChar| c.origin().to_vector().dot(dir);
                let visual = along(&chars[chars.len() - 1]) >= along(&chars[0]);
                if bidi::mostly_rtl(&text) && visual && chars.len() > 1 {
                    text = bidi::reverse_clusters(&text);
                }
                words.push((text, along(&chars[0])));
            }
            if words.is_empty() {
                continue;
            }
            // An RTL line whose words were drawn left to right: reverse them.
            // Numbers and punctuation are neutral and don't count.
            let rtl_words = words.iter().filter(|(t, _)| bidi::mostly_rtl(t)).count();
            let ltr_words = words
                .iter()
                .filter(|(t, _)| !bidi::mostly_rtl(t) && bidi::has_ltr_letters(t))
                .count();
            let visual = words.last().map(|w| w.1) >= words.first().map(|w| w.1);
            if rtl_words > ltr_words && visual {
                words.reverse();
            }
            if !out.is_empty() {
                out.push('\n');
            }
            let texts: Vec<String> = words.into_iter().map(|(t, _)| t).collect();
            out.push_str(&texts.join(" "));
        }
        out
    }

    /// The character whose box contains `p`.
    pub fn char_at(&self, p: PdfPoint) -> Option<usize> {
        self.chars.iter().position(|c| c.quad.contains(p))
    }

    /// The word containing character `c`.
    pub fn word_of(&self, c: usize) -> Option<&Word> {
        let i = self.words.partition_point(|w| w.chars.end <= c);
        self.words.get(i).filter(|w| w.chars.contains(&c))
    }

    /// The line containing character `c`.
    pub fn line_of(&self, c: usize) -> Option<&Line> {
        let i = self.lines.partition_point(|l| l.chars.end <= c);
        self.lines.get(i).filter(|l| l.chars.contains(&c))
    }
}

/// Extracts text layers from one document, reusing parsed fonts across pages.
pub struct Extractor<'a> {
    pdf: &'a Pdf,
    cache: InterpreterCache<'a>,
    settings: InterpreterSettings,
}

impl<'a> Extractor<'a> {
    pub fn new(pdf: &'a Pdf) -> Self {
        Self {
            pdf,
            cache: InterpreterCache::new(),
            settings: InterpreterSettings {
                // Page content only: annotations (forms, notes) are separate.
                render_content: RenderContent::PAGE,
                ..InterpreterSettings::default()
            },
        }
    }

    /// Also extract text drawn by annotations (form field values, notes).
    /// Off by default: annotations are drawn and edited separately.
    pub fn with_annotations(mut self, include: bool) -> Self {
        self.settings.render_content = if include {
            RenderContent::all()
        } else {
            RenderContent::PAGE
        };
        self
    }

    /// The text layer of page `index`, or `None` if there's no such page.
    pub fn page(&self, index: usize) -> Option<TextPage> {
        let page = self.pdf.pages().get(index)?;
        let mut ctx = Context::new(
            // Identity: glyph positions come out in PDF user space.
            Affine::IDENTITY,
            Rect::new(-1e9, -1e9, 1e9, 1e9),
            &self.cache,
            page.xref(),
            self.settings.clone(),
        );
        let mut device = TextDevice::default();
        interpret_page(page, &mut ctx, &mut device);
        Some(TextPage::from_chars(device.chars))
    }
}

/// Collects glyphs; ignores everything else.
#[derive(Default)]
struct TextDevice {
    chars: Vec<TextChar>,
}

impl<'a> Device<'a> for TextDevice {
    fn draw_path(&mut self, _: &BezPath, _: DrawProps<'a>, _: &DrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_image(&mut self, _: Image<'a, '_>, _: ImageDrawProps<'a>) {}
    fn pop_clip(&mut self) {}
    fn pop_transparency_group(&mut self) {}

    fn draw_glyph_run(&mut self, run: &GlyphRun<'_, 'a>, props: DrawProps<'a>, _: &DrawMode) {
        let vertical = matches!(
            run.writing_mode(),
            hayro::hayro_interpret::hayro_cmap::WritingMode::Vertical
        );
        for g in run.glyphs() {
            let t = props.transform * g.transform();
            let text = match g.as_unicode() {
                Some(BfString::Char(c)) => c.to_string(),
                Some(BfString::String(s)) => s,
                None => String::new(),
            };
            let advance = match &**g {
                Glyph::Outline(o) => o
                    .advance_width()
                    .map(f64::from)
                    .filter(|a| *a > 0.0)
                    .unwrap_or_else(|| o.outline().bounding_box().x1.max(EM * 0.5)),
                Glyph::Type3(_) => EM * 0.5,
            };
            let at = |x: f64, y: f64| {
                let p = t * Point::new(x, y);
                PdfPoint::new(p.x, p.y)
            };
            let quad = if vertical {
                // Vertical text advances downwards; the glyph spans the em
                // horizontally, centred on the origin.
                PdfQuad::new([
                    at(-EM / 2.0, 0.0),
                    at(-EM / 2.0, -EM),
                    at(EM / 2.0, -EM),
                    at(EM / 2.0, 0.0),
                ])
            } else {
                PdfQuad::new([
                    at(0.0, DESCENT * EM),
                    at(advance, DESCENT * EM),
                    at(advance, ASCENT * EM),
                    at(0.0, ASCENT * EM),
                ])
            };
            let em: Vec2 = t * Point::new(0.0, EM) - t * Point::ZERO;
            let size = em.hypot();
            if !size.is_finite() || size < 1e-6 {
                continue; // degenerate (invisible) text
            }
            self.chars.push(TextChar { text, quad, size });
        }
    }
}
