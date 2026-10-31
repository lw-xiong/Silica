//! PDF text strings (PDF 32000-1:2008 section 7.9.2.2): UTF-16 behind a byte order
//! mark, UTF-8 behind one (PDF 2.0), otherwise PDFDocEncoding.

/// Decode a text string. Undefined PDFDocEncoding bytes and unpaired UTF-16
/// surrogates become U+FFFD.
pub(crate) fn decode(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return utf16(rest, u16::from_be_bytes);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return utf16(rest, u16::from_le_bytes);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    bytes.iter().map(|&b| pdfdoc(b)).collect()
}

fn utf16(bytes: &[u8], unit: fn([u8; 2]) -> u16) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| unit([pair[0], pair[1]]))
        .collect();
    char::decode_utf16(units)
        .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect()
}

/// One PDFDocEncoding byte. The table is appendix D.2 of the spec.
fn pdfdoc(byte: u8) -> char {
    match byte {
        0x18 => '\u{02D8}',
        0x19 => '\u{02C7}',
        0x1A => '\u{02C6}',
        0x1B => '\u{02D9}',
        0x1C => '\u{02DD}',
        0x1D => '\u{02DB}',
        0x1E => '\u{02DA}',
        0x1F => '\u{02DC}',
        0x00..=0x7F => byte as char,
        0x80 => '\u{2022}',
        0x81 => '\u{2020}',
        0x82 => '\u{2021}',
        0x83 => '\u{2026}',
        0x84 => '\u{2014}',
        0x85 => '\u{2013}',
        0x86 => '\u{0192}',
        0x87 => '\u{2044}',
        0x88 => '\u{2039}',
        0x89 => '\u{203A}',
        0x8A => '\u{2212}',
        0x8B => '\u{2030}',
        0x8C => '\u{201E}',
        0x8D => '\u{201C}',
        0x8E => '\u{201D}',
        0x8F => '\u{2018}',
        0x90 => '\u{2019}',
        0x91 => '\u{201A}',
        0x92 => '\u{2122}',
        0x93 => '\u{FB01}',
        0x94 => '\u{FB02}',
        0x95 => '\u{0141}',
        0x96 => '\u{0152}',
        0x97 => '\u{0160}',
        0x98 => '\u{0178}',
        0x99 => '\u{017D}',
        0x9A => '\u{0131}',
        0x9B => '\u{0142}',
        0x9C => '\u{0153}',
        0x9D => '\u{0161}',
        0x9E => '\u{017E}',
        0x9F => char::REPLACEMENT_CHARACTER,
        0xA0 => '\u{20AC}',
        0xAD => char::REPLACEMENT_CHARACTER,
        // 0xA1..=0xFF bar the two above are Latin-1.
        _ => byte as char,
    }
}

#[cfg(test)]
mod tests {
    use super::decode;

    #[test]
    fn ascii_passes_through() {
        assert_eq!(decode(b"Pages fixture"), "Pages fixture");
    }

    #[test]
    fn pdfdoc_specials_and_latin1() {
        assert_eq!(
            decode(&[0x80, b' ', 0x93, b' ', 0xA0, b' ', 0xE9]),
            "\u{2022} \u{FB01} \u{20AC} \u{E9}"
        );
        assert_eq!(decode(&[0x9F]), "\u{FFFD}");
    }

    #[test]
    fn utf16_with_bom() {
        assert_eq!(
            decode(&[0xFE, 0xFF, 0x00, 0x48, 0x00, 0x69, 0x20, 0x14]),
            "Hi\u{2014}"
        );
        assert_eq!(decode(&[0xFF, 0xFE, 0x48, 0x00, 0x69, 0x00]), "Hi");
        // A surrogate pair, then a trailing odd byte that is dropped.
        assert_eq!(
            decode(&[0xFE, 0xFF, 0xD8, 0x3D, 0xDE, 0x00, 0x00]),
            "\u{1F600}"
        );
        assert_eq!(decode(&[0xFE, 0xFF, 0xD8, 0x3D]), "\u{FFFD}");
    }

    #[test]
    fn utf8_with_bom() {
        assert_eq!(decode("\u{FEFF}héllo".as_bytes()), "héllo");
    }
}
