//! Obsidian Flavored Markdown.
//!
//! This crate owns the *one* parser that every other subsystem must share:
//! editor decorations, reading view, the metadata cache, and link rewriting
//! all read the same tree so that edge cases cannot diverge.
//!
//! [`parse`] turns a note into a [`Document`]: CommonMark block and inline
//! structure, GitHub extensions (tables, strikethrough, task lists,
//! footnotes, bare URLs), and Obsidian's own syntax (YAML properties,
//! `[[wikilinks]]`, `![[embeds]]`, `#tags`, `^block-ids`, callouts,
//! `==highlights==`, `%%comments%%`, `$math$`). Every node carries the byte
//! span it came from. [`render_html`] renders the tree for the reading view
//! and for conformance testing against the CommonMark spec, and
//! [`highlight::highlight_lines`] turns it into per-line editor decorations.

pub mod ast;
mod block;
mod entities;
pub mod frontmatter;
pub mod highlight;
pub mod html;
mod inline;
pub mod linktext;
mod refdef;
mod scan;
pub mod span;

pub use ast::*;
pub use frontmatter::{extract_frontmatter, FrontMatter};
pub use html::{render_html, render_html_with, HtmlOptions};
pub use inline::is_tag_char;
pub use linktext::{link_target, parse_linktext, LinkText, Subpath};
pub use span::{LineIndex, Loc, Pos, Span};

use block::{RawBlock, RawItem, RawKind};
use inline::InlineContext;
use std::collections::HashMap;
use std::ops::Range;

/// Which extensions the parser recognizes. [`Options::obsidian`] is the
/// default; [`Options::commonmark`] turns every extension off for spec tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// `---` properties block on line 1.
    pub frontmatter: bool,
    /// `[[link]]` and `![[embed]]`.
    pub wikilinks: bool,
    /// `#tag`.
    pub tags: bool,
    /// ` ^id` at the end of a block.
    pub block_ids: bool,
    /// `> [!type]` block quotes.
    pub callouts: bool,
    /// `$inline$` and `$$display$$`.
    pub math: bool,
    /// `%%comment%%`.
    pub comments: bool,
    /// `==highlight==`.
    pub highlight: bool,
    /// `~~strikethrough~~`.
    pub strikethrough: bool,
    /// GFM pipe tables.
    pub tables: bool,
    /// `- [ ]` task list items.
    pub task_lists: bool,
    /// `[^label]`, `[^label]: text` and `^[inline]` footnotes.
    pub footnotes: bool,
    /// Bare `https://` and `www.` URLs become links.
    pub bare_urls: bool,
    /// Obsidian's observed departures from CommonMark and GFM: an unclosed
    /// backtick run retries one backtick shorter, `~~~` and `===` runs
    /// delimit with their first two characters, a `#` right after a tag
    /// starts another tag, legacy entities decode without a semicolon, a
    /// list marker indented four or more columns inside a list is a sibling
    /// item, and `[^x]` without a definition renders as `x`.
    pub quirks: bool,
}

impl Options {
    pub const fn obsidian() -> Options {
        Options {
            frontmatter: true,
            wikilinks: true,
            tags: true,
            block_ids: true,
            callouts: true,
            math: true,
            comments: true,
            highlight: true,
            strikethrough: true,
            tables: true,
            task_lists: true,
            footnotes: true,
            bare_urls: true,
            quirks: true,
        }
    }

    pub const fn commonmark() -> Options {
        Options {
            frontmatter: false,
            wikilinks: false,
            tags: false,
            block_ids: false,
            callouts: false,
            math: false,
            comments: false,
            highlight: false,
            strikethrough: false,
            tables: false,
            task_lists: false,
            footnotes: false,
            bare_urls: false,
            quirks: false,
        }
    }
}

impl Default for Options {
    fn default() -> Options {
        Options::obsidian()
    }
}

/// Parses `src` with the default (Obsidian) options.
pub fn parse(src: &str) -> Document<'_> {
    parse_with(src, &Options::obsidian())
}

/// Parses `src` with explicit options.
pub fn parse_with<'a>(src: &'a str, opts: &Options) -> Document<'a> {
    let out = block::BlockParser::parse(src, opts);
    let cx = InlineContext {
        src,
        opts,
        definitions: &out.link_definitions,
        footnote_labels: &out.footnote_labels,
    };
    let blocks = lower_blocks(&cx, out.blocks);
    Document {
        source: src,
        blocks,
        link_definitions: out.link_definitions,
        footnote_labels: out.footnote_labels,
    }
}

/// A re-parsed window of a document: the whole top-level blocks overlapping
/// a byte range, with every span and slice in the coordinates of the source
/// it was parsed from. This is the half-document an edit produces; the
/// caller splices it into the results it already holds.
pub struct PartialDocument<'a> {
    /// The byte range this covers, in the source it was parsed from.
    pub window: Range<usize>,
    pub blocks: Vec<Block<'a>>,
    /// Definitions found inside the window. Definitions seeded from outside
    /// are not repeated here.
    pub link_definitions: Vec<LinkDefinition<'a>>,
    /// Footnote labels found inside the window, in order.
    pub footnote_labels: Vec<&'a str>,
}

/// The whole top-level blocks of `blocks` that overlap `[from, to)`,
/// widened on both sides so no block is cut in half. The cost is a walk
/// over the block list, not a parse — which is what lets an edit ask
/// which region to re-parse in O(blocks).
pub fn snap_window(blocks: &[Block<'_>], from: usize, to: usize) -> Range<usize> {
    let spans: Vec<Span> = blocks.iter().map(|b| b.span).collect();
    snap_window_spans(&spans, from, to)
}

/// [`snap_window`] over bare block spans, for a caller that keeps the
/// boundaries but not the tree they came from: the editor drops its
/// `Document` once the decorations are derived, and a byte span is all a
/// window needs.
pub fn snap_window_spans(spans: &[Span], from: usize, to: usize) -> Range<usize> {
    let start = spans
        .iter()
        .find(|s| s.end > from)
        .map(|s| s.start)
        .unwrap_or(from);
    let end = spans
        .iter()
        .find(|s| s.start >= to)
        .map(|s| s.start)
        .unwrap_or_else(|| spans.last().map_or(to, |s| s.end));
    start..end
}

/// Parses only the lines of `src` whose starts fall in `window`, positioned
/// as a full parse would be at the window's first line.
///
/// `definitions` and `footnote_labels` are the document's, from the parse
/// the edit is applied to: an inline inside the window may point at a
/// definition outside it, and the window has to resolve those. Pass the
/// ones whose spans sit outside `window` — the ones inside are stale in a
/// just-edited document, and the window re-derives its own.
///
/// The window must start and end at top-level block boundaries; compute it
/// with [`snap_window`]. A caller holding no previous parse passes the
/// whole document and calls [`parse`] instead.
pub fn parse_range<'a>(
    src: &'a str,
    window: Range<usize>,
    start_line: usize,
    opts: &Options,
    definitions: &HashMap<String, LinkDefinition<'a>>,
    footnote_labels: &[&'a str],
) -> PartialDocument<'a> {
    let out = block::BlockParser::parse_window(
        src,
        window.clone(),
        start_line,
        opts,
        definitions,
        footnote_labels,
    );
    // * The window's own definitions are the ones whose spans sit inside it:
    //   the seeded map carries the outside ones, and a window-parsed
    //   definition always lies within the window's lines. A label both
    //   inside and outside keeps the earlier one, exactly as a full parse
    //   does. Footnote labels carry no spans, so the window's are the
    //   collected set minus what was seeded.
    let owned: Vec<LinkDefinition<'a>> = out
        .link_definitions
        .iter()
        .filter(|(_, def)| def.span.start >= window.start && def.span.end <= window.end)
        .map(|(_, def)| def.clone())
        .collect();
    let notes: Vec<&'a str> = out
        .footnote_labels
        .iter()
        .filter(|label| !footnote_labels.contains(label))
        .copied()
        .collect();
    let cx = InlineContext {
        src,
        opts,
        definitions: &out.link_definitions,
        footnote_labels: &out.footnote_labels,
    };
    let blocks = lower_blocks(&cx, out.blocks);
    PartialDocument {
        window,
        blocks,
        link_definitions: owned,
        footnote_labels: notes,
    }
}

fn lower_blocks<'a>(cx: &InlineContext<'a, '_>, raw: Vec<RawBlock<'a>>) -> Vec<Block<'a>> {
    raw.into_iter().map(|b| lower_block(cx, b)).collect()
}

fn lower_items<'a>(cx: &InlineContext<'a, '_>, items: Vec<RawItem<'a>>) -> Vec<ListItem<'a>> {
    items
        .into_iter()
        .map(|i| ListItem {
            span: i.span,
            marker: i.marker,
            task: i.task,
            task_span: i.task_span,
            children: lower_blocks(cx, i.children),
        })
        .collect()
}

fn lower_block<'a>(cx: &InlineContext<'a, '_>, raw: RawBlock<'a>) -> Block<'a> {
    let cells = |spans: Vec<Span>| -> Vec<TableCell<'a>> {
        spans
            .into_iter()
            .map(|s| TableCell {
                span: s,
                content: inline::parse_inlines(cx, &[s]),
            })
            .collect()
    };
    let kind = match raw.kind {
        RawKind::Yaml { yaml } => BlockKind::Yaml { yaml },
        RawKind::Paragraph { lines } => BlockKind::Paragraph {
            content: inline::parse_inlines(cx, &lines),
        },
        RawKind::Heading {
            level,
            lines,
            setext,
        } => BlockKind::Heading {
            level,
            content: inline::parse_inlines(cx, &lines),
            setext,
        },
        RawKind::ThematicBreak => BlockKind::ThematicBreak,
        RawKind::CodeBlock {
            fence,
            info,
            content,
        } => BlockKind::CodeBlock {
            fence,
            info,
            content,
        },
        RawKind::HtmlBlock { content, kind } => BlockKind::HtmlBlock { content, kind },
        RawKind::BlockQuote { children } => BlockKind::BlockQuote {
            children: lower_blocks(cx, children),
        },
        RawKind::Callout {
            callout_type,
            fold,
            title,
            children,
        } => BlockKind::Callout {
            callout_type,
            fold,
            title: if title.is_empty() {
                Vec::new()
            } else {
                lower_blocks(cx, block::BlockParser::parse_span(cx.src, title, cx.opts))
            },
            children: lower_blocks(cx, children),
        },
        RawKind::List { kind, tight, items } => BlockKind::List {
            kind,
            tight,
            items: lower_items(cx, items),
        },
        RawKind::Table {
            alignments,
            header,
            rows,
        } => BlockKind::Table {
            alignments,
            header: cells(header),
            rows: rows.into_iter().map(cells).collect(),
        },
        RawKind::Math { content } => BlockKind::Math { content },
        RawKind::Comment { content } => BlockKind::Comment { content },
        RawKind::FootnoteDefinition { label, children } => BlockKind::FootnoteDefinition {
            label,
            children: lower_blocks(cx, children),
        },
    };
    Block {
        kind,
        span: raw.span,
        id: raw.id,
    }
}

/// Callout types shipped with Obsidian and their aliases. Unknown types render as `note`.
pub fn canonical_callout_type(name: &str) -> &'static str {
    match name.to_ascii_lowercase().as_str() {
        "note" => "note",
        "abstract" | "summary" | "tldr" => "abstract",
        "info" => "info",
        "todo" => "todo",
        "tip" | "hint" | "important" => "tip",
        "success" | "check" | "done" => "success",
        "question" | "help" | "faq" => "question",
        "warning" | "caution" | "attention" => "warning",
        "failure" | "fail" | "missing" => "failure",
        "danger" | "error" => "danger",
        "bug" => "bug",
        "example" => "example",
        "quote" | "cite" => "quote",
        _ => "note",
    }
}

/// Parses a callout header such as `[!tip]- Title` (the leading `> ` already stripped).
/// Returns `(type, fold, title)`; fold is `Some(true)` for `-` (collapsed), `Some(false)` for `+`.
///
/// Delegates to the block parser's own scanner rather than repeating the rule:
/// the two used to disagree about `[! bad]`, and a header the editor reads
/// differently from the reading view is the one bug this crate exists to
/// prevent.
pub fn parse_callout_header(line: &str) -> Option<(&str, Option<bool>, &str)> {
    let (ty, fold, title_offset) = crate::block::parse_callout_header_raw(line)?;
    Some((ty, fold, line[title_offset..].trim()))
}

/// The title Obsidian shows for a callout whose header carries none: the type
/// with each `-` turned into a space, lowercased, and its first character
/// upper-cased.
///
/// So `[!note]` shows "Note", `[!bad-type-two]` shows "Bad type two" and
/// `[!NOTE-X]` shows "Note x". `_` is not touched, so `[!bad_type]` shows
/// "Bad_type", and an empty type (which `[! ]` has) shows nothing. Observed
/// against Obsidian 1.14.4 (tools/probe-gen, 2026-10-29).
///
/// Shared so that the reading view and Live Preview cannot drift apart on it,
/// which they had: Live Preview used to leave the tail of the type as written.
pub fn callout_default_title(callout_type: &str) -> String {
    let spaced = callout_type.replace('-', " ");
    let mut chars = spaced.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first
            .to_uppercase()
            .chain(chars.map(|c| c.to_ascii_lowercase()))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callout_header_variants() {
        assert_eq!(
            parse_callout_header("[!tip]- Title"),
            Some(("tip", Some(true), "Title"))
        );
        assert_eq!(
            parse_callout_header("[!warning]+"),
            Some(("warning", Some(false), ""))
        );
        assert_eq!(
            parse_callout_header("[!note] Plain"),
            Some(("note", None, "Plain"))
        );
        // * The type is trimmed, so this is a `bad` callout; it used to be None
        // * here and a `bad` callout in the block parser.
        assert_eq!(parse_callout_header("[! bad]"), Some(("bad", None, "")));
        // * Something must sit between the brackets, but it may be whitespace.
        assert_eq!(parse_callout_header("[! ]"), Some(("", None, "")));
        assert_eq!(parse_callout_header("[!   ] T"), Some(("", None, "T")));
        assert_eq!(parse_callout_header("[!]"), None);
        assert_eq!(parse_callout_header("not a callout"), None);
    }

    #[test]
    fn callout_aliases_fold_to_canonical() {
        assert_eq!(canonical_callout_type("TLDR"), "abstract");
        assert_eq!(canonical_callout_type("cite"), "quote");
        assert_eq!(canonical_callout_type("custom"), "note");
    }

    /// P1: large-note parse regression benchmark. A 50k-line / ~4 MB document
    /// should parse well under the audit's ~222 ms baseline. This is a
    /// regression guard, not a precise measurement — CI hardware varies.
    #[test]
    fn large_document_parse_stays_under_budget() {
        let mut lines = Vec::with_capacity(50_001);
        for i in 0..50_001 {
            lines.push(format!(
                "## Heading {i}\n\nSome paragraph text with **bold** and *italic* and [a link](file.md).\n\n- list item one\n- list item two\n\n```rust\nfn hello_{i}() -> i32 {{\n    {i}\n}}\n```\n\n> A blockquote with `inline code` and ~~strikethrough~~.\n\n---\n\n| Column A | Column B |\n|----------|----------|\n| cell {i}a | cell {i}b |\n\n"
            ));
        }
        let text = lines.join("\n");
        assert!(text.len() > 4_000_000, "document should be multi-MB");

        let started = std::time::Instant::now();
        let doc = parse(&text);
        let elapsed = started.elapsed();

        // The document must parse at all (correctness), and the time is
        // recorded for regression tracking. The audit observed ~222 ms mean
        // on release hardware; debug-mode CI is much slower, so the budget is
        // generous and exists to catch regressions, not to enforce a target.
        let _block_count = doc.blocks.len();
        assert!(
            elapsed.as_millis() < 15000,
            "large parse took {} ms, over the 15000 ms regression budget",
            elapsed.as_millis()
        );
    }

    /// A small deterministic PRNG, so the property test below needs no
    /// dependency: a 64-bit LCG seeded per test.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            self.0 >> 33
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    /// The document the edit tests run over: every construct whose parse
    /// could disagree with a re-parse of a window — fences, lists, quotes,
    /// callouts, tables, html, refdefs and footnotes.
    const CORPUS: &str = "---\ntitle: Test\n---\n\nIntro paragraph with [ref] and a [[wikilink]].\n\n[ref]: /target \"Title\"\n\n## A heading\n\n- one\n- two\n  - nested\n\n1. first\n2. second\n\n> a quote\n> second line\n\n```rust\nfn main() {}\n```\n\n> [!note] Title here\n> body of the callout\n\n| a | b |\n| - | - |\n| 1 | 2 |\n\n<div>\nhtml block\n</div>\n\nA footnote reference[^1] in prose.\n\n[^1]: The footnote body.\n\nFinal paragraph.\n";

    /// One edit, CommonMark semantics: (from, to, replacement), where a
    /// pure insert has from == to.
    fn random_edit(rng: &mut Rng, len: usize) -> (usize, usize, String) {
        let chars: [&str; 12] = ["*", "`", "#", "-", ">", "[", "]", "\n", "|", "!", " ", "1"];
        if rng.below(3) == 0 && len > 4 {
            let from = rng.below(len);
            let span = 1 + rng.below(3);
            let to = (from + span).min(len);
            (from, to, String::new())
        } else {
            let from = rng.below(len);
            let count = 1 + rng.below(6);
            let mut s = String::new();
            for _ in 0..count {
                s.push_str(chars[rng.below(chars.len())]);
            }
            (from, from, s)
        }
    }

    /// The core property: for every edit, re-parsing only the snapped
    /// window produces the same blocks a full parse of the edited text
    /// produces over that same region. This is the contract the editor's
    /// incremental pipeline rests on.
    #[test]
    fn a_window_reparse_equals_a_full_parse() {
        let mut rng = Rng(0x5eed_1234);
        let opts = Options::obsidian();
        let mut text = CORPUS.to_string();
        for case in 0..120 {
            // * Each edit is applied to the already-edited text, so a bug
            //   in one step compounds into the next rather than hiding.
            let (from, to, replacement) = random_edit(&mut rng, text.len());
            text.replace_range(from..to, &replacement);
            // * Snap against a full parse of the edited text — the parse the
            //   editor would already have on screen if it parsed this far.
            let full = parse_with(&text, &opts);
            let window = snap_window(&full.blocks, from, from + replacement.len());
            // * The window's inlines may point at definitions and footnotes
            //   outside it, so those get seeded.
            let outside: HashMap<_, _> = full
                .link_definitions
                .iter()
                .filter(|(_, d)| d.span.end <= window.start || d.span.start >= window.end)
                .map(|(k, d)| (k.clone(), d.clone()))
                .collect();
            let start_line = text[..window.start].matches('\n').count();
            let partial = parse_range(
                &text,
                window.clone(),
                start_line,
                &opts,
                &outside,
                &full.footnote_labels,
            );
            // * The same region of the full parse, structurally compared.
            let wanted: Vec<Block<'_>> = full
                .blocks
                .iter()
                .filter(|b| b.span.start < window.end && b.span.end > window.start)
                .cloned()
                .collect();
            assert_eq!(
                partial.blocks, wanted,
                "case {case}: window {window:?} of edit {from}..{to} disagrees with a full parse"
            );
        }
    }
}
