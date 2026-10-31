//! Low-level scanners shared by the block and inline parsers: link labels,
//! destinations and titles, raw HTML tags, entities, and backslash unescaping.

use crate::entities::ENTITIES;
use std::borrow::Cow;

pub(crate) fn is_ascii_punct(b: u8) -> bool {
    b.is_ascii_punctuation()
}

pub(crate) fn is_space_or_tab(b: u8) -> bool {
    b == b' ' || b == b'\t'
}

/// Unicode whitespace per CommonMark (Zs plus tab, LF, FF, CR).
pub(crate) fn is_unicode_whitespace(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\u{0c}' | '\r') || c.is_whitespace()
}

/// Unicode punctuation per CommonMark: the P and S general categories.
pub(crate) fn is_unicode_punctuation(c: char) -> bool {
    if c.is_ascii() {
        return c.is_ascii_punctuation();
    }
    !c.is_alphanumeric() && !c.is_whitespace() && !c.is_control()
}

/// Looks up a named entity (without `&` and `;`).
pub(crate) fn named_entity(name: &str) -> Option<&'static str> {
    ENTITIES
        .binary_search_by(|(n, _)| (*n).cmp(name))
        .ok()
        .map(|i| ENTITIES[i].1)
}

/// Looks up a legacy entity that needs no semicolon (`&copy`); returns the
/// longest matching name length and its text.
pub(crate) fn legacy_entity(s: &str) -> Option<(usize, &'static str)> {
    use crate::entities::LEGACY_ENTITIES;
    let max = s
        .bytes()
        .take(8)
        .take_while(|b| b.is_ascii_alphanumeric())
        .count();
    for len in (2..=max).rev() {
        if let Ok(i) = LEGACY_ENTITIES.binary_search_by(|(n, _)| (*n).cmp(&s[..len])) {
            return Some((len, LEGACY_ENTITIES[i].1));
        }
    }
    None
}

/// Scans an entity or numeric character reference starting at `&`. Returns
/// the total length and the decoded text.
pub(crate) fn scan_entity(s: &str) -> Option<(usize, Cow<'static, str>)> {
    let b = s.as_bytes();
    if b.first() != Some(&b'&') {
        return None;
    }
    if b.get(1) == Some(&b'#') {
        let (radix, start) = if matches!(b.get(2), Some(b'x') | Some(b'X')) {
            (16, 3)
        } else {
            (10, 2)
        };
        let max = if radix == 16 { 6 } else { 7 };
        let mut i = start;
        while i < b.len() && i - start < max && (b[i] as char).is_digit(radix) {
            i += 1;
        }
        if i == start || b.get(i) != Some(&b';') {
            return None;
        }
        let code = u32::from_str_radix(&s[start..i], radix).unwrap_or(0);
        let c = match code {
            0 => '\u{fffd}',
            _ => char::from_u32(code).unwrap_or('\u{fffd}'),
        };
        return Some((i + 1, Cow::Owned(c.to_string())));
    }
    let mut i = 1;
    while i < b.len() && i < 33 && b[i].is_ascii_alphanumeric() {
        i += 1;
    }
    if i == 1 || b.get(i) != Some(&b';') {
        return None;
    }
    named_entity(&s[1..i]).map(|t| (i + 1, Cow::Borrowed(t)))
}

/// Decodes backslash escapes and entities.
pub(crate) fn unescape(s: &str) -> Cow<'_, str> {
    if !s.contains(['\\', '&']) {
        return Cow::Borrowed(s);
    }
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\\' if i + 1 < b.len() && is_ascii_punct(b[i + 1]) => {
                out.push(b[i + 1] as char);
                i += 2;
            }
            b'&' => {
                if let Some((len, text)) = scan_entity(&s[i..]) {
                    out.push_str(&text);
                    i += len;
                } else {
                    out.push('&');
                    i += 1;
                }
            }
            _ => {
                let c = s[i..].chars().next().unwrap();
                out.push(c);
                i += c.len_utf8();
            }
        }
    }
    Cow::Owned(out)
}

/// Scans `[label]` at the start of `s`: no unescaped brackets inside, at most
/// 999 characters, at least one non-whitespace character. Returns the total
/// length including brackets.
pub(crate) fn scan_link_label(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    if b.first() != Some(&b'[') {
        return None;
    }
    let mut i = 1;
    let mut nonspace = false;
    while i < b.len() {
        match b[i] {
            b'\\' if i + 1 < b.len() && is_ascii_punct(b[i + 1]) => {
                nonspace = true;
                i += 2;
            }
            b'[' => return None,
            b']' => {
                if !nonspace || i - 1 > 999 {
                    return None;
                }
                return Some(i + 1);
            }
            c => {
                if !is_space_or_tab(c) && c != b'\n' && c != b'\r' {
                    nonspace = true;
                }
                i += 1;
            }
        }
    }
    None
}

/// Scans a link destination at the start of `s`. Returns the total length
/// and the raw (still escaped) destination text.
pub(crate) fn scan_link_destination(s: &str) -> Option<(usize, &str)> {
    let b = s.as_bytes();
    if b.first() == Some(&b'<') {
        let mut i = 1;
        while i < b.len() {
            match b[i] {
                b'\\' if i + 1 < b.len() && is_ascii_punct(b[i + 1]) => i += 2,
                b'\n' | b'\r' | b'<' => return None,
                b'>' => return Some((i + 1, &s[1..i])),
                _ => i += 1,
            }
        }
        return None;
    }
    let mut i = 0;
    let mut depth = 0usize;
    while i < b.len() {
        match b[i] {
            b'\\' if i + 1 < b.len() && is_ascii_punct(b[i + 1]) => i += 2,
            b'(' => {
                depth += 1;
                if depth > 32 {
                    return None;
                }
                i += 1;
            }
            b')' => {
                if depth == 0 {
                    break;
                }
                depth -= 1;
                i += 1;
            }
            c if c <= b' ' || c == 0x7f => break,
            _ => i += 1,
        }
    }
    if depth != 0 {
        return None;
    }
    Some((i, &s[..i]))
}

/// Scans a link title (`"…"`, `'…'` or `(…)`) at the start of `s`. Returns the
/// total length and the raw inner text.
pub(crate) fn scan_link_title(s: &str) -> Option<(usize, &str)> {
    let b = s.as_bytes();
    let close = match b.first()? {
        b'"' => b'"',
        b'\'' => b'\'',
        b'(' => b')',
        _ => return None,
    };
    let mut i = 1;
    while i < b.len() {
        match b[i] {
            b'\\' if i + 1 < b.len() && is_ascii_punct(b[i + 1]) => i += 2,
            c if c == close => return Some((i + 1, &s[1..i])),
            b'(' if close == b')' => return None,
            _ => i += 1,
        }
    }
    None
}

fn is_tag_name_start(b: u8) -> bool {
    b.is_ascii_alphabetic()
}

fn is_tag_name_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-'
}

fn is_attr_name_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b == b':'
}

fn is_attr_name_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b':' | b'-')
}

fn skip_whitespace(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\n' | b'\r' | 0x0c) {
        i += 1;
    }
    i
}

/// Scans an HTML open tag `<name attr="v">` at the start of `s`; returns its length.
pub(crate) fn scan_open_tag(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    if b.first() != Some(&b'<') || !is_tag_name_start(*b.get(1)?) {
        return None;
    }
    let mut i = 2;
    while i < b.len() && is_tag_name_char(b[i]) {
        i += 1;
    }
    loop {
        let ws_start = i;
        i = skip_whitespace(b, i);
        match b.get(i) {
            Some(b'>') => return Some(i + 1),
            Some(b'/') => {
                return if b.get(i + 1) == Some(&b'>') {
                    Some(i + 2)
                } else {
                    None
                };
            }
            Some(&c) if i > ws_start && is_attr_name_start(c) => {
                i += 1;
                while i < b.len() && is_attr_name_char(b[i]) {
                    i += 1;
                }
                let after_name = i;
                let j = skip_whitespace(b, i);
                if b.get(j) == Some(&b'=') {
                    let mut k = skip_whitespace(b, j + 1);
                    match b.get(k) {
                        Some(b'"') => {
                            k += 1;
                            while k < b.len() && b[k] != b'"' {
                                k += 1;
                            }
                            if k >= b.len() {
                                return None;
                            }
                            i = k + 1;
                        }
                        Some(b'\'') => {
                            k += 1;
                            while k < b.len() && b[k] != b'\'' {
                                k += 1;
                            }
                            if k >= b.len() {
                                return None;
                            }
                            i = k + 1;
                        }
                        Some(&c)
                            if !matches!(
                                c,
                                b' ' | b'\t'
                                    | b'\n'
                                    | b'\r'
                                    | b'"'
                                    | b'\''
                                    | b'='
                                    | b'<'
                                    | b'>'
                                    | b'`'
                            ) =>
                        {
                            while k < b.len()
                                && !matches!(
                                    b[k],
                                    b' ' | b'\t'
                                        | b'\n'
                                        | b'\r'
                                        | b'"'
                                        | b'\''
                                        | b'='
                                        | b'<'
                                        | b'>'
                                        | b'`'
                                )
                            {
                                k += 1;
                            }
                            i = k;
                        }
                        _ => return None,
                    }
                } else {
                    i = after_name;
                }
            }
            _ => return None,
        }
    }
}

/// Scans a closing tag `</name >` at the start of `s`; returns its length.
pub(crate) fn scan_closing_tag(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    if b.len() < 3 || b[0] != b'<' || b[1] != b'/' || !is_tag_name_start(b[2]) {
        return None;
    }
    let mut i = 3;
    while i < b.len() && is_tag_name_char(b[i]) {
        i += 1;
    }
    i = skip_whitespace(b, i);
    (b.get(i) == Some(&b'>')).then_some(i + 1)
}

/// Scans a comment, processing instruction, declaration or CDATA section; returns its length.
pub(crate) fn scan_html_special(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    if b.first() != Some(&b'<') {
        return None;
    }
    if s.starts_with("<!-->") {
        return Some(5);
    }
    if s.starts_with("<!--->") {
        return Some(6);
    }
    if let Some(rest) = s.strip_prefix("<!--") {
        return rest.find("-->").map(|i| 4 + i + 3);
    }
    if let Some(rest) = s.strip_prefix("<?") {
        return rest.find("?>").map(|i| 2 + i + 2);
    }
    if let Some(rest) = s.strip_prefix("<![CDATA[") {
        return rest.find("]]>").map(|i| 9 + i + 3);
    }
    if b.len() > 2 && b[1] == b'!' && b[2].is_ascii_alphabetic() {
        return s[2..].find('>').map(|i| 2 + i + 1);
    }
    None
}

/// Scans any inline raw HTML construct at the start of `s`.
pub(crate) fn scan_inline_html(s: &str) -> Option<usize> {
    scan_open_tag(s)
        .or_else(|| scan_closing_tag(s))
        .or_else(|| scan_html_special(s))
}

/// Scans an autolink `<scheme:…>` or `<email>`; returns `(len, is_email)`.
/// `min_scheme` is 2 for CommonMark; Obsidian accepts one-letter schemes.
pub(crate) fn scan_autolink(s: &str, min_scheme: usize) -> Option<(usize, bool)> {
    let b = s.as_bytes();
    if b.first() != Some(&b'<') {
        return None;
    }
    // URI autolink: scheme of 2..=32 chars then ':' then no spaces, '<' or '>'.
    if b.get(1).is_some_and(|c| c.is_ascii_alphabetic()) {
        let mut i = 2;
        while i < b.len()
            && i < 34
            && (b[i].is_ascii_alphanumeric() || matches!(b[i], b'+' | b'.' | b'-'))
        {
            i += 1;
        }
        if i > min_scheme && b.get(i) == Some(&b':') {
            let mut j = i + 1;
            while j < b.len() && !matches!(b[j], b'<' | b'>') && b[j] > b' ' && b[j] != 0x7f {
                j += 1;
            }
            if b.get(j) == Some(&b'>') {
                return Some((j + 1, false));
            }
        }
    }
    // Email autolink.
    let mut i = 1;
    while i < b.len() && (b[i].is_ascii_alphanumeric() || b".!#$%&'*+/=?^_`{|}~-".contains(&b[i])) {
        i += 1;
    }
    if i == 1 || b.get(i) != Some(&b'@') {
        return None;
    }
    i += 1;
    loop {
        let start = i;
        if !b.get(i).is_some_and(|c| c.is_ascii_alphanumeric()) {
            return None;
        }
        while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'-') && i - start < 63 {
            i += 1;
        }
        if b[i - 1] == b'-' {
            return None;
        }
        match b.get(i) {
            Some(b'.') => i += 1,
            Some(b'>') => return Some((i + 1, true)),
            _ => return None,
        }
    }
}

/// HTML block tag names for start condition 6 (CommonMark 0.31.2).
pub(crate) const HTML_BLOCK_TAGS: &[&str] = &[
    "address",
    "article",
    "aside",
    "base",
    "basefont",
    "blockquote",
    "body",
    "caption",
    "center",
    "col",
    "colgroup",
    "dd",
    "details",
    "dialog",
    "dir",
    "div",
    "dl",
    "dt",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "frame",
    "frameset",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "header",
    "hr",
    "html",
    "iframe",
    "legend",
    "li",
    "link",
    "main",
    "menu",
    "menuitem",
    "nav",
    "noframes",
    "ol",
    "optgroup",
    "option",
    "p",
    "param",
    "search",
    "section",
    "summary",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "title",
    "tr",
    "track",
    "ul",
];

/// Returns the HTML block kind (1–7) that `line` (starting at `<`) opens.
/// Kind 7 cannot interrupt a paragraph, so it is skipped when `tip_is_paragraph`.
pub(crate) fn html_block_start(line: &str, tip_is_paragraph: bool) -> Option<u8> {
    let b = line.as_bytes();
    if b.first() != Some(&b'<') {
        return None;
    }
    let lower = line
        .get(..line.len().min(16))
        .map(str::to_ascii_lowercase)?;
    for tag in ["script", "pre", "style", "textarea"] {
        if let Some(rest) = lower.strip_prefix(&format!("<{tag}")) {
            if rest.is_empty() || matches!(rest.as_bytes()[0], b' ' | b'\t' | b'>') {
                return Some(1);
            }
        }
    }
    if line.starts_with("<!--") {
        return Some(2);
    }
    if line.starts_with("<?") {
        return Some(3);
    }
    if b.len() > 2 && b[1] == b'!' && b[2].is_ascii_alphabetic() {
        return Some(4);
    }
    if line.starts_with("<![CDATA[") {
        return Some(5);
    }
    let mut i = 1;
    if b.get(i) == Some(&b'/') {
        i += 1;
    }
    let name_start = i;
    while i < b.len() && (b[i].is_ascii_alphanumeric()) {
        i += 1;
    }
    let name = line[name_start..i].to_ascii_lowercase();
    if !name.is_empty() && HTML_BLOCK_TAGS.contains(&name.as_str()) {
        let rest = &b[i..];
        if rest.is_empty() || matches!(rest[0], b' ' | b'\t' | b'>') || rest.starts_with(b"/>") {
            return Some(6);
        }
    }
    if tip_is_paragraph {
        return None;
    }
    let len = if b.get(1) == Some(&b'/') {
        scan_closing_tag(line)
    } else {
        scan_open_tag(line)
    }?;
    let tail = &line[len..];
    if !name.is_empty()
        && b.get(1) != Some(&b'/')
        && ["script", "pre", "style", "textarea"].contains(&name.as_str())
    {
        return None;
    }
    tail.bytes().all(is_space_or_tab).then_some(7)
}

/// Whether `line` satisfies the end condition for HTML block `kind` (1–5).
pub(crate) fn html_block_end(kind: u8, line: &str) -> bool {
    match kind {
        1 => {
            let lower = line.to_ascii_lowercase();
            ["</script>", "</pre>", "</style>", "</textarea>"]
                .iter()
                .any(|t| lower.contains(t))
        }
        2 => line.contains("-->"),
        3 => line.contains("?>"),
        4 => line.contains('>'),
        5 => line.contains("]]>"),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entities_decode() {
        assert_eq!(scan_entity("&amp;x"), Some((5, Cow::Borrowed("&"))));
        assert_eq!(scan_entity("&#35;"), Some((5, Cow::Owned("#".into()))));
        assert_eq!(scan_entity("&#x0;").unwrap().1, "\u{fffd}");
        assert_eq!(scan_entity("&nope;"), None);
        assert_eq!(unescape(r"\*a\b &copy;"), r"*a\b ©");
    }

    #[test]
    fn link_parts() {
        assert_eq!(scan_link_label("[foo bar]: x"), Some(9));
        assert_eq!(scan_link_label("[ ]"), None);
        assert_eq!(scan_link_destination("<a b>"), Some((5, "a b")));
        assert_eq!(scan_link_destination("a(b)c d"), Some((5, "a(b)c")));
        assert_eq!(scan_link_title("\"t\" x"), Some((3, "t")));
        assert_eq!(scan_link_title("(a(b)"), None);
    }

    #[test]
    fn tags() {
        assert_eq!(scan_open_tag("<a href=\"x\" disabled>"), Some(21));
        assert_eq!(scan_open_tag("<a/>"), Some(4));
        assert_eq!(scan_open_tag("<a href=x y>"), Some(12));
        assert_eq!(scan_open_tag("<33>"), None);
        assert_eq!(scan_closing_tag("</div >"), Some(7));
        assert_eq!(scan_html_special("<!-- c -->"), Some(10));
        assert_eq!(scan_autolink("<https://x.y/z>", 2), Some((15, false)));
        assert_eq!(scan_autolink("<a@b.co>", 2), Some((8, true)));
        assert_eq!(scan_autolink("<a://b>", 2), None);
        assert_eq!(scan_autolink("<a://b>", 1), Some((7, false)));
        assert_eq!(html_block_start("<div class=x>", true), Some(6));
        assert_eq!(html_block_start("<span>", true), None);
        assert_eq!(html_block_start("<span>", false), Some(7));
    }
}
