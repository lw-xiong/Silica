//! Writing a block id into the note a block reference points at.
//!
//! Accepting a block row whose block carries no id of its own means writing
//! one into the *target* note. Obsidian does it in exactly two shapes — a
//! block whose text ends a line takes the id inline, a block that spans
//! lines takes it on a line of its own with a blank line either side — and
//! both were observed byte for byte: see
//! [[Specification#Observed block reference behavior, Obsidian 1.14.4]].
//!
//! The planning here is over the target's **raw bytes**, never through the
//! editor's text model, so a file's own line endings and byte order mark
//! survive a write to a note nobody has open.

use silica_cache::SectionKind;
use std::collections::BTreeMap;

/// How a kind takes its id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// ` ^id` at the end of the block's line, after whatever is already
    /// there — a trailing hard break included.
    Inline,
    /// `^id` on a line of its own, a blank line either side.
    OwnLine,
}

impl Placement {
    /// How `kind` takes an id. A list item is inline like a paragraph; a
    /// list *section* is not addressed as a whole, so it has none.
    pub fn of(kind: SectionKind) -> Option<Placement> {
        Some(match kind {
            SectionKind::Paragraph => Placement::Inline,
            SectionKind::Heading
            | SectionKind::Table
            | SectionKind::Code
            | SectionKind::Callout
            | SectionKind::Blockquote
            | SectionKind::Math => Placement::OwnLine,
            // Nothing observed writes an id onto these, so Silica does not
            // offer them: a thematic break, the frontmatter, an HTML block,
            // a footnote definition, a comment, and the list as a whole.
            _ => return None,
        })
    }
}

/// An id written into a note: the bytes to insert and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdInsertion {
    /// The id itself, without its caret.
    pub id: String,
    /// The byte range of the target's text this replaces. It is empty for
    /// an inline id and swallows the newlines after the block for an
    /// own-line one, which is how Obsidian collapses them.
    pub range: std::ops::Range<usize>,
    pub text: String,
}

impl IdInsertion {
    /// `text` with the id written in.
    pub fn applied(&self, text: &str) -> String {
        let mut out = String::with_capacity(text.len() + self.text.len());
        out.push_str(&text[..self.range.start]);
        out.push_str(&self.text);
        out.push_str(&text[self.range.end..]);
        out
    }
}

/// Where the id for a block ending at `end` goes in `text`.
///
/// `end` is the block's last byte as the cache records it. For an inline
/// placement the id goes at the end of that line, so a paragraph's trailing
/// spaces stay where they are and the id follows them. For an own-line
/// placement the newlines after the block collapse to one, so the id is a
/// paragraph of its own rather than another line of the next block.
/// `nl` is the file's own line ending. Obsidian writes LF into a CRLF note
/// and leaves it mixed; Silica writes the note's own ending, as it already
/// does for properties and for every other edit.
pub fn id_insertion(
    text: &str,
    end: usize,
    placement: Placement,
    id: &str,
    nl: &str,
) -> IdInsertion {
    let end = clamp(text, end);
    let at = line_end(text, end);
    match placement {
        Placement::Inline => IdInsertion {
            id: id.to_string(),
            range: at..at,
            text: format!(" ^{id}"),
        },
        Placement::OwnLine => {
            // The newlines after the block collapse to one, so the id is a
            // paragraph of its own rather than a line of the next block.
            let rest = newline_run(text, at);
            let kept = if rest > 0 { nl } else { "" };
            IdInsertion {
                id: id.to_string(),
                range: at..at + rest,
                text: format!("{nl}{nl}^{id}{nl}{kept}"),
            }
        }
    }
}

/// The end of the line `at` is on, trailing whitespace included so an id
/// goes after a hard break — but before the `\r` of a CRLF ending, which is
/// part of the break and not of the text.
fn line_end(text: &str, at: usize) -> usize {
    match text[at..].find('\n') {
        Some(ix) => {
            let nl = at + ix;
            if nl > 0 && text.as_bytes()[nl - 1] == b'\r' {
                nl - 1
            } else {
                nl
            }
        }
        None => text.len(),
    }
}

/// Bytes of unbroken line endings starting at `at`.
fn newline_run(text: &str, at: usize) -> usize {
    text[at..]
        .bytes()
        .take_while(|b| *b == b'\n' || *b == b'\r')
        .count()
}

fn clamp(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while at > 0 && !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// The insertion for a block, or `None` when the id would not read back as
/// one.
///
/// A real vault turned up the case this exists for: a note whose fenced
/// code block is never closed runs to the end of the file, so an own-line
/// id written after it lands *inside* the fence and indexes as nothing. The
/// link would dangle and a file nobody edited would have changed for it, so
/// the row is not offered at all. Verifying costs one parse of the target,
/// which only an accept pays for.
pub fn plan_id(
    text: &str,
    end: usize,
    placement: Placement,
    id: &str,
    nl: &str,
) -> Option<IdInsertion> {
    let insertion = id_insertion(text, end, placement, id, nl);
    let after = insertion.applied(text);
    silica_cache::index_markdown(&after)
        .blocks
        .contains_key(id)
        .then_some(insertion)
}

/// A fresh block id: six lowercase hex characters, the shape Obsidian
/// writes, that no block of the note already uses.
///
/// The randomness is the caller's: the app seeds from the clock, and a test
/// passes a fixed seed so the bytes it asserts are the bytes it gets.
pub fn new_id(seed: u64, taken: &BTreeMap<String, silica_cache::BlockCache>) -> String {
    let mut state = seed;
    for _ in 0..64 {
        state = mix(state);
        let id = format!("{:06x}", state & 0x00ff_ffff);
        if !taken.contains_key(&id) {
            return id;
        }
    }
    format!("{:06x}", mix(seed ^ taken.len() as u64) & 0x00ff_ffff)
}

/// SplitMix64, so an id does not need a dependency to be well spread.
fn mix(state: u64) -> u64 {
    let mut z = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(text: &str, end: usize, placement: Placement) -> String {
        id_insertion(text, end, placement, "abc123", "\n").applied(text)
    }

    /// The offset of the last byte of `needle` in `text`.
    fn ends(text: &str, needle: &str) -> usize {
        text.find(needle).expect("in the text") + needle.len()
    }

    #[test]
    fn an_inline_id_goes_after_everything_on_the_line() {
        let text = "# Breaks\n\nEnds in two spaces.  \n\nEnds in a backslash\\\n";
        // The hard break stays and the id follows it, as Obsidian writes it.
        assert_eq!(
            write(text, ends(text, "Ends in two spaces."), Placement::Inline),
            "# Breaks\n\nEnds in two spaces.   ^abc123\n\nEnds in a backslash\\\n"
        );
        assert_eq!(
            write(text, ends(text, "Ends in a backslash\\"), Placement::Inline),
            "# Breaks\n\nEnds in two spaces.  \n\nEnds in a backslash\\ ^abc123\n"
        );
        // A list item takes it inline too, on its own line even when a
        // nested item follows.
        let list = "# Kind\n\n- parent item\n  - nested item\n- sibling item\n";
        assert_eq!(
            write(list, ends(list, "- parent item"), Placement::Inline),
            "# Kind\n\n- parent item ^abc123\n  - nested item\n- sibling item\n"
        );
    }

    #[test]
    fn an_own_line_id_takes_a_blank_line_either_side() {
        // Every shape of what follows a heading, as observed.
        let cases = [
            (
                "## H at EOF, no trailing newline",
                "## H at EOF, no trailing newline\n\n^abc123\n",
            ),
            (
                "## H at EOF with newline\n",
                "## H at EOF with newline\n\n^abc123\n\n",
            ),
            (
                "## H then blank\n\nbody text\n",
                "## H then blank\n\n^abc123\n\nbody text\n",
            ),
            (
                "## H then body\nbody text\n",
                "## H then body\n\n^abc123\n\nbody text\n",
            ),
            (
                "## H then heading\n### H2\nbody\n",
                "## H then heading\n\n^abc123\n\n### H2\nbody\n",
            ),
        ];
        for (before, want) in cases {
            let end = before.find('\n').unwrap_or(before.len());
            assert_eq!(
                write(before, end, Placement::OwnLine),
                want,
                "for {before:?}"
            );
        }
    }

    #[test]
    fn a_block_that_spans_lines_takes_the_id_after_its_last_line() {
        let table = "# Kind\n\nlead paragraph.\n\n| A | B |\n| --- | --- |\n| 1 | 2 |\n\ntrailing paragraph.\n";
        assert_eq!(
            write(table, ends(table, "| 1 | 2 |"), Placement::OwnLine),
            "# Kind\n\nlead paragraph.\n\n| A | B |\n| --- | --- |\n| 1 | 2 |\n\n^abc123\n\ntrailing paragraph.\n"
        );
        let fence = "# Kind\n\n```js\nlet fenced = 1;\n```\n\ntrailing paragraph.\n";
        assert_eq!(
            write(
                fence,
                ends(fence, "```js\nlet fenced = 1;\n```"),
                Placement::OwnLine
            ),
            "# Kind\n\n```js\nlet fenced = 1;\n```\n\n^abc123\n\ntrailing paragraph.\n"
        );
        let callout = "# Kind\n\n> [!note] A callout title\n> callout body line\n\ntrailing.\n";
        assert_eq!(
            write(
                callout,
                ends(callout, "> callout body line"),
                Placement::OwnLine
            ),
            "# Kind\n\n> [!note] A callout title\n> callout body line\n\n^abc123\n\ntrailing.\n"
        );
    }

    #[test]
    fn the_end_of_a_file_keeps_its_shape() {
        // An inline id leaves a file that ended without a newline ending
        // without one; an own-line id brings one with it.
        let para = "# Eof\n\nlast paragraph, no trailing newline";
        assert_eq!(
            write(para, para.len(), Placement::Inline),
            "# Eof\n\nlast paragraph, no trailing newline ^abc123"
        );
        let table = "# EofTable\n\n| A | B |\n| --- | --- |\n| 1 | 2 |";
        assert_eq!(
            write(table, table.len(), Placement::OwnLine),
            "# EofTable\n\n| A | B |\n| --- | --- |\n| 1 | 2 |\n\n^abc123\n"
        );
    }

    #[test]
    fn only_the_kinds_obsidian_writes_onto_take_an_id() {
        assert_eq!(
            Placement::of(SectionKind::Paragraph),
            Some(Placement::Inline)
        );
        for kind in [
            SectionKind::Heading,
            SectionKind::Table,
            SectionKind::Code,
            SectionKind::Callout,
            SectionKind::Blockquote,
            SectionKind::Math,
        ] {
            assert_eq!(
                Placement::of(kind),
                Some(Placement::OwnLine),
                "for {kind:?}"
            );
        }
        // A list is addressed by its items, and nothing is written onto a
        // thematic break or the frontmatter.
        for kind in [
            SectionKind::List,
            SectionKind::ThematicBreak,
            SectionKind::Yaml,
            SectionKind::Html,
            SectionKind::Comment,
            SectionKind::FootnoteDefinition,
        ] {
            assert_eq!(Placement::of(kind), None, "for {kind:?}");
        }
    }

    #[test]
    fn a_crlf_note_keeps_its_own_endings() {
        // Obsidian writes LF here and leaves the file mixed; Silica writes
        // the note's own ending throughout.
        let text = "# Crlf\r\n\r\npara in a CRLF file.\r\n";
        let end = text.find("\r\n").expect("a first line");
        let ins = id_insertion(text, end, Placement::OwnLine, "abc123", "\r\n");
        assert_eq!(
            ins.applied(text),
            "# Crlf\r\n\r\n^abc123\r\n\r\npara in a CRLF file.\r\n"
        );
        // An inline id goes before the `\r`, not between it and the `\n`.
        let para = text.find("para in a CRLF file.").expect("the paragraph");
        let ins = id_insertion(text, para, Placement::Inline, "abc123", "\r\n");
        assert_eq!(
            ins.applied(text),
            "# Crlf\r\n\r\npara in a CRLF file. ^abc123\r\n"
        );
    }

    #[test]
    fn an_id_is_six_hex_characters_that_the_note_does_not_hold() {
        let empty = BTreeMap::new();
        let id = new_id(1, &empty);
        assert_eq!(id.len(), 6);
        assert!(id
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        // The same seed writes the same id, which is what the tests rely on.
        assert_eq!(new_id(1, &empty), id);
        // An id the note already uses is skipped.
        let mut taken = BTreeMap::new();
        taken.insert(
            id.clone(),
            silica_cache::BlockCache {
                id: id.clone(),
                position: Default::default(),
            },
        );
        assert_ne!(new_id(1, &taken), id);
    }

    #[test]
    fn ids_from_a_run_of_seeds_do_not_repeat() {
        let empty = BTreeMap::new();
        let ids: std::collections::BTreeSet<String> =
            (0..500).map(|seed| new_id(seed, &empty)).collect();
        // 500 seeds into 24 bits: a handful of collisions would be chance,
        // a pile of them would mean the mixing is not doing its job.
        assert!(ids.len() > 495, "only {} distinct ids", ids.len());
    }
}
