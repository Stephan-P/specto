//! Decoding PDF text strings (titles, outline entries, metadata).

/// Decodes a PDF "text string".
///
/// UTF-16BE or UTF-8 with a byte-order mark, otherwise PDFDocEncoding. Invalid sequences become
/// U+FFFD.
pub fn decode_text(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        let (pairs, _odd_byte) = rest.as_chunks::<2>();
        let units: Vec<u16> = pairs.iter().map(|&p| u16::from_be_bytes(p)).collect();
        return String::from_utf16_lossy(&units);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    bytes.iter().map(|&b| pdf_doc_char(b)).collect()
}

/// PDFDocEncoding (ISO 32000-2, Annex D): Latin-1 except for a few ranges.
fn pdf_doc_char(b: u8) -> char {
    const HIGH: [char; 32] = [
        '•', '†', '‡', '…', '—', '–', 'ƒ', '⁄', '‹', '›', '−', '‰', '„', '“', '”', '‘', //
        '’', '‚', '™', 'ﬁ', 'ﬂ', 'Ł', 'Œ', 'Š', 'Ÿ', 'Ž', 'ı', 'ł', 'œ', 'š', 'ž', '\u{FFFD}',
    ];
    const LOW: [char; 8] = ['˘', 'ˇ', 'ˆ', '˙', '˝', '˛', '˚', '˜'];
    match b {
        0x18..=0x1F => LOW[(b - 0x18) as usize],
        0x80..=0x9F => HIGH[(b - 0x80) as usize],
        0xA0 => '€',
        0xAD => '\u{FFFD}',
        _ => b as char,
    }
}

#[cfg(test)]
mod tests {
    use super::decode_text;

    #[test]
    fn decodes_the_three_encodings() {
        assert_eq!(decode_text(b"Chapter 1"), "Chapter 1");
        assert_eq!(decode_text(&[0xFE, 0xFF, 0x00, 0x48, 0x00, 0xE9]), "Hé");
        assert_eq!(decode_text(&[0xEF, 0xBB, 0xBF, 0xC3, 0xA9]), "é");
        // PDFDocEncoding: 0x84 is an em dash, 0xE9 is é (as in Latin-1).
        assert_eq!(decode_text(&[b'a', 0x84, 0xE9]), "a—é");
    }
}
