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
mod search;

pub use search::{Query, SearchText};

use std::ops::Range;

use hayro::hayro_interpret::font::{Glyph, GlyphRun};
use hayro::hayro_interpret::hayro_cmap::BfString;
use hayro::hayro_interpret::{
    BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image, ImageDrawProps,
    InterpreterCache, InterpreterSettings, RenderContent, SoftMask, interpret_page,
};
use hayro::hayro_syntax::Pdf;
use hayro::kurbo::{Affine, BezPath, Point, Rect, Shape, Vec2};
use specto_geom::{PdfPoint, PdfQuad, PdfVector};

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

    // --- selection ------------------------------------------------------

    /// The position between characters (`0..=chars.len()`) closest to `p`,
    /// for selecting: on the nearest line, before or after the character
    /// nearest to `p` along it, whichever half `p` is in. Works for points
    /// off the text too (margins, between lines), so a drag can go anywhere.
    pub fn position_at(&self, p: PdfPoint) -> usize {
        let Some(line) = self
            .lines
            .iter()
            .map(|l| (l, self.line_frame(l).distance(p)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(l, _)| l)
        else {
            return 0;
        };
        // The character nearest to `p` along the line.
        let (i, along) = line
            .chars
            .clone()
            .map(|i| {
                let c = &self.chars[i];
                let d = c.direction();
                let start = (c.origin().to_vector()).dot(d);
                let end = (c.end().to_vector()).dot(d);
                let at = p.to_vector().dot(d);
                let dist = if at < start.min(end) {
                    start.min(end) - at
                } else if at > start.max(end) {
                    at - start.max(end)
                } else {
                    0.0
                };
                (i, dist, (at - start) / (end - start).abs().max(1e-9))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _, along)| (i, along))
            .unwrap_or((line.chars.start, 0.0));
        if along < 0.5 { i } else { i + 1 }
    }

    /// The line whose box contains `p` (for showing a text cursor).
    pub fn line_at(&self, p: PdfPoint) -> Option<&Line> {
        self.lines
            .iter()
            .find(|l| self.line_frame(l).distance(p) == 0.0)
    }

    /// Characters of the word containing character `c`, or just `c` if it
    /// isn't part of a word (a space).
    pub fn word_range(&self, c: usize) -> Range<usize> {
        self.word_of(c)
            .map_or(c..(c + 1).min(self.chars.len()), |w| w.chars.clone())
    }

    /// Characters of the line containing character `c`.
    pub fn line_range(&self, c: usize) -> Range<usize> {
        self.line_of(c)
            .map_or(c..(c + 1).min(self.chars.len()), |l| l.chars.clone())
    }

    /// The area to highlight for selected characters `range`: one quad per
    /// line (rotated with the text), spanning the selected characters along
    /// the line and the full line height across it.
    pub fn selection_quads(&self, range: Range<usize>) -> Vec<PdfQuad> {
        let mut out = Vec::new();
        for line in &self.lines {
            let start = line.chars.start.max(range.start);
            let end = line.chars.end.min(range.end);
            if start >= end {
                continue;
            }
            let frame = self.line_frame(line);
            let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
            for c in &self.chars[start..end] {
                for q in c.quad.points {
                    let a = q.to_vector().dot(frame.dir);
                    lo = lo.min(a);
                    hi = hi.max(a);
                }
            }
            out.push(frame.quad(lo, hi));
        }
        out
    }

    /// A line's extent in its own frame: along its direction and across it.
    fn line_frame(&self, line: &Line) -> LineFrame {
        let dir = self.chars[line.chars.start].direction();
        let up = PdfVector::new(-dir.y, dir.x);
        let mut f = LineFrame {
            dir,
            up,
            along: (f64::INFINITY, f64::NEG_INFINITY),
            across: (f64::INFINITY, f64::NEG_INFINITY),
        };
        for c in &self.chars[line.chars.clone()] {
            for q in c.quad.points {
                let (a, b) = (q.to_vector().dot(dir), q.to_vector().dot(up));
                f.along = (f.along.0.min(a), f.along.1.max(a));
                f.across = (f.across.0.min(b), f.across.1.max(b));
            }
        }
        f
    }
}

/// A line's box in a frame aligned with its text direction.
struct LineFrame {
    dir: PdfVector,
    up: PdfVector,
    along: (f64, f64),
    across: (f64, f64),
}

impl LineFrame {
    /// How far `p` is from the line, for picking the line a pointer is on:
    /// the distance across the line counts fully, the distance along it a
    /// tenth. So in the margin the line at the pointer's height wins over a
    /// longer line above it, while in two columns the line in the pointer's
    /// own column still wins.
    fn distance(&self, p: PdfPoint) -> f64 {
        let outside = |v: f64, (lo, hi): (f64, f64)| (lo - v).max(v - hi).max(0.0);
        let along = outside(p.to_vector().dot(self.dir), self.along);
        let across = outside(p.to_vector().dot(self.up), self.across);
        across + 0.1 * along
    }

    /// The quad from `lo` to `hi` along the line, at full line height.
    fn quad(&self, lo: f64, hi: f64) -> PdfQuad {
        let at = |a: f64, b: f64| (self.dir * a + self.up * b).to_point();
        let (bottom, top) = self.across;
        PdfQuad::new([at(lo, bottom), at(hi, bottom), at(hi, top), at(lo, top)])
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
