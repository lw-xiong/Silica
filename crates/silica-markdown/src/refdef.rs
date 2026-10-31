//! Link reference definitions: `[label]: destination "title"`.

use crate::scan::{is_space_or_tab, scan_link_destination, scan_link_label, scan_link_title};

fn skip_spaces(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && is_space_or_tab(b[i]) {
        i += 1;
    }
    i
}

/// Skips spaces and tabs and at most one line ending.
fn skip_spnl(b: &[u8], i: usize) -> usize {
    let i = skip_spaces(b, i);
    if i < b.len() && b[i] == b'\n' {
        skip_spaces(b, i + 1)
    } else {
        i
    }
}

/// Parses a definition at the start of `text` (paragraph lines joined with
/// `\n`). Returns the label, raw destination, raw title, and the number of
/// bytes consumed; the consumed region always ends at a line end.
pub(crate) fn parse_reference_definition(text: &str) -> Option<(&str, &str, Option<&str>, usize)> {
    let b = text.as_bytes();
    let mut i = skip_spaces(b, 0);
    if i > 3 {
        return None;
    }
    let label_len = scan_link_label(&text[i..])?;
    let label = &text[i + 1..i + label_len - 1];
    i += label_len;
    if b.get(i) != Some(&b':') {
        return None;
    }
    i += 1;
    i = skip_spnl(b, i);
    let (dest_len, dest) = scan_link_destination(&text[i..])?;
    if dest_len == 0 {
        return None;
    }
    i += dest_len;
    let after_dest = i;

    // Optional title, separated by whitespace, must end its line.
    let j = skip_spnl(b, i);
    if j > i {
        if let Some((title_len, title)) = scan_link_title(&text[j..]) {
            let k = skip_spaces(b, j + title_len);
            if k >= b.len() {
                return Some((label, dest, Some(title), k));
            }
            if b[k] == b'\n' {
                return Some((label, dest, Some(title), k + 1));
            }
        }
    }
    let k = skip_spaces(b, after_dest);
    if k >= b.len() {
        return Some((label, dest, None, k));
    }
    if b[k] == b'\n' {
        return Some((label, dest, None, k + 1));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_definitions() {
        assert_eq!(
            parse_reference_definition("[foo]: /url \"title\"\nrest"),
            Some(("foo", "/url", Some("title"), 20))
        );
        assert_eq!(
            parse_reference_definition("[foo]:\n  /url\n  'the title'"),
            Some(("foo", "/url", Some("the title"), 27))
        );
        assert_eq!(parse_reference_definition("[foo]: /url \"title\" ok"), None);
        assert_eq!(
            parse_reference_definition("[foo]: /url\n\"title\" ok"),
            Some(("foo", "/url", None, 12))
        );
        assert_eq!(
            parse_reference_definition("[foo]: <>"),
            Some(("foo", "", None, 9))
        );
        assert_eq!(parse_reference_definition("[foo]:"), None);
    }
}
