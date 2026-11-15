//! Grouping characters (in content order) into words and lines.

use specto_geom::{PdfPoint, PdfVector};

use crate::{ASCENT, DESCENT, Line, TextChar, Word};

/// Directions within this angle count as the same line direction.
const SAME_DIRECTION_COS: f64 = 0.985; // ~10°
/// A gap wider than this (in ems) starts a new word.
const WORD_GAP: f64 = 0.2;
/// A baseline offset larger than this (in ems of the larger glyph) starts a
/// new line; smaller offsets are super- and subscripts.
const LINE_OFFSET: f64 = 0.5;
/// Moving back further than this (in ems) starts a new line.
const BACKWARDS: f64 = 0.5;

fn baseline_start(c: &TextChar) -> PdfPoint {
    // The quad spans DESCENT..ASCENT ems; the baseline is at 0.
    let up = c.quad.points[3] - c.quad.points[0];
    c.quad.points[0] + up * (-DESCENT / (ASCENT - DESCENT))
}

fn baseline_end(c: &TextChar) -> PdfPoint {
    let up = c.quad.points[2] - c.quad.points[1];
    c.quad.points[1] + up * (-DESCENT / (ASCENT - DESCENT))
}

fn is_space(c: &TextChar) -> bool {
    !c.text.is_empty() && c.text.chars().all(char::is_whitespace)
}

/// Where `next` sits relative to the end of `prev`, in `prev`'s frame:
/// distance along the baseline direction, and offset across it.
fn relative(prev: &TextChar, next: &TextChar) -> (f64, f64) {
    let dir = prev.direction();
    let rel: PdfVector = baseline_start(next) - baseline_end(prev);
    (rel.dot(dir), dir.cross(rel))
}

/// Whether `next` starts a new line, or (if not) a new word.
fn breaks(prev: &TextChar, next: &TextChar) -> (bool, bool) {
    let em = prev.size.max(next.size);
    let same_direction = prev.direction().dot(next.direction()) > SAME_DIRECTION_COS;
    let (mut along, across) = relative(prev, next);
    // Right-to-left text in logical order: each glyph sits left of the one
    // before. Measure the gap from its end to the previous glyph's start.
    // Spaces are neutral: they continue an RTL run on either side.
    let rtl = |c: &TextChar| c.text.chars().any(crate::bidi::is_rtl);
    let rtl_or_space = |c: &TextChar| rtl(c) || is_space(c);
    if rtl_or_space(prev) && rtl_or_space(next) && (rtl(prev) || rtl(next)) && along < 0.0 {
        let dir = prev.direction();
        let back = (baseline_start(prev) - baseline_end(next)).dot(dir);
        if back > -BACKWARDS * em {
            along = back;
        }
    }
    let new_line = !same_direction || across.abs() > LINE_OFFSET * em || along < -BACKWARDS * em;
    (new_line, new_line || along > WORD_GAP * em)
}

pub(crate) fn group(chars: &[TextChar]) -> (Vec<Word>, Vec<Line>) {
    let mut words: Vec<Word> = Vec::new();
    let mut lines: Vec<Line> = Vec::new();
    let mut word_start: Option<usize> = None;
    let (mut line_char, mut line_word) = (0, 0);

    let close_word = |words: &mut Vec<Word>, start: &mut Option<usize>, end: usize| {
        if let Some(s) = start.take() {
            words.push(Word { chars: s..end });
        }
    };

    for (i, c) in chars.iter().enumerate() {
        if i > 0 {
            let (new_line, new_word) = breaks(&chars[i - 1], c);
            if new_word {
                close_word(&mut words, &mut word_start, i);
            }
            if new_line {
                if i > line_char {
                    lines.push(Line {
                        words: line_word..words.len(),
                        chars: line_char..i,
                    });
                }
                line_char = i;
                line_word = words.len();
            }
        }
        if is_space(c) {
            // Spaces end words and belong to none.
            close_word(&mut words, &mut word_start, i);
        } else if word_start.is_none() {
            word_start = Some(i);
        }
    }
    close_word(&mut words, &mut word_start, chars.len());
    if chars.len() > line_char {
        lines.push(Line {
            words: line_word..words.len(),
            chars: line_char..chars.len(),
        });
    }
    (words, lines)
}
