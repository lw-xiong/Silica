//! What the text before the cursor asks for.
//!
//! Obsidian scans back from the cursor on the cursor's line only, and
//! nothing suppresses it: a `[[` inside a code span, a fenced block, a
//! `%%comment%%` or frontmatter all open the link suggester (see
//! [[Specification#Observed autocompletion behavior, Obsidian 1.14.4]]).
//! That is why this is a scan over one line and not a parse.

use silica_markdown::is_tag_char;
use std::ops::Range;

/// Which suggester the line asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `[[` — a link to a file, heading, block or alias.
    Link,
    /// `![[` — the same, embedded.
    Embed,
    /// `#` at the start of a word.
    Tag,
}

impl Kind {
    pub fn is_link(self) -> bool {
        matches!(self, Kind::Link | Kind::Embed)
    }

    /// Whether an accepted link is written as an embed.
    pub fn is_embed(self) -> bool {
        self == Kind::Embed
    }
}

/// A suggester opened by the text before the cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Triggered<'a> {
    pub kind: Kind,
    /// Everything accepting replaces: the `[[`, `![[` or `#` markers and
    /// the query after them. Byte columns on the cursor's line.
    pub range: Range<usize>,
    /// The query's own columns, for highlighting what matched.
    pub query_range: Range<usize>,
    /// What was typed: the whole link body (`Alpha#First`, spaces and all),
    /// or the tag without its `#`.
    pub query: &'a str,
    /// The rest of the line after the cursor, which decides whether an
    /// accepted link has to write its own closing brackets.
    pub after: &'a str,
}

/// What `line` asks for with the cursor at byte column `col`, if anything.
///
/// The link suggester is tried first because Obsidian's is first in
/// `editorSuggest.suggests` and the manager takes the first that fires, so
/// `[[Alpha #` is a link query and not a tag.
pub fn trigger(line: &str, col: usize) -> Option<Triggered<'_>> {
    let col = col.min(line.len());
    link(line, col).or_else(|| tag(line, col))
}

/// The last `[[` before the cursor with nothing closing it in between.
fn link(line: &str, col: usize) -> Option<Triggered<'_>> {
    let before = &line[..col];
    let open = before.rfind("[[")?;
    let query = &before[open + 2..];
    // A cursor past the end of the link (`[[Alpha]]|`) is not in it. One
    // still inside (`[[al|]]`) is: the brackets only close after the cursor.
    if query.contains(']') || query.contains('[') {
        return None;
    }
    let embed = line[..open].ends_with('!');
    let start = if embed { open - 1 } else { open };
    Some(Triggered {
        kind: if embed { Kind::Embed } else { Kind::Link },
        range: start..col,
        query_range: open + 2..col,
        query,
        after: &line[col..],
    })
}

/// A `#` at the start of the line or after whitespace, with tag characters
/// through the cursor. `word#tag` does not trigger.
fn tag(line: &str, col: usize) -> Option<Triggered<'_>> {
    let before = &line[..col];
    // Back over the tag characters typed so far.
    let mut start = col;
    for (i, c) in before.char_indices().rev() {
        if !is_tag_char(c) {
            break;
        }
        start = i;
    }
    // What they hang off has to be the `#`.
    if !before[..start].ends_with('#') {
        return None;
    }
    let hash = start - 1;
    if !before[..hash]
        .chars()
        .next_back()
        .is_none_or(char::is_whitespace)
    {
        return None;
    }
    Some(Triggered {
        kind: Kind::Tag,
        range: hash..col,
        query_range: hash + 1..col,
        query: &before[hash + 1..],
        after: &line[col..],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `(kind, replaced text, query)` for the cursor at the end of `line`.
    fn at_end(line: &str) -> Option<(Kind, &str, &str)> {
        let t = trigger(line, line.len())?;
        Some((t.kind, &line[t.range.clone()], t.query))
    }

    #[test]
    fn link_takes_everything_after_the_last_open_brackets() {
        assert_eq!(at_end("[["), Some((Kind::Link, "[[", "")));
        assert_eq!(at_end("[[al"), Some((Kind::Link, "[[al", "al")));
        assert_eq!(at_end("see [[al"), Some((Kind::Link, "[[al", "al")));
        // Spaces, folders, `#`, `^` and `|` all stay in the one query.
        assert_eq!(
            at_end("[[beta no"),
            Some((Kind::Link, "[[beta no", "beta no"))
        );
        assert_eq!(at_end("[[Sub/ga"), Some((Kind::Link, "[[Sub/ga", "Sub/ga")));
        assert_eq!(
            at_end("[[Alpha#first"),
            Some((Kind::Link, "[[Alpha#first", "Alpha#first"))
        );
        assert_eq!(
            at_end("[[Alpha#^blk"),
            Some((Kind::Link, "[[Alpha#^blk", "Alpha#^blk"))
        );
        assert_eq!(
            at_end("[[Alpha|disp"),
            Some((Kind::Link, "[[Alpha|disp", "Alpha|disp"))
        );
        assert_eq!(at_end("[[al "), Some((Kind::Link, "[[al ", "al ")));
        // Obsidian takes the last `[[` of a run.
        assert_eq!(at_end("[[[al"), Some((Kind::Link, "[[al", "al")));
    }

    #[test]
    fn link_needs_an_unclosed_pair_before_the_cursor() {
        assert_eq!(at_end("[al"), None);
        assert_eq!(at_end("[[Alpha]]"), None);
        assert_eq!(at_end("[text]("), None);
        assert_eq!(at_end("[text](al"), None);
        // A cursor inside the brackets still triggers.
        let t = trigger("[[al]]", 4).expect("triggers inside");
        assert_eq!((t.kind, t.query), (Kind::Link, "al"));
        assert_eq!(t.range, 0..4);
    }

    #[test]
    fn embeds_swallow_the_bang() {
        assert_eq!(at_end("![["), Some((Kind::Embed, "![[", "")));
        assert_eq!(at_end("![[al"), Some((Kind::Embed, "![[al", "al")));
        assert_eq!(at_end("a ![[pi"), Some((Kind::Embed, "![[pi", "pi")));
    }

    #[test]
    fn nothing_suppresses_a_link_query() {
        assert_eq!(at_end("`[[al"), Some((Kind::Link, "[[al", "al")));
        assert_eq!(at_end("`code` [[al"), Some((Kind::Link, "[[al", "al")));
        assert_eq!(at_end("%%[[al"), Some((Kind::Link, "[[al", "al")));
        assert_eq!(at_end("k: [[al"), Some((Kind::Link, "[[al", "al")));
    }

    #[test]
    fn tags_start_a_word() {
        assert_eq!(at_end("#"), Some((Kind::Tag, "#", "")));
        assert_eq!(at_end("#inl"), Some((Kind::Tag, "#inl", "inl")));
        assert_eq!(at_end("text #inl"), Some((Kind::Tag, "#inl", "inl")));
        assert_eq!(
            at_end("#project/"),
            Some((Kind::Tag, "#project/", "project/"))
        );
        assert_eq!(at_end("tags: #inl"), Some((Kind::Tag, "#inl", "inl")));
        // Not after a letter, and not once the tag has ended.
        assert_eq!(at_end("word#inl"), None);
        assert_eq!(at_end("#inl and"), None);
    }

    #[test]
    fn a_link_query_wins_over_a_tag() {
        assert_eq!(
            at_end("[[Alpha #"),
            Some((Kind::Link, "[[Alpha #", "Alpha #"))
        );
    }

    #[test]
    fn nothing_triggers_on_plain_text() {
        assert_eq!(at_end(""), None);
        assert_eq!(at_end("plain words"), None);
        assert_eq!(at_end("a - b"), None);
    }

    #[test]
    fn the_query_is_what_the_cursor_sees() {
        // The cursor before the query's end cuts it short.
        let t = trigger("[[Alpha", 4).expect("triggers");
        assert_eq!(t.query, "Al");
        assert_eq!(t.range, 0..4);
        assert_eq!(t.query_range, 2..4);
    }

    #[test]
    fn multibyte_text_keeps_byte_columns() {
        let line = "héllo [[ál";
        let t = trigger(line, line.len()).expect("triggers");
        assert_eq!(t.query, "ál");
        assert_eq!(&line[t.range.clone()], "[[ál");
        let line = "#café";
        let t = trigger(line, line.len()).expect("triggers");
        assert_eq!((t.kind, t.query), (Kind::Tag, "café"));
    }
}
