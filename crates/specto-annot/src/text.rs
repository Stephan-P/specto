//! Text in annotations: text boxes are set in Helvetica, one of the fonts
//! every PDF program has, so nothing needs embedding. That limits them to
//! the characters of Windows' Latin code page for now.
//!
//! The same line breaks are used wherever a text box is measured or
//! written, so its text sits the same on screen and in the file.

/// The width of each character of Helvetica from code 32 on, in thousandths
/// of the font size (from the font's metrics, as Adobe publishes them;
/// codes are those of Windows' Latin code page, `WinAnsiEncoding`).
#[rustfmt::skip]
const WIDTHS: [u16; 224] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278,
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556,
    1015, 667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778,
    667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 278, 278, 278, 469, 556,
    333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556,
    556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584, 0,
    556, 0, 222, 556, 333, 1000, 556, 556, 333, 1000, 667, 333, 1000, 0, 611, 0,
    0, 222, 222, 333, 333, 350, 556, 1000, 333, 1000, 500, 333, 944, 0, 500, 667,
    278, 333, 556, 556, 556, 556, 260, 556, 333, 737, 370, 556, 584, 333, 737, 333,
    400, 584, 333, 333, 333, 556, 537, 278, 333, 333, 365, 556, 834, 834, 834, 611,
    667, 667, 667, 667, 667, 667, 1000, 722, 667, 667, 667, 667, 278, 278, 278, 278,
    722, 722, 778, 778, 778, 778, 778, 584, 778, 722, 722, 722, 722, 667, 667, 611,
    556, 556, 556, 556, 556, 556, 889, 500, 556, 556, 556, 556, 278, 278, 278, 278,
    556, 556, 556, 556, 556, 556, 556, 584, 611, 556, 556, 556, 556, 500, 556, 500,
];

/// How far above the baseline Helvetica's tallest letters reach, and how
/// far below it the longest tails, as parts of the font size.
const ASCENT: f64 = 0.718;
const DESCENT: f64 = 0.207;

/// The distance between baselines, as a multiple of the font size.
const LEADING: f64 = 1.2;

/// The space between a text box's edge and its text, in points.
pub const TEXT_MARGIN: f64 = 2.0;

/// The byte for `c` in Windows' Latin code page, which text boxes are
/// written in; `None` for characters it doesn't have.
pub fn winansi(c: char) -> Option<u8> {
    let code = c as u32;
    match code {
        0x20..=0x7E | 0xA0..=0xFF => Some(code as u8),
        _ => {
            const HIGH: [(char, u8); 27] = [
                ('€', 0x80),
                ('‚', 0x82),
                ('ƒ', 0x83),
                ('„', 0x84),
                ('…', 0x85),
                ('†', 0x86),
                ('‡', 0x87),
                ('ˆ', 0x88),
                ('‰', 0x89),
                ('Š', 0x8A),
                ('‹', 0x8B),
                ('Œ', 0x8C),
                ('Ž', 0x8E),
                ('‘', 0x91),
                ('’', 0x92),
                ('“', 0x93),
                ('”', 0x94),
                ('•', 0x95),
                ('–', 0x96),
                ('—', 0x97),
                ('˜', 0x98),
                ('™', 0x99),
                ('š', 0x9A),
                ('›', 0x9B),
                ('œ', 0x9C),
                ('ž', 0x9E),
                ('Ÿ', 0x9F),
            ];
            HIGH.iter().find(|(h, _)| *h == c).map(|(_, b)| *b)
        }
    }
}

/// `text` as the bytes a text box is written in: characters the code page
/// doesn't have become question marks.
pub(crate) fn encode(text: &str) -> Vec<u8> {
    text.chars().map(|c| winansi(c).unwrap_or(b'?')).collect()
}

/// How wide `text` is at font size `size`, in points.
pub fn text_width(text: &str, size: f64) -> f64 {
    let thousandths: u32 = encode(text)
        .iter()
        .map(|&b| u32::from(WIDTHS[usize::from(b.max(32)) - 32]))
        .sum();
    f64::from(thousandths) * size / 1000.0
}

/// `text` broken into lines no wider than `width` at font size `size`:
/// at its own line breaks, and between words where a line would get too
/// long. A word longer than a line is broken where it has to be.
pub fn wrap(text: &str, size: f64, width: f64) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.lines() {
        let mut line = String::new();
        for word in paragraph.split(' ') {
            let with_word = if line.is_empty() {
                word.to_owned()
            } else {
                format!("{line} {word}")
            };
            if text_width(&with_word, size) <= width || line.is_empty() {
                line = with_word;
            } else {
                lines.push(std::mem::replace(&mut line, word.to_owned()));
            }
            // A word too long for a line of its own.
            while text_width(&line, size) > width && line.chars().count() > 1 {
                let mut fits = 0;
                for (i, _) in line.char_indices().skip(1) {
                    if text_width(&line[..i], size) > width {
                        break;
                    }
                    fits = i;
                }
                let at = if fits == 0 {
                    line.char_indices().nth(1).map_or(line.len(), |(i, _)| i)
                } else {
                    fits
                };
                let rest = line.split_off(at);
                lines.push(std::mem::replace(&mut line, rest));
            }
        }
        lines.push(line);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

/// How high a text box of `lines` lines at font size `size` is, margins
/// included.
pub fn box_height(lines: usize, size: f64) -> f64 {
    let lines = lines.max(1) as f64;
    2.0 * TEXT_MARGIN + (ASCENT + DESCENT) * size + (lines - 1.0) * LEADING * size
}

/// Where the first baseline of a text box is below its top edge, and how
/// far apart baselines are.
pub(crate) fn baselines(size: f64) -> (f64, f64) {
    (TEXT_MARGIN + ASCENT * size, LEADING * size)
}

#[cfg(test)]
mod tests {
    use super::{box_height, encode, text_width, wrap};

    #[test]
    fn widths_and_bytes() {
        // Helvetica: a space is 278, "A" 667, "i" 222 thousandths.
        assert!((text_width("A i", 10.0) - 11.67).abs() < 1e-9);
        assert_eq!(encode("Größe “x” €"), b"Gr\xf6\xdfe \x93x\x94 \x80");
        assert_eq!(encode("日本"), b"??");
    }

    #[test]
    fn wrapping() {
        // "aaa" is 16.68 wide at size 10, a space 2.78.
        assert_eq!(wrap("aaa aaa aaa", 10.0, 40.0), ["aaa aaa", "aaa"]);
        assert_eq!(wrap("aaa\n\naaa", 10.0, 40.0), ["aaa", "", "aaa"]);
        assert_eq!(wrap("", 10.0, 40.0), [""]);
        // Too long for a line: broken where it must be.
        assert_eq!(wrap("aaaaaaaa", 10.0, 17.0), ["aaa", "aaa", "aa"]);
        assert_eq!(wrap("W", 10.0, 1.0), ["W"]);
        assert!(box_height(2, 10.0) > box_height(1, 10.0));
    }
}
