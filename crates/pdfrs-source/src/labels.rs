//! Page labels (`/PageLabels`): the page "numbers" printed in the document,
//! like "i, ii, …, 1, 2, …" or "A-1".

/// Numbering style of a label range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LabelStyle {
    /// 1, 2, 3
    Decimal,
    /// I, II, III
    UpperRoman,
    /// i, ii, iii
    LowerRoman,
    /// A, B, …, Z, AA, BB, …
    UpperLetters,
    /// a, b, …, z, aa, bb, …
    LowerLetters,
    /// Prefix only, no number.
    None,
}

/// One range: from page `start` on, labels are `prefix` + number in `style`,
/// counting from `first`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LabelRange {
    pub start: usize,
    pub style: LabelStyle,
    pub prefix: String,
    pub first: u32,
}

/// The labels of a document. Pages before the first range (if a broken file
/// doesn't start at 0) get plain decimal numbers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageLabels {
    /// Sorted by `start`, without duplicates.
    ranges: Vec<LabelRange>,
    page_count: usize,
}

impl PageLabels {
    /// `ranges` in any order; ranges starting beyond the last page are
    /// dropped. Returns `None` if nothing remains.
    pub fn new(mut ranges: Vec<LabelRange>, page_count: usize) -> Option<Self> {
        ranges.retain(|r| r.start < page_count);
        ranges.sort_by_key(|r| r.start);
        ranges.dedup_by_key(|r| r.start);
        (!ranges.is_empty()).then_some(Self { ranges, page_count })
    }

    /// The label of page `index` (zero-based).
    pub fn label(&self, index: usize) -> String {
        let i = self.ranges.partition_point(|r| r.start <= index);
        let Some(range) = i.checked_sub(1).map(|i| &self.ranges[i]) else {
            return (index + 1).to_string();
        };
        let n = range.first as u64 + (index - range.start) as u64;
        let number = match range.style {
            LabelStyle::Decimal => n.to_string(),
            LabelStyle::UpperRoman => roman(n),
            LabelStyle::LowerRoman => roman(n).to_lowercase(),
            LabelStyle::UpperLetters => letters(n),
            LabelStyle::LowerLetters => letters(n).to_lowercase(),
            LabelStyle::None => String::new(),
        };
        format!("{}{number}", range.prefix)
    }

    /// The first page whose label is `text`: exactly, then ignoring case.
    pub fn find(&self, text: &str) -> Option<usize> {
        let text = text.trim();
        let labels: Vec<String> = (0..self.page_count).map(|i| self.label(i)).collect();
        labels
            .iter()
            .position(|l| l == text)
            .or_else(|| labels.iter().position(|l| l.eq_ignore_ascii_case(text)))
    }

    /// Whether every label is just the page number (1, 2, 3, …), in which
    /// case there's no point showing labels.
    pub fn is_plain(&self) -> bool {
        (0..self.page_count).all(|i| self.label(i) == (i + 1).to_string())
    }

    pub fn ranges(&self) -> &[LabelRange] {
        &self.ranges
    }
}

/// Roman numerals; large numbers repeat `M` (as other viewers do).
fn roman(mut n: u64) -> String {
    if n == 0 {
        return String::new();
    }
    const DIGITS: [(u64, &str); 13] = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    // Absurdly large values from broken files: don't build huge strings.
    n = n.min(100_000);
    let mut out = String::new();
    for (value, digits) in DIGITS {
        while n >= value {
            out.push_str(digits);
            n -= value;
        }
    }
    out
}

/// A, B, …, Z, AA, BB, …, ZZ, AAA, … (the letter repeated, per the spec).
fn letters(n: u64) -> String {
    if n == 0 {
        return String::new();
    }
    let letter = (b'A' + ((n - 1) % 26) as u8) as char;
    let count = ((n - 1) / 26 + 1).min(1000) as usize;
    std::iter::repeat_n(letter, count).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(start: usize, style: LabelStyle, prefix: &str, first: u32) -> LabelRange {
        LabelRange {
            start,
            style,
            prefix: prefix.into(),
            first,
        }
    }

    #[test]
    fn styles() {
        assert_eq!(roman(1994), "MCMXCIV");
        assert_eq!(roman(4), "IV");
        assert_eq!(letters(1), "A");
        assert_eq!(letters(26), "Z");
        assert_eq!(letters(27), "AA");
        assert_eq!(letters(53), "AAA");
    }

    #[test]
    fn book_with_front_matter_and_appendix() {
        // i–iv, then 1–5, then A-1…
        let labels = PageLabels::new(
            vec![
                range(9, LabelStyle::Decimal, "A-", 1),
                range(0, LabelStyle::LowerRoman, "", 1),
                range(4, LabelStyle::Decimal, "", 1),
            ],
            11,
        )
        .unwrap();
        let all: Vec<String> = (0..11).map(|i| labels.label(i)).collect();
        assert_eq!(
            all,
            [
                "i", "ii", "iii", "iv", "1", "2", "3", "4", "5", "A-1", "A-2"
            ]
        );
        assert_eq!(labels.find("iv"), Some(3));
        assert_eq!(labels.find("IV"), Some(3)); // case-insensitive fallback
        assert_eq!(labels.find("3"), Some(6));
        assert_eq!(labels.find("A-2"), Some(10));
        assert_eq!(labels.find("nope"), None);
        assert!(!labels.is_plain());
    }

    #[test]
    fn edge_cases() {
        // A range that doesn't start at page 0: earlier pages are numbered.
        let l = PageLabels::new(vec![range(2, LabelStyle::None, "Cover", 1)], 4).unwrap();
        assert_eq!(
            (0..4).map(|i| l.label(i)).collect::<Vec<_>>(),
            ["1", "2", "Cover", "Cover"]
        );
        // Plain decimal labels are reported as such.
        let plain = PageLabels::new(vec![range(0, LabelStyle::Decimal, "", 1)], 5).unwrap();
        assert!(plain.is_plain());
        // Ranges past the end are dropped.
        assert!(PageLabels::new(vec![range(9, LabelStyle::Decimal, "", 1)], 5).is_none());
    }
}
