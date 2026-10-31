//! Syntax highlighting spans for editors, derived from the parsed tree.
//!
//! [`highlight_lines`] returns, for every source line, the byte ranges inside
//! that line that belong to a construct worth decorating: headings, links,
//! tags, code, emphasis, and so on. Spans nest (a tag inside a heading, a
//! link inside bold text); [`flatten`] turns one line's spans into
//! non-overlapping segments with every kind that applies, which is what a
//! text layout with per-run styles needs.

use crate::ast::*;
use crate::span::{LineIndex, Span};
use std::ops::Range;

/// Which part of a heading a span covers. A setext heading gets two:
/// its title lines, and the `===` or `---` under them. Only the parser
/// can tell those apart, since a title may itself be a run of the
/// underline character.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HeadingPart {
    pub level: u8,
    /// The span is the setext underline, which is markup, not title text.
    pub underline: bool,
}

impl HeadingPart {
    pub fn title(level: u8) -> HeadingPart {
        HeadingPart {
            level,
            underline: false,
        }
    }

    pub fn underline(level: u8) -> HeadingPart {
        HeadingPart {
            level,
            underline: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HighlightKind {
    /// Heading text of the given level, ATX markers included, or the
    /// underline of a setext heading.
    Heading(HeadingPart),
    /// `[[link]]`
    Wikilink,
    /// `![[embed]]`
    Embed,
    /// `[text](url)`, `![alt](src)`, reference links.
    Link,
    /// Autolinks and bare URLs.
    Url,
    /// `#tag`
    Tag,
    /// Inline code span.
    Code,
    /// Fenced or indented code block.
    CodeBlock,
    Emphasis,
    Strong,
    Strikethrough,
    /// `==highlight==`
    Highlight,
    /// `%%comment%%` and comment blocks.
    Comment,
    /// `$math$` and math blocks.
    Math,
    /// YAML properties block.
    Frontmatter,
    /// `^block-id`
    BlockId,
    /// The `> [!type] Title` line of a callout.
    Callout,
    /// Bullet or number of a list item.
    ListMarker,
    /// `[ ]`, `[x]` task marker.
    Task,
    ThematicBreak,
    /// Raw HTML, inline or block.
    Html,
    /// Footnote references and inline footnotes.
    Footnote,
    /// One `>` of a block quote or callout line, with the space after it.
    QuoteMarker,
}

/// A highlighted byte range within one line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HighlightSpan {
    pub range: Range<usize>,
    pub kind: HighlightKind,
}

/// Per-line highlight spans, in order of start offset; outer spans come
/// before the spans they contain.
pub fn highlight_lines(doc: &Document<'_>) -> Vec<Vec<HighlightSpan>> {
    let lines = LineIndex::new(doc.source);
    let mut global: Vec<(Span, HighlightKind)> = Vec::new();
    collect_blocks(doc.source, &doc.blocks, &lines, 0, &mut global);
    global.sort_by(|a, b| a.0.start.cmp(&b.0.start).then(b.0.end.cmp(&a.0.end)));

    let mut out: Vec<Vec<HighlightSpan>> = vec![Vec::new(); lines.line_count()];
    for (span, kind) in global {
        if span.is_empty() {
            continue;
        }
        let first = lines.pos(span.start).line as usize;
        let last = lines.pos(span.end.saturating_sub(1).max(span.start)).line as usize;
        for (line, line_spans) in out.iter_mut().enumerate().take(last + 1).skip(first) {
            let ls = lines.line_start(line);
            let le = if line + 1 < lines.line_count() {
                lines.line_start(line + 1) - 1
            } else {
                doc.source.len()
            };
            let start = span.start.max(ls) - ls;
            let end = span.end.min(le).max(ls) - ls;
            if start < end {
                line_spans.push(HighlightSpan {
                    range: start..end,
                    kind,
                });
            }
        }
    }
    out
}

/// Splits one line's nested spans into non-overlapping segments, each with
/// the kinds that apply, outermost first. Gaps between segments are plain text.
pub fn flatten(spans: &[HighlightSpan]) -> Vec<(Range<usize>, Vec<HighlightKind>)> {
    let mut bounds: Vec<usize> = spans
        .iter()
        .flat_map(|s| [s.range.start, s.range.end])
        .collect();
    bounds.sort_unstable();
    bounds.dedup();
    let mut out = Vec::new();
    for w in bounds.windows(2) {
        let (a, b) = (w[0], w[1]);
        let kinds: Vec<HighlightKind> = spans
            .iter()
            .filter(|s| s.range.start <= a && s.range.end >= b)
            .map(|s| s.kind)
            .collect();
        if !kinds.is_empty() {
            out.push((a..b, kinds));
        }
    }
    out
}

fn line_range_at(src: &str, lines: &LineIndex, offset: usize) -> Span {
    let line = lines.pos(offset).line as usize;
    let start = lines.line_start(line);
    let end = if line + 1 < lines.line_count() {
        lines.line_start(line + 1) - 1
    } else {
        src.len()
    };
    Span::new(start, end)
}

/// The `>` (plus one space) that opens quote level `depth` on `line`,
/// skipping the markers of the enclosing quotes.
fn quote_marker(line: &str, depth: usize) -> Option<Range<usize>> {
    let b = line.as_bytes();
    let mut i = 0;
    for level in 0..=depth {
        let mut spaces = 0;
        while i < b.len() && b[i] == b' ' && spaces < 3 {
            i += 1;
            spaces += 1;
        }
        if i >= b.len() || b[i] != b'>' {
            return None;
        }
        let start = i;
        i += 1;
        if i < b.len() && b[i] == b' ' {
            i += 1;
        }
        if level == depth {
            return Some(start..i);
        }
    }
    None
}

/// Marks the quote markers of every line a quote block spans.
fn collect_quote_markers(
    src: &str,
    span: Span,
    lines: &LineIndex,
    depth: usize,
    out: &mut Vec<(Span, HighlightKind)>,
) {
    let first = lines.pos(span.start).line as usize;
    let last = lines.pos(span.end.saturating_sub(1).max(span.start)).line as usize;
    for line in first..=last {
        let range = line_range_at(src, lines, lines.line_start(line));
        if let Some(m) = quote_marker(&src[range.start..range.end], depth) {
            out.push((
                Span::new(range.start + m.start, range.start + m.end),
                HighlightKind::QuoteMarker,
            ));
        }
    }
}

fn collect_blocks(
    src: &str,
    blocks: &[Block<'_>],
    lines: &LineIndex,
    quote_depth: usize,
    out: &mut Vec<(Span, HighlightKind)>,
) {
    for block in blocks {
        if let Some(id) = block.id {
            out.push((id.span, HighlightKind::BlockId));
        }
        match &block.kind {
            BlockKind::Yaml { .. } => out.push((block.span, HighlightKind::Frontmatter)),
            BlockKind::Paragraph { content } => collect_inlines(content, out),
            BlockKind::Heading {
                level,
                content,
                setext,
            } => {
                // A setext heading's span runs through its underline, and
                // the display layer cannot tell that line from a title
                // that is itself a run of `=` or `-`, so split it here.
                let underline = setext
                    .then(|| line_range_at(src, lines, block.span.end.saturating_sub(1)))
                    .filter(|u| u.start > block.span.start);
                let title_end = underline.map_or(block.span.end, |u| u.start);
                out.push((
                    Span::new(block.span.start, title_end),
                    HighlightKind::Heading(HeadingPart::title(*level)),
                ));
                if let Some(u) = underline {
                    out.push((
                        Span::new(u.start, block.span.end),
                        HighlightKind::Heading(HeadingPart::underline(*level)),
                    ));
                }
                collect_inlines(content, out);
            }
            BlockKind::ThematicBreak => out.push((block.span, HighlightKind::ThematicBreak)),
            BlockKind::CodeBlock { .. } => out.push((block.span, HighlightKind::CodeBlock)),
            BlockKind::HtmlBlock { .. } => out.push((block.span, HighlightKind::Html)),
            BlockKind::BlockQuote { children } => {
                collect_quote_markers(src, block.span, lines, quote_depth, out);
                collect_blocks(src, children, lines, quote_depth + 1, out);
            }
            BlockKind::Callout {
                title, children, ..
            } => {
                out.push((
                    line_range_at(src, lines, block.span.start),
                    HighlightKind::Callout,
                ));
                collect_quote_markers(src, block.span, lines, quote_depth, out);
                for run in crate::inline_runs(title) {
                    collect_inlines(run, out);
                }
                collect_blocks(src, children, lines, quote_depth + 1, out);
            }
            BlockKind::List { items, .. } => {
                for item in items {
                    out.push((item.marker, HighlightKind::ListMarker));
                    if let Some(task) = item.task_span {
                        out.push((task, HighlightKind::Task));
                    }
                    collect_blocks(src, &item.children, lines, quote_depth, out);
                }
            }
            BlockKind::Table { header, rows, .. } => {
                for cell in header.iter().chain(rows.iter().flatten()) {
                    collect_inlines(&cell.content, out);
                }
            }
            BlockKind::Math { .. } => out.push((block.span, HighlightKind::Math)),
            BlockKind::Comment { .. } => out.push((block.span, HighlightKind::Comment)),
            BlockKind::FootnoteDefinition { children, .. } => {
                collect_blocks(src, children, lines, quote_depth, out)
            }
        }
    }
}

fn collect_inlines(inlines: &[Inline<'_>], out: &mut Vec<(Span, HighlightKind)>) {
    for inline in inlines {
        let kind = match &inline.kind {
            InlineKind::Text(_) | InlineKind::SoftBreak | InlineKind::HardBreak => None,
            InlineKind::Code(_) => Some(HighlightKind::Code),
            InlineKind::Html(_) => Some(HighlightKind::Html),
            InlineKind::Emphasis(c) => {
                collect_inlines(c, out);
                Some(HighlightKind::Emphasis)
            }
            InlineKind::Strong(c) => {
                collect_inlines(c, out);
                Some(HighlightKind::Strong)
            }
            InlineKind::Strikethrough(c) => {
                collect_inlines(c, out);
                Some(HighlightKind::Strikethrough)
            }
            InlineKind::Highlight { children, .. } => {
                collect_inlines(children, out);
                Some(HighlightKind::Highlight)
            }
            InlineKind::Link { kind, children, .. } => {
                collect_inlines(children, out);
                Some(match kind {
                    LinkKind::Autolink | LinkKind::BareUrl => HighlightKind::Url,
                    _ => HighlightKind::Link,
                })
            }
            InlineKind::Image { children, .. } => {
                collect_inlines(children, out);
                Some(HighlightKind::Link)
            }
            InlineKind::Wikilink { embed: true, .. } => Some(HighlightKind::Embed),
            InlineKind::Wikilink { .. } => Some(HighlightKind::Wikilink),
            InlineKind::Tag(_) => Some(HighlightKind::Tag),
            InlineKind::Math { .. } => Some(HighlightKind::Math),
            InlineKind::Comment(_) => Some(HighlightKind::Comment),
            InlineKind::FootnoteReference(_) => Some(HighlightKind::Footnote),
            InlineKind::InlineFootnote(c) => {
                collect_inlines(c, out);
                Some(HighlightKind::Footnote)
            }
        };
        if let Some(kind) = kind {
            out.push((inline.span, kind));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn quote_markers_are_marked_per_level() {
        let src = "> one\n> > two\n\n> [!note] Title\n> body\n";
        let doc = parse(src);
        let lines = highlight_lines(&doc);
        let markers = |l: usize| -> Vec<Range<usize>> {
            lines[l]
                .iter()
                .filter(|s| s.kind == HighlightKind::QuoteMarker)
                .map(|s| s.range.clone())
                .collect()
        };
        assert_eq!(markers(0), vec![0..2]);
        assert_eq!(markers(1), vec![0..2, 2..4]);
        assert_eq!(markers(2), vec![]);
        assert_eq!(markers(3), vec![0..2]);
        assert_eq!(markers(4), vec![0..2]);
        assert!(lines[3].iter().any(|s| s.kind == HighlightKind::Callout));
    }

    /// The brackets sit 1-4 spaces or a tab after the marker, so the span has
    /// to come from the parser. Deriving it as `marker.end + 1` silently lost
    /// the checkbox for every padding but one space and a tab.
    #[test]
    fn task_spans_follow_the_parser_through_every_padding() {
        for src in [
            "- [x] a\n",
            "-  [x] a\n",
            "-   [x] a\n",
            "-    [x] a\n",
            "-\t[x] a\n",
            "* [X] a\n",
            "1.  [?] a\n",
            "> - [ ] a\n",
        ] {
            let doc = parse(src);
            let lines = highlight_lines(&doc);
            let tasks: Vec<&str> = lines[0]
                .iter()
                .filter(|s| s.kind == HighlightKind::Task)
                .map(|s| &src[s.range.clone()])
                .collect();
            assert_eq!(tasks.len(), 1, "no task span for {src:?}");
            assert_eq!(
                &tasks[0][..1],
                "[",
                "span does not start at the bracket for {src:?}"
            );
            assert_eq!(
                &tasks[0][2..3],
                "]",
                "span does not end at the bracket for {src:?}"
            );
        }
    }

    /// A task that the block parser rejects must not get a span either.
    #[test]
    fn non_tasks_get_no_task_span() {
        for src in ["- [ ]\n", "- [ ]no space\n", "-     [x] five spaces\n"] {
            let doc = parse(src);
            let lines = highlight_lines(&doc);
            assert!(
                !lines[0].iter().any(|s| s.kind == HighlightKind::Task),
                "unexpected task span for {src:?}"
            );
        }
    }

    #[test]
    fn spans_are_per_line_and_nested() {
        let src = "# Title #t\n\n**bold [[Link]]** and `code`\n- [ ] task ^id\n";
        let doc = parse(src);
        let lines = highlight_lines(&doc);
        assert_eq!(lines.len(), 5);
        let kinds = |l: usize| -> Vec<(Range<usize>, HighlightKind)> {
            lines[l].iter().map(|s| (s.range.clone(), s.kind)).collect()
        };
        assert_eq!(
            kinds(0),
            vec![
                (0..10, HighlightKind::Heading(HeadingPart::title(1))),
                (8..10, HighlightKind::Tag)
            ]
        );
        assert_eq!(
            kinds(2),
            vec![
                (0..17, HighlightKind::Strong),
                (7..15, HighlightKind::Wikilink),
                (22..28, HighlightKind::Code),
            ]
        );
        assert_eq!(
            kinds(3),
            vec![
                (0..1, HighlightKind::ListMarker),
                (2..5, HighlightKind::Task),
                (11..14, HighlightKind::BlockId),
            ]
        );
        let flat = flatten(&lines[2]);
        assert_eq!(
            flat,
            vec![
                (0..7, vec![HighlightKind::Strong]),
                (7..15, vec![HighlightKind::Strong, HighlightKind::Wikilink]),
                (15..17, vec![HighlightKind::Strong]),
                (22..28, vec![HighlightKind::Code]),
            ]
        );
    }

    /// The underline of a setext heading is markup, and the display layer
    /// cannot find it by looking at the text: `==` over `==` is an H1
    /// whose title reads exactly like its own underline.
    #[test]
    fn a_setext_underline_is_its_own_span() {
        let heads = |src: &str| -> Vec<Vec<(Range<usize>, HighlightKind)>> {
            highlight_lines(&parse(src))
                .iter()
                .map(|l| {
                    l.iter()
                        .filter(|s| matches!(s.kind, HighlightKind::Heading(_)))
                        .map(|s| (s.range.clone(), s.kind))
                        .collect()
                })
                .collect()
        };
        let h = heads("Setext title\n============\n");
        assert_eq!(
            h[0],
            vec![(0..12, HighlightKind::Heading(HeadingPart::title(1)))]
        );
        assert_eq!(
            h[1],
            vec![(0..12, HighlightKind::Heading(HeadingPart::underline(1)))]
        );
        // The title that looks like an underline: only the second row is one.
        let h = heads("==\n==\n");
        assert_eq!(
            h[0],
            vec![(0..2, HighlightKind::Heading(HeadingPart::title(1)))]
        );
        assert_eq!(
            h[1],
            vec![(0..2, HighlightKind::Heading(HeadingPart::underline(1)))]
        );
        // Inside a quote the title span starts after the marker, and the
        // underline covers the whole of its line.
        let h = heads("> Title\n> ===\n");
        assert_eq!(
            h[0],
            vec![(2..7, HighlightKind::Heading(HeadingPart::title(1)))]
        );
        assert_eq!(
            h[1],
            vec![(0..5, HighlightKind::Heading(HeadingPart::underline(1)))]
        );
        // An ATX heading is one span, underline false, markers included.
        let h = heads("## Title ##\n");
        assert_eq!(
            h[0],
            vec![(0..11, HighlightKind::Heading(HeadingPart::title(2)))]
        );
    }

    #[test]
    fn multi_line_constructs_split_per_line() {
        let src = "```rs\nlet x;\n```\n*a\nb*\n";
        let lines = highlight_lines(&parse(src));
        assert!(lines[0..3]
            .iter()
            .all(|l| l.iter().any(|s| s.kind == HighlightKind::CodeBlock)));
        assert_eq!(lines[3][0].range, 0..2);
        assert_eq!(lines[4][0].range, 0..2);
        assert_eq!(lines[4][0].kind, HighlightKind::Emphasis);
    }
}
