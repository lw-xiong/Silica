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
        RawKind::HtmlBlock { content } => BlockKind::HtmlBlock { content },
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
        // The type is trimmed, so this is a `bad` callout; it used to be None
        // here and a `bad` callout in the block parser.
        assert_eq!(parse_callout_header("[! bad]"), Some(("bad", None, "")));
        // Something must sit between the brackets, but it may be whitespace.
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
}
