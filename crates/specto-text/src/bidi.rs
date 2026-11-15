//! Right-to-left text (Hebrew, Arabic) in logical order.
//!
//! PDFs place glyphs on the page. Most generators draw right-to-left text in *visual* order (left
//! to right on the page), some in *logical* order. Text output (copy, search) needs logical order,
//! so right-to-left words drawn in visual order have their characters reversed, and right-to-left
//! lines drawn in visual order have their word order reversed. Geometry and character indices are
//! untouched: selection follows what's on screen.

/// Strong right-to-left characters: Hebrew, Arabic, Syriac, Thaana, N'Ko and their presentation
/// forms.
///
/// Historic right-to-left scripts (U+10800 and up) are left out on purpose: they're rare, and
/// mostly appear in code charts laid out left to right, which Poppler doesn't reorder either.
pub(crate) fn is_rtl(c: char) -> bool {
    matches!(c,
        '\u{0590}'..='\u{08FF}'
        | '\u{FB1D}'..='\u{FDFF}'
        | '\u{FE70}'..='\u{FEFF}')
}

/// Strong left-to-right characters (letters of other scripts).
fn is_ltr(c: char) -> bool {
    c.is_alphabetic() && !is_rtl(c)
}

/// Whether a string is predominantly right-to-left.
pub(crate) fn mostly_rtl(s: &str) -> bool {
    let (mut rtl, mut ltr) = (0, 0);
    for c in s.chars() {
        if is_rtl(c) {
            rtl += 1;
        } else if is_ltr(c) {
            ltr += 1;
        }
    }
    rtl > ltr
}

/// Whether a string contains letters of a left-to-right script.
pub(crate) fn has_ltr_letters(s: &str) -> bool {
    s.chars().any(is_ltr)
}

/// Combining marks (Hebrew points, Arabic harakat, generic diacritics) that belong to the preceding
/// base character.
fn is_mark(c: char) -> bool {
    matches!(c,
        '\u{0300}'..='\u{036F}'
        | '\u{0591}'..='\u{05BD}' | '\u{05BF}' | '\u{05C1}'..='\u{05C2}'
        | '\u{05C4}'..='\u{05C5}' | '\u{05C7}'
        | '\u{0610}'..='\u{061A}' | '\u{064B}'..='\u{065F}' | '\u{0670}'
        | '\u{06D6}'..='\u{06DC}' | '\u{06DF}'..='\u{06E4}' | '\u{06E7}'..='\u{06E8}'
        | '\u{06EA}'..='\u{06ED}'
        | '\u{FE20}'..='\u{FE2F}')
}

/// Reverses `s` by clusters (a base character plus its marks), so marks stay on their letter.
pub(crate) fn reverse_clusters(s: &str) -> String {
    let mut clusters: Vec<String> = Vec::new();
    for c in s.chars() {
        match clusters.last_mut() {
            Some(last) if is_mark(c) => last.push(c),
            _ => clusters.push(c.to_string()),
        }
    }
    clusters.into_iter().rev().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detection_and_reversal() {
        assert!(mostly_rtl("שלום"));
        assert!(mostly_rtl("مرحبا"));
        assert!(!mostly_rtl("hello"));
        assert!(!mostly_rtl("15"));
        assert_eq!(reverse_clusters("םולש"), "שלום");
        // The mark (sheva, U+05B0) stays with its letter.
        assert_eq!(reverse_clusters("אב\u{05B0}"), "ב\u{05B0}א");
    }
}
