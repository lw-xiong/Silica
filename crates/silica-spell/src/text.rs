//! Which bytes of a note are spellchecked, and how they split into words.
//!
//! The checked text comes from the Markdown tree, never from a second
//! Markdown tokenizer. What Obsidian checks was read off its editor on
//! 2026-11-15 (Source mode): its editor marks code, URLs, tags,
//! frontmatter, math, block ids and footnote references as not
//! spellchecked, and leaves everything else to Chromium — headings,
//! emphasis, link text, the inside of a wikilink or an embed (target,
//! heading and alias alike), raw HTML (tag names and attributes included),
//! comments, callout types and titles, list items, quotes, table cells and
//! footnote definitions.
//!
//! The words inside that text follow Chromium's checker: letters, marks and
//! digits make a word, an apostrophe, a period, a middle dot or a colon
//! between two of them joins it, and everything else (spaces, hyphens,
//! underscores, slashes, other punctuation) splits. A word with no letter
//! is never checked; one with digits is (`zorp42` and `v2` were flagged,
//! `42nd` was not, by the dictionary's own rule). See [`words`].

use std::ops::Range;

use silica_markdown::{walk_inlines, Block, BlockKind, Document, Inline, InlineKind, LinkKind};

/// The byte ranges of `doc.source` whose words are checked, sorted, not
/// overlapping, adjacent ones merged.
pub fn checked_ranges(doc: &Document<'_>) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let src = doc.source;
    blocks(src, &doc.blocks, &mut out);
    out.retain(|r| r.start < r.end && r.end <= src.len());
    out.sort_by_key(|r| (r.start, r.end));
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(out.len());
    for r in out {
        match merged.last_mut() {
            Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
            _ => merged.push(r),
        }
    }
    merged
}

/// The offset of `part` in `src` when it is a slice of it.
fn offset_in(src: &str, part: &str) -> Option<usize> {
    let s = src.as_ptr() as usize;
    let p = part.as_ptr() as usize;
    (p >= s && p + part.len() <= s + src.len()).then(|| p - s)
}

fn blocks(src: &str, list: &[Block<'_>], out: &mut Vec<Range<usize>>) {
    for block in list {
        let before = out.len();
        match &block.kind {
            BlockKind::Paragraph { content } | BlockKind::Heading { content, .. } => {
                inlines(src, content, out)
            }
            BlockKind::HtmlBlock { .. } => out.push(block.span.start..block.span.end),
            BlockKind::Comment { content } => match offset_in(src, content) {
                Some(at) => out.push(at..at + content.len()),
                None => out.push(block.span.start..block.span.end),
            },
            BlockKind::BlockQuote { children } => blocks(src, children, out),
            BlockKind::Callout {
                callout_type,
                title,
                children,
                ..
            } => {
                if let Some(at) = offset_in(src, callout_type) {
                    out.push(at..at + callout_type.len());
                }
                blocks(src, title, out);
                blocks(src, children, out);
            }
            BlockKind::List { items, .. } => {
                for item in items {
                    blocks(src, &item.children, out);
                }
            }
            BlockKind::Table { header, rows, .. } => {
                for cell in header.iter().chain(rows.iter().flatten()) {
                    inlines(src, &cell.content, out);
                }
            }
            BlockKind::FootnoteDefinition { children, .. } => blocks(src, children, out),
            BlockKind::Yaml { .. }
            | BlockKind::CodeBlock { .. }
            | BlockKind::Math { .. }
            | BlockKind::ThematicBreak => {}
        }
        // A block id is never checked, whatever text node it sits in.
        if let Some(id) = &block.id {
            let cut = id.span.start..id.span.end;
            let mut i = before;
            while i < out.len() {
                let r = out[i].clone();
                if r.end <= cut.start || r.start >= cut.end {
                    i += 1;
                    continue;
                }
                out.remove(i);
                if r.start < cut.start {
                    out.insert(i, r.start..cut.start);
                    i += 1;
                }
                if r.end > cut.end {
                    out.insert(i, cut.end..r.end);
                    i += 1;
                }
            }
        }
    }
}

fn inlines(src: &str, list: &[Inline<'_>], out: &mut Vec<Range<usize>>) {
    // `walk_inlines` goes into every container; links of the URL kinds and
    // the leaves that are never checked are filtered here.
    let mut skip_until = 0usize;
    walk_inlines(list, &mut |inline| {
        let span = inline.span.start..inline.span.end;
        if span.start < skip_until {
            return;
        }
        match &inline.kind {
            InlineKind::Text(_) => out.push(span),
            // * Raw HTML is checked as it is written: the tag names and
            //   the attributes with them.
            InlineKind::Html(_) | InlineKind::HtmlInline { .. } => out.push(span),
            InlineKind::Comment(_) => {
                // Inside the `%%` pairs.
                let inner = src[span.clone()]
                    .strip_prefix("%%")
                    .map(|s| s.strip_suffix("%%").unwrap_or(s));
                match inner.and_then(|s| offset_in(src, s)) {
                    Some(at) => out.push(at..at + inner.map_or(0, str::len)),
                    None => out.push(span),
                }
            }
            InlineKind::Wikilink { body, .. } => {
                if let Some(at) = offset_in(src, body) {
                    out.push(at..at + body.len());
                } else {
                    let s = &src[span.clone()];
                    let open = s.find("[[").map_or(0, |i| i + 2);
                    let close = s.rfind("]]").unwrap_or(s.len()).max(open);
                    out.push(span.start + open..span.start + close);
                }
            }
            InlineKind::Link { kind, .. } | InlineKind::Image { kind, .. }
                if matches!(kind, LinkKind::Autolink | LinkKind::BareUrl) =>
            {
                skip_until = span.end;
            }
            _ => {}
        }
    });
}

/// Characters that join two word characters into one word.
fn is_joiner(c: char) -> bool {
    matches!(
        c,
        '\'' | '\u{2019}' | '\u{02bc}' | '.' | '\u{00b7}' | ':' | '\u{00ad}'
    )
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || is_mark(c)
}

/// Combining marks (a decomposed accent stays in its word).
fn is_mark(c: char) -> bool {
    matches!(c as u32,
        0x0300..=0x036f | 0x1ab0..=0x1aff | 0x1dc0..=0x1dff | 0x20d0..=0x20ff | 0xfe20..=0xfe2f)
}

/// The words of `text` with their byte offsets, in Chromium's breaking.
pub fn words(text: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    for_each_word(text, |at, w| out.push((at, w)));
    out
}

/// [`words`] without the vector: `f` gets each word and its offset.
pub fn for_each_word<'a>(text: &'a str, mut f: impl FnMut(usize, &'a str)) {
    let mut it = text.char_indices().peekable();
    while let Some((start, c)) = it.next() {
        if !is_word_char(c) {
            continue;
        }
        let mut end = start + c.len_utf8();
        loop {
            match it.peek().copied() {
                Some((i, c)) if is_word_char(c) => {
                    it.next();
                    end = i + c.len_utf8();
                }
                Some((i, c)) if is_joiner(c) => {
                    // A joiner counts only with a word character after it.
                    let after = text[i + c.len_utf8()..].chars().next();
                    if after.is_some_and(is_word_char) {
                        it.next();
                        end = i + c.len_utf8();
                    } else {
                        break;
                    }
                }
                _ => break,
            }
        }
        f(start, &text[start..end]);
    }
}

/// Whether a word is checked at all: it has a letter and is not too long
/// for Hunspell.
pub fn is_checked(word: &str) -> bool {
    word.len() <= 100 && word.chars().any(char::is_alphabetic)
}

/// The parts a joined word splits into at its joiners, each with its
/// offset in the word. Chromium accepts a joined word whose every part is
/// a word (`rock'n'roll`, `foo.bar`, `e.g`), and flags the whole word
/// otherwise (`recieve's`, `Imaage.png`).
pub fn parts(word: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, c) in word.char_indices() {
        if is_joiner(c) {
            if start < i {
                out.push((start, &word[start..i]));
            }
            start = i + c.len_utf8();
        }
    }
    if start < word.len() {
        out.push((start, &word[start..]));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(text: &str) -> Vec<&str> {
        words(text).into_iter().map(|(_, w)| w).collect()
    }

    #[test]
    fn words_break_the_way_the_editor_breaks_them() {
        assert_eq!(
            w("self-awarness snake_case_wrod don't don’t 'quoted' wrold. e.g. Imaage.png zorp·ville a/b"),
            [
                "self", "awarness", "snake", "case", "wrod", "don't", "don’t", "quoted", "wrold",
                "e.g", "Imaage.png", "zorp·ville", "a", "b"
            ]
        );
        assert_eq!(
            w("zorp42 2ndd COVID-19 cafe\u{301}"),
            ["zorp42", "2ndd", "COVID", "19", "cafe\u{301}"]
        );
        assert_eq!(words("  hi there")[1], (5, "there"));
        assert!(!is_checked("19"));
        assert!(is_checked("v2"));
    }

    #[test]
    fn joined_words_split_into_parts() {
        assert_eq!(
            parts("rock'n'roll")
                .into_iter()
                .map(|p| p.1)
                .collect::<Vec<_>>(),
            ["rock", "n", "roll"]
        );
        assert_eq!(parts("foo.bar")[1], (4, "bar"));
    }

    fn checked(src: &str) -> Vec<&str> {
        let doc = silica_markdown::parse(src);
        checked_ranges(&doc).into_iter().map(|r| &src[r]).collect()
    }

    #[test]
    fn code_urls_tags_math_and_frontmatter_are_not_checked() {
        let src = "---\nkey: value\n---\n# Head **bold**\n\nA `code` and #tag and $x$ and https://example.com/path and <https://a.org/b> and someone@example.com and [text](https://u.rl/p) end.\n\n```\nfenced\n```\n\n$$\nmath\n$$\n";
        let got = checked(src).join("|");
        for no in [
            "key", "value", "code", "tag", "x$", "example", "a.org", "u.rl", "fenced", "math",
        ] {
            assert!(!got.contains(no), "{no} is not checked: {got}");
        }
        for yes in ["Head", "bold", "A ", "text", "end."] {
            assert!(got.contains(yes), "{yes} is checked: {got}");
        }
    }

    #[test]
    fn wikilinks_html_comments_callouts_and_footnotes() {
        let src = "Links [[Untill]] and [[Other#Headding|aliass]] and ![[Imaage.png]] and <span title=\"publically\">enviroment</span> and %%embarasment%% here.[^fnoot]\n\n> [!notte] persue title\n> wich body\n\nBlock line ^bloock\n\n[^fnoot]: definitionn\n\n| tabble | b |\n| --- | --- |\n| c | d |\n";
        let got = checked(src).join("|");
        for yes in [
            "Untill",
            "Other#Headding|aliass",
            "Imaage.png",
            "publically",
            "enviroment",
            "embarasment",
            "notte",
            "persue",
            "wich",
            "definitionn",
            "tabble",
            "Block line",
        ] {
            assert!(got.contains(yes), "{yes} is checked: {got}");
        }
        for no in ["fnoot", "bloock", "%%"] {
            assert!(!got.contains(no), "{no} is not checked: {got}");
        }
    }
}
