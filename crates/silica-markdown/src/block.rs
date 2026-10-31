//! Block structure, the first parsing phase.
//!
//! The algorithm is CommonMark's: each line is matched against the stack of
//! open blocks (continuation), then checked for block starts, and what
//! remains is paragraph text or a lazy continuation line. Obsidian's block
//! constructs (callouts, `$$` math, `%%` comments, block ids) and GFM tables
//! and footnote definitions are additional starts in the same loop.
//!
//! The result is a [`RawBlock`] tree whose paragraph, heading and cell
//! contents are still source spans; the inline phase lowers it to [`crate::Block`].

use crate::ast::{normalize_label, Alignment, BlockId, Fence, Fold, LinkDefinition, ListKind};
use crate::scan::{self, is_space_or_tab};
use crate::span::Span;
use crate::Options;
use std::borrow::Cow;
use std::collections::HashMap;

/// A block whose inline content has not been parsed yet.
#[derive(Debug)]
pub(crate) struct RawBlock<'a> {
    pub kind: RawKind<'a>,
    pub span: Span,
    pub id: Option<BlockId<'a>>,
}

#[derive(Debug)]
pub(crate) struct RawItem<'a> {
    pub span: Span,
    pub marker: Span,
    pub task: Option<char>,
    pub task_span: Option<Span>,
    pub children: Vec<RawBlock<'a>>,
}

#[derive(Debug)]
pub(crate) enum RawKind<'a> {
    Yaml {
        yaml: &'a str,
    },
    Paragraph {
        lines: Vec<Span>,
    },
    Heading {
        level: u8,
        lines: Vec<Span>,
        setext: bool,
    },
    ThematicBreak,
    CodeBlock {
        fence: Option<Fence>,
        info: Option<Cow<'a, str>>,
        content: Cow<'a, str>,
    },
    HtmlBlock {
        content: Cow<'a, str>,
    },
    BlockQuote {
        children: Vec<RawBlock<'a>>,
    },
    Callout {
        callout_type: &'a str,
        fold: Option<Fold>,
        title: Span,
        children: Vec<RawBlock<'a>>,
    },
    List {
        kind: ListKind,
        tight: bool,
        items: Vec<RawItem<'a>>,
    },
    Table {
        alignments: Vec<Alignment>,
        header: Vec<Span>,
        rows: Vec<Vec<Span>>,
    },
    Math {
        content: Cow<'a, str>,
    },
    Comment {
        content: &'a str,
    },
    FootnoteDefinition {
        label: &'a str,
        children: Vec<RawBlock<'a>>,
    },
}

pub(crate) struct BlockOutput<'a> {
    pub blocks: Vec<RawBlock<'a>>,
    pub link_definitions: HashMap<String, LinkDefinition<'a>>,
    pub footnote_labels: Vec<&'a str>,
}

/// A line of leaf content: the source span after container prefixes plus the
/// number of spaces a partially consumed tab contributes before it.
#[derive(Debug, Clone, Copy)]
struct RawLine {
    span: Span,
    pad: u8,
}

/// A callout's `> [!type]+ Title` line, taken off the quote's first line while
/// parsing.
///
/// Keeping it here rather than leaving it as the first line of the body
/// paragraph is what lets a table or a setext underline on the *next* line be
/// seen at all: both of those rules ask how many lines the open paragraph has,
/// and a header still sitting in it makes the answer one too many. That is why
/// `> [!note]` followed by a table row used to render as literal pipes.
#[derive(Debug)]
struct CalloutHeader<'a> {
    callout_type: &'a str,
    fold: Option<Fold>,
    title: Span,
}

#[derive(Debug)]
enum NodeKind<'a> {
    Document,
    BlockQuote {
        /// Set when this quote's first line was a callout header.
        header: Option<CalloutHeader<'a>>,
    },
    List {
        kind: ListKind,
        items: Vec<RawItem<'a>>,
        /// Per item: ends with a blank line, and the same for each of its children.
        items_meta: Vec<(bool, Vec<bool>)>,
    },
    Item {
        marker: Span,
        content_indent: usize,
        task: Option<char>,
        task_span: Option<Span>,
    },
    FootnoteDefinition {
        label: &'a str,
    },
    Paragraph {
        lines: Vec<Span>,
    },
    Heading {
        level: u8,
        lines: Vec<Span>,
        setext: bool,
    },
    ThematicBreak,
    FencedCode {
        fence: Fence,
        info: Option<Cow<'a, str>>,
        lines: Vec<RawLine>,
    },
    IndentedCode {
        lines: Vec<RawLine>,
    },
    HtmlBlock {
        kind: u8,
        lines: Vec<RawLine>,
    },
    Table {
        alignments: Vec<Alignment>,
        header: Vec<Span>,
        rows: Vec<Vec<Span>>,
    },
    Math {
        lines: Vec<RawLine>,
    },
    Comment {
        lines: Vec<Span>,
    },
}

#[derive(Debug)]
struct Node<'a> {
    kind: NodeKind<'a>,
    start: usize,
    end: usize,
    start_line: usize,
    children: Vec<RawBlock<'a>>,
    children_end_blank: Vec<bool>,
    last_line_blank: bool,
}

impl<'a> Node<'a> {
    fn new(kind: NodeKind<'a>, start: usize, start_line: usize) -> Node<'a> {
        Node {
            kind,
            start,
            end: start,
            start_line,
            children: Vec::new(),
            children_end_blank: Vec::new(),
            last_line_blank: false,
        }
    }

    fn is_paragraph(&self) -> bool {
        matches!(self.kind, NodeKind::Paragraph { .. })
    }

    fn accepts_lines(&self) -> bool {
        matches!(
            self.kind,
            NodeKind::Paragraph { .. }
                | NodeKind::FencedCode { .. }
                | NodeKind::IndentedCode { .. }
                | NodeKind::HtmlBlock { .. }
                | NodeKind::Table { .. }
                | NodeKind::Math { .. }
                | NodeKind::Comment { .. }
        )
    }

    /// Leaves whose content cannot be interrupted by a block start.
    fn is_verbatim_leaf(&self) -> bool {
        matches!(
            self.kind,
            NodeKind::FencedCode { .. }
                | NodeKind::IndentedCode { .. }
                | NodeKind::HtmlBlock { .. }
                | NodeKind::Math { .. }
                | NodeKind::Comment { .. }
        )
    }

    fn can_contain(&self, child_is_item: bool) -> bool {
        match self.kind {
            NodeKind::Document
            | NodeKind::BlockQuote { .. }
            | NodeKind::Item { .. }
            | NodeKind::FootnoteDefinition { .. } => !child_is_item,
            NodeKind::List { .. } => child_is_item,
            _ => false,
        }
    }

    fn has_children(&self) -> bool {
        match &self.kind {
            NodeKind::List { items, .. } => !items.is_empty(),
            _ => !self.children.is_empty(),
        }
    }

    fn ends_with_blank_line(&self) -> bool {
        if self.last_line_blank {
            return true;
        }
        match &self.kind {
            NodeKind::List { items_meta, .. } => items_meta.last().is_some_and(|m| m.0),
            NodeKind::Item { .. } => self.children_end_blank.last().copied().unwrap_or(false),
            _ => false,
        }
    }

    fn mark_last_child_blank(&mut self) {
        match &mut self.kind {
            NodeKind::List { items_meta, .. } => {
                if let Some(m) = items_meta.last_mut() {
                    m.0 = true;
                }
            }
            _ => {
                if let Some(b) = self.children_end_blank.last_mut() {
                    *b = true;
                }
            }
        }
    }
}

enum Continue {
    Matched,
    NotMatched,
    /// The line closed the block (closing fence) and is fully consumed.
    Finished,
}

enum Start {
    Container,
    Leaf,
}

pub(crate) struct BlockParser<'a, 'o> {
    src: &'a str,
    opts: &'o Options,
    open: Vec<Node<'a>>,
    line: Span,
    line_number: usize,
    offset: usize,
    column: usize,
    next_nonspace: usize,
    next_nonspace_column: usize,
    indent: usize,
    indented: bool,
    blank: bool,
    partially_consumed_tab: bool,
    all_closed: bool,
    last_matched: usize,
    /// The current line was fully consumed by a block start and must not be added as content.
    line_consumed: bool,
    link_definitions: HashMap<String, LinkDefinition<'a>>,
    footnote_labels: Vec<&'a str>,
}

impl<'a, 'o> BlockParser<'a, 'o> {
    /// Parses one span of `src` as blocks, with every span in the result still
    /// an offset into `src` rather than into the slice.
    ///
    /// A callout's title is block-parsed by Obsidian -- `[!note] # h` really
    /// does make a heading inside `.callout-title-inner` -- and that title is
    /// a contiguous span of the header line, so it goes through this parser
    /// rather than a reduced copy of it. Feeding the line span directly is what
    /// keeps the offsets absolute: parsing `&src[span]` as its own document
    /// would renumber every position in it.
    pub(crate) fn parse_span(src: &'a str, span: Span, opts: &'o Options) -> Vec<RawBlock<'a>> {
        let mut p = BlockParser::new(src, opts);
        p.incorporate_line(span);
        while p.open.len() > 1 {
            p.finalize();
        }
        p.open.pop().unwrap().children
    }

    pub(crate) fn parse(src: &'a str, opts: &'o Options) -> BlockOutput<'a> {
        let mut p = BlockParser::new(src, opts);
        let mut body_start = 0;
        if opts.frontmatter {
            if let Some(fm) = crate::extract_frontmatter(src) {
                let text_end = src[..fm.end].trim_end_matches(['\n', '\r']).len();
                p.open[0].children.push(RawBlock {
                    kind: RawKind::Yaml { yaml: fm.yaml },
                    span: Span::new(fm.start, text_end),
                    id: None,
                });
                p.open[0].children_end_blank.push(false);
                p.line_number = src[..fm.end].matches('\n').count();
                body_start = fm.end;
            }
        }
        let mut pos = body_start;
        while pos < src.len() {
            let rest = &src[pos..];
            let (end, next) = match rest.find('\n') {
                Some(i) => {
                    let end = if i > 0 && rest.as_bytes()[i - 1] == b'\r' {
                        pos + i - 1
                    } else {
                        pos + i
                    };
                    (end, pos + i + 1)
                }
                None => {
                    let end = pos + rest.trim_end_matches('\r').len();
                    (end, src.len())
                }
            };
            p.incorporate_line(Span::new(pos, end));
            p.line_number += 1;
            pos = next;
        }
        while p.open.len() > 1 {
            p.finalize();
        }
        let doc = p.open.pop().unwrap();
        BlockOutput {
            blocks: doc.children,
            link_definitions: p.link_definitions,
            footnote_labels: p.footnote_labels,
        }
    }

    fn new(src: &'a str, opts: &'o Options) -> BlockParser<'a, 'o> {
        BlockParser {
            src,
            opts,
            open: vec![Node::new(NodeKind::Document, 0, 0)],
            line: Span::default(),
            line_number: 0,
            offset: 0,
            column: 0,
            next_nonspace: 0,
            next_nonspace_column: 0,
            indent: 0,
            indented: false,
            blank: false,
            partially_consumed_tab: false,
            all_closed: true,
            last_matched: 0,
            line_consumed: false,
            link_definitions: HashMap::new(),
            footnote_labels: Vec::new(),
        }
    }

    // ----- line cursor -------------------------------------------------------

    fn byte(&self, i: usize) -> Option<u8> {
        if i < self.line.end {
            Some(self.src.as_bytes()[i])
        } else {
            None
        }
    }

    fn rest(&self) -> &'a str {
        &self.src[self.next_nonspace..self.line.end]
    }

    fn find_next_nonspace(&mut self) {
        let bytes = self.src.as_bytes();
        let mut i = self.offset;
        let mut cols = self.column;
        while i < self.line.end {
            match bytes[i] {
                b' ' => {
                    i += 1;
                    cols += 1;
                }
                b'\t' => {
                    i += 1;
                    cols += 4 - (cols % 4);
                }
                _ => break,
            }
        }
        self.blank = i >= self.line.end;
        self.next_nonspace = i;
        self.next_nonspace_column = cols;
        self.indent = cols - self.column;
        self.indented = self.indent >= 4;
    }

    fn advance_next_nonspace(&mut self) {
        self.offset = self.next_nonspace;
        self.column = self.next_nonspace_column;
        self.partially_consumed_tab = false;
    }

    fn advance_offset(&mut self, count: usize, columns: bool) {
        let bytes = self.src.as_bytes();
        let mut count = count;
        while count > 0 && self.offset < self.line.end {
            if bytes[self.offset] == b'\t' {
                let chars_to_tab = 4 - (self.column % 4);
                if columns {
                    self.partially_consumed_tab = chars_to_tab > count;
                    let advance = chars_to_tab.min(count);
                    self.column += advance;
                    if !self.partially_consumed_tab {
                        self.offset += 1;
                    }
                    count -= advance;
                } else {
                    self.partially_consumed_tab = false;
                    self.column += chars_to_tab;
                    self.offset += 1;
                    count -= 1;
                }
            } else {
                self.partially_consumed_tab = false;
                self.offset += 1;
                self.column += 1;
                count -= 1;
            }
        }
    }

    /// The remainder of the line as a leaf content line.
    fn current_raw_line(&mut self) -> RawLine {
        let mut pad = 0;
        if self.partially_consumed_tab {
            pad = (4 - (self.column % 4)) as u8;
            self.offset += 1;
            self.partially_consumed_tab = false;
        }
        RawLine {
            span: Span::new(self.offset, self.line.end),
            pad,
        }
    }

    // ----- stack -------------------------------------------------------------

    fn tip(&self) -> &Node<'a> {
        self.open.last().unwrap()
    }

    fn add_child(&mut self, kind: NodeKind<'a>, start: usize) {
        let is_item = matches!(kind, NodeKind::Item { .. });
        while !self.tip().can_contain(is_item) {
            self.finalize();
        }
        let node = Node::new(kind, start, self.line_number);
        self.open.push(node);
    }

    fn close_unmatched_blocks(&mut self) {
        if !self.all_closed {
            while self.open.len() - 1 > self.last_matched {
                self.finalize();
            }
            self.all_closed = true;
        }
    }

    /// Pops the tip, converts it to a block, and attaches it to its parent.
    fn finalize(&mut self) {
        let node = self.open.pop().unwrap();
        let ends_blank = node.ends_with_blank_line();
        if matches!(node.kind, NodeKind::Item { .. }) {
            let NodeKind::Item {
                marker,
                task,
                task_span,
                ..
            } = node.kind
            else {
                unreachable!()
            };
            let item = RawItem {
                span: Span::new(node.start, node.end),
                marker,
                task,
                task_span,
                children: node.children,
            };
            if let NodeKind::List {
                items, items_meta, ..
            } = &mut self.open.last_mut().unwrap().kind
            {
                items.push(item);
                items_meta.push((ends_blank, node.children_end_blank));
            }
            return;
        }
        if let Some(block) = self.finalize_block(node) {
            let parent = self.open.last_mut().unwrap();
            parent.children.push(block);
            parent.children_end_blank.push(ends_blank);
        }
    }

    fn finalize_block(&mut self, node: Node<'a>) -> Option<RawBlock<'a>> {
        let mut span = Span::new(node.start, node.end);
        let mut id = None;
        let kind = match node.kind {
            NodeKind::Document | NodeKind::Item { .. } => unreachable!(),
            NodeKind::Paragraph { mut lines } => {
                while let Some(n) = self.extract_reference_definition(&lines) {
                    lines.drain(..n);
                }
                if lines.is_empty() {
                    return None;
                }
                if self.opts.block_ids {
                    if let Some((bid, trailing_only)) = self.take_block_id(&mut lines) {
                        if trailing_only && lines.is_empty() {
                            // A `^id` on its own attaches to the preceding block.
                            let parent = self.open.last_mut().unwrap();
                            if let Some(prev) = parent.children.last_mut() {
                                prev.id = Some(bid);
                                return None;
                            }
                            lines.push(bid.span);
                        } else {
                            id = Some(bid);
                        }
                    }
                }
                span.start = lines[0].start;
                RawKind::Paragraph { lines }
            }
            NodeKind::Heading {
                level,
                lines,
                setext,
            } => RawKind::Heading {
                level,
                lines,
                setext,
            },
            NodeKind::ThematicBreak => RawKind::ThematicBreak,
            NodeKind::FencedCode { fence, info, lines } => RawKind::CodeBlock {
                fence: Some(fence),
                info,
                content: self.join_raw_lines(&lines),
            },
            NodeKind::IndentedCode { mut lines } => {
                while lines
                    .last()
                    .is_some_and(|l| l.span.of(self.src).bytes().all(is_space_or_tab))
                {
                    lines.pop();
                }
                if let Some(last) = lines.last() {
                    span.end = last.span.end;
                }
                RawKind::CodeBlock {
                    fence: None,
                    info: None,
                    content: self.join_raw_lines(&lines),
                }
            }
            NodeKind::HtmlBlock { lines, .. } => RawKind::HtmlBlock {
                content: self.join_raw_lines(&lines),
            },
            NodeKind::BlockQuote { header } => {
                let children = node.children;
                match header {
                    None => RawKind::BlockQuote { children },
                    Some(header) => {
                        let mut title = header.title;
                        if self.opts.block_ids {
                            let mut title_lines = vec![title];
                            if let Some((bid, whole)) = self.take_block_id(&mut title_lines) {
                                id = Some(bid);
                                title = if whole {
                                    Span::new(title.start, title.start)
                                } else {
                                    title_lines[0]
                                };
                            }
                        }
                        RawKind::Callout {
                            callout_type: header.callout_type,
                            fold: header.fold,
                            title,
                            children,
                        }
                    }
                }
            }
            NodeKind::List {
                kind,
                items,
                items_meta,
            } => {
                let n = items.len();
                let mut tight = true;
                'outer: for (i, (item_blank, child_flags)) in items_meta.iter().enumerate() {
                    if *item_blank && i + 1 < n {
                        tight = false;
                        break;
                    }
                    let m = child_flags.len();
                    for (j, blank) in child_flags.iter().enumerate() {
                        if *blank && (i + 1 < n || j + 1 < m) {
                            tight = false;
                            break 'outer;
                        }
                    }
                }
                if let Some(last) = items.last() {
                    span.end = last.span.end;
                }
                RawKind::List { kind, tight, items }
            }
            NodeKind::Table {
                alignments,
                header,
                rows,
            } => RawKind::Table {
                alignments,
                header,
                rows,
            },
            NodeKind::Math { lines } => RawKind::Math {
                content: self.join_raw_lines(&lines),
            },
            NodeKind::Comment { lines } => {
                let content = match (lines.first(), lines.last()) {
                    (Some(f), Some(l)) => &self.src[f.start..l.end],
                    _ => "",
                };
                RawKind::Comment { content }
            }
            NodeKind::FootnoteDefinition { label } => RawKind::FootnoteDefinition {
                label,
                children: node.children,
            },
        };
        Some(RawBlock { kind, span, id })
    }

    fn join_raw_lines(&self, lines: &[RawLine]) -> Cow<'a, str> {
        if lines.is_empty() {
            return Cow::Borrowed("");
        }
        let contiguous = lines.iter().all(|l| l.pad == 0)
            && lines.windows(2).all(|w| {
                let between = &self.src[w[0].span.end..w[1].span.start];
                between == "\n"
            });
        let last = lines.last().unwrap();
        if contiguous && self.src[last.span.end..].starts_with('\n') {
            return Cow::Borrowed(&self.src[lines[0].span.start..last.span.end + 1]);
        }
        let mut out = String::new();
        for l in lines {
            for _ in 0..l.pad {
                out.push(' ');
            }
            out.push_str(l.span.of(self.src));
            out.push('\n');
        }
        Cow::Owned(out)
    }

    /// Removes a trailing ` ^id` from the last paragraph line. Returns the id
    /// and whether the id was the whole last line.
    fn take_block_id(&self, lines: &mut Vec<Span>) -> Option<(BlockId<'a>, bool)> {
        let last = *lines.last()?;
        let text = last.of(self.src).trim_end_matches(is_space_or_tab_char);
        let caret = text.rfind('^')?;
        let id = &text[caret + 1..];
        if id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            return None;
        }
        let before = &text[..caret];
        let whole_line = before.trim_end_matches(is_space_or_tab_char).is_empty();
        if !whole_line && !before.ends_with(is_space_or_tab_char) {
            return None;
        }
        let id_span = Span::new(last.start + caret, last.start + text.len());
        let bid = BlockId { id, span: id_span };
        if whole_line {
            lines.pop();
        } else {
            let new_end = last.start + before.trim_end_matches(is_space_or_tab_char).len();
            *lines.last_mut().unwrap() = Span::new(last.start, new_end);
        }
        Some((bid, whole_line))
    }

    /// Converts a blockquote's children into a callout if the first line is a
    /// header; a ` ^id` at the end of the header becomes the callout's block id.
    /// Parses one link reference definition from the start of the paragraph
    /// lines; returns how many lines it consumed.
    fn extract_reference_definition(&mut self, lines: &[Span]) -> Option<usize> {
        let first = *lines.first()?;
        if !first.of(self.src).trim_start().starts_with('[') {
            return None;
        }
        let text = self.join_spans(lines);
        let (label, dest, title, consumed) = crate::refdef::parse_reference_definition(&text)?;
        let consumed_text = &text[..consumed];
        let n_lines =
            consumed_text.matches('\n').count() + usize::from(!consumed_text.ends_with('\n'));
        let n_lines = n_lines.clamp(1, lines.len());
        let key = normalize_label(label);
        if let std::collections::hash_map::Entry::Vacant(e) = self.link_definitions.entry(key) {
            e.insert(LinkDefinition {
                label: Cow::Owned(label.to_string()),
                destination: Cow::Owned(scan::unescape(dest).into_owned()),
                title: title.map(|t| Cow::Owned(scan::unescape(t).into_owned())),
                span: Span::new(first.start, lines[n_lines - 1].end),
            });
        }
        Some(n_lines)
    }

    fn join_spans(&self, lines: &[Span]) -> String {
        let mut out = String::new();
        for (i, l) in lines.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            out.push_str(l.of(self.src));
        }
        out
    }

    // ----- per-line driver ---------------------------------------------------

    fn incorporate_line(&mut self, line: Span) {
        self.line = line;
        self.offset = line.start;
        self.column = 0;
        self.blank = false;
        self.partially_consumed_tab = false;
        self.line_consumed = false;

        let mut container = 0;
        for i in 1..self.open.len() {
            self.find_next_nonspace();
            match self.try_continue(i) {
                Continue::Matched => {
                    container = i;
                }
                Continue::NotMatched => break,
                Continue::Finished => return,
            }
        }
        self.all_closed = container == self.open.len() - 1;
        self.last_matched = container;

        let mut matched_leaf = self.open[container].is_verbatim_leaf();

        while !matched_leaf {
            self.find_next_nonspace();
            if !self.indented
                && !self
                    .byte(self.next_nonspace)
                    .is_some_and(is_block_start_char)
            {
                self.advance_next_nonspace();
                break;
            }
            match self.try_block_starts(container) {
                None => {
                    self.advance_next_nonspace();
                    break;
                }
                Some(Start::Container) => {
                    container = self.open.len() - 1;
                }
                Some(Start::Leaf) => {
                    container = self.open.len() - 1;
                    matched_leaf = true;
                }
            }
        }

        if !self.all_closed && !self.blank && self.tip().is_paragraph() {
            // Lazy paragraph continuation.
            self.add_line();
            self.extend_open_spans(self.open.len());
            return;
        }

        // The same laziness one step earlier. A callout whose header line has
        // just been taken has no open paragraph to continue, but Obsidian still
        // draws an unprefixed next line into the body, so open the paragraph
        // the header used to stand in for.
        if !self.all_closed
            && !self.blank
            && matches!(self.tip().kind, NodeKind::BlockQuote { header: Some(_) })
            && !self.tip().has_children()
        {
            self.add_child(NodeKind::Paragraph { lines: Vec::new() }, self.offset);
            self.advance_next_nonspace();
            self.add_line();
            self.extend_open_spans(self.open.len());
            return;
        }

        self.close_unmatched_blocks();
        let blank = self.blank;
        let tip_idx = self.open.len() - 1;
        if blank {
            self.open[tip_idx].mark_last_child_blank();
        }
        let tip = &self.open[tip_idx];
        let last_line_blank = blank
            && !(matches!(tip.kind, NodeKind::BlockQuote { .. })
                || matches!(tip.kind, NodeKind::FencedCode { .. })
                || (matches!(tip.kind, NodeKind::Item { .. })
                    && !tip.has_children()
                    && tip.start_line == self.line_number));
        for node in self.open.iter_mut() {
            node.last_line_blank = last_line_blank;
        }

        if self.line_consumed {
            // Opening fence, table delimiter row, or similar: nothing left for the block.
        } else if self.tip().accepts_lines() {
            self.add_line();
            if let NodeKind::HtmlBlock { kind, .. } = self.tip().kind {
                if (1..=5).contains(&kind)
                    && scan::html_block_end(kind, &self.src[self.offset..self.line.end])
                {
                    self.extend_open_spans(self.open.len());
                    self.finalize();
                    return;
                }
            }
        } else if self.offset < self.line.end && !blank {
            self.add_child(NodeKind::Paragraph { lines: Vec::new() }, self.offset);
            self.advance_next_nonspace();
            self.add_line();
        }
        if !blank {
            self.extend_open_spans(self.open.len());
        }
    }

    /// Extends the end of every open node below `upto` to the current line.
    fn extend_open_spans(&mut self, upto: usize) {
        let end = self.line.end;
        for node in self.open[..upto].iter_mut() {
            node.end = end;
        }
    }

    fn add_line(&mut self) {
        let raw = self.current_raw_line();
        let end = self.line.end;
        let tip = self.open.last_mut().unwrap();
        tip.end = end;
        match &mut tip.kind {
            NodeKind::Paragraph { lines } | NodeKind::Comment { lines } => lines.push(raw.span),
            NodeKind::FencedCode { lines, .. }
            | NodeKind::IndentedCode { lines }
            | NodeKind::HtmlBlock { lines, .. }
            | NodeKind::Math { lines } => lines.push(raw),
            NodeKind::Table { rows, header, .. } => {
                let cells = split_table_row(self.src, raw.span);
                let mut cells: Vec<Span> = cells.into_iter().take(header.len()).collect();
                while cells.len() < header.len() {
                    cells.push(Span::new(raw.span.end, raw.span.end));
                }
                rows.push(cells);
            }
            _ => {}
        }
    }

    // ----- continuation ------------------------------------------------------

    fn try_continue(&mut self, i: usize) -> Continue {
        let src = self.src;
        match &self.open[i].kind {
            NodeKind::Document => Continue::Matched,
            NodeKind::BlockQuote { .. } => {
                if !self.indented && self.byte(self.next_nonspace) == Some(b'>') {
                    self.advance_next_nonspace();
                    self.advance_offset(1, false);
                    if self.byte(self.offset).is_some_and(is_space_or_tab) {
                        self.advance_offset(1, true);
                    }
                    self.open[i].end = self.line.end;
                    Continue::Matched
                } else {
                    Continue::NotMatched
                }
            }
            NodeKind::Item { content_indent, .. } => {
                let content_indent = *content_indent;
                if self.blank {
                    // An open child higher on the stack counts as content too.
                    if !self.open[i].has_children() && i + 1 == self.open.len() {
                        Continue::NotMatched
                    } else {
                        self.advance_next_nonspace();
                        Continue::Matched
                    }
                } else if self.indent >= content_indent {
                    self.advance_offset(content_indent, true);
                    Continue::Matched
                } else if self.opts.quirks
                    && !self.indented
                    && self.byte(self.next_nonspace) == Some(b'>')
                    && self
                        .open
                        .get(i + 1)
                        .is_some_and(|open| matches!(open.kind, NodeKind::BlockQuote { .. }))
                {
                    // A quote that `try_block_starts` kept inside this item
                    // keeps its later lines there too. Those lines carry the
                    // item's own indentation no more than the first one did, so
                    // the test above rejects them and the quote would restart
                    // as a sibling of the whole list -- `- item` / `> quote` /
                    // `> more` put "more" in a second blockquote outside the
                    // item. Match without consuming: the quote's own arm takes
                    // the `>` on the next turn of the loop.
                    Continue::Matched
                } else {
                    Continue::NotMatched
                }
            }
            NodeKind::List { .. } => Continue::Matched,
            NodeKind::FootnoteDefinition { .. } => {
                if self.blank {
                    self.advance_next_nonspace();
                    Continue::Matched
                } else if self.indent >= 4 {
                    self.advance_offset(4, true);
                    Continue::Matched
                } else {
                    Continue::NotMatched
                }
            }
            NodeKind::FencedCode { fence, .. } => {
                let fence = *fence;
                if self.indent <= 3 && self.byte(self.next_nonspace) == Some(fence.marker) {
                    let rest = self.rest();
                    let run = rest.bytes().take_while(|&b| b == fence.marker).count();
                    if run >= fence.length && rest[run..].bytes().all(is_space_or_tab) {
                        self.open[i].end = self.line.end;
                        self.finalize();
                        return Continue::Finished;
                    }
                }
                let mut k = fence.indent;
                while k > 0 && self.byte(self.offset).is_some_and(is_space_or_tab) {
                    self.advance_offset(1, true);
                    k -= 1;
                }
                Continue::Matched
            }
            NodeKind::IndentedCode { .. } => {
                if self.indent >= 4 {
                    self.advance_offset(4, true);
                    Continue::Matched
                } else if self.blank {
                    self.advance_next_nonspace();
                    Continue::Matched
                } else {
                    Continue::NotMatched
                }
            }
            NodeKind::HtmlBlock { kind, .. } => {
                if self.blank && (*kind == 6 || *kind == 7) {
                    Continue::NotMatched
                } else {
                    Continue::Matched
                }
            }
            NodeKind::Paragraph { .. } => {
                if self.blank {
                    Continue::NotMatched
                } else {
                    Continue::Matched
                }
            }
            NodeKind::Table { .. } => {
                // A row must contain a pipe; Obsidian ends the table otherwise.
                if self.blank || !self.rest().contains('|') {
                    Continue::NotMatched
                } else {
                    Continue::Matched
                }
            }
            NodeKind::Math { .. } => {
                let rest = self.rest().trim_end_matches(is_space_or_tab_char);
                if let Some(before) = rest.strip_suffix("$$") {
                    let span = Span::new(self.next_nonspace, self.next_nonspace + before.len());
                    if !before.trim().is_empty() {
                        if let NodeKind::Math { lines, .. } = &mut self.open[i].kind {
                            lines.push(RawLine { span, pad: 0 });
                        }
                    }
                    self.open[i].end = self.line.end;
                    self.finalize();
                    return Continue::Finished;
                }
                Continue::Matched
            }
            NodeKind::Comment { .. } => {
                let rest = &src[self.offset..self.line.end];
                if let Some(idx) = rest.find("%%") {
                    let span = Span::new(self.offset, self.offset + idx);
                    if let NodeKind::Comment { lines } = &mut self.open[i].kind {
                        lines.push(span);
                    }
                    self.open[i].end = self.line.end;
                    self.finalize();
                    return Continue::Finished;
                }
                Continue::Matched
            }
            NodeKind::Heading { .. } | NodeKind::ThematicBreak => Continue::NotMatched,
        }
    }

    // ----- block starts ------------------------------------------------------

    fn try_block_starts(&mut self, container: usize) -> Option<Start> {
        let c = self.byte(self.next_nonspace)?;
        let tip_is_paragraph = self.open[container].is_paragraph();

        // Block quote.
        if !self.indented && c == b'>' {
            let start = self.next_nonspace;
            self.advance_next_nonspace();
            self.advance_offset(1, false);
            if self.byte(self.offset).is_some_and(is_space_or_tab) {
                self.advance_offset(1, true);
            }
            // Obsidian keeps a quote that directly follows a list item's text
            // inside that item, where CommonMark would end the list.
            if self.opts.quirks
                && !self.all_closed
                && self.open.len() == self.last_matched + 3
                && matches!(self.open[self.last_matched].kind, NodeKind::List { .. })
                && matches!(self.open[self.last_matched + 1].kind, NodeKind::Item { .. })
                && self.tip().is_paragraph()
            {
                self.last_matched += 1;
            }
            self.close_unmatched_blocks();
            self.add_child(NodeKind::BlockQuote { header: None }, start);
            return Some(Start::Container);
        }

        // Callout header: `[!type]` as the first line of a quote. This runs on
        // the next pass through, after the `>` prefix above has been consumed,
        // and consumes the whole line so that nothing downstream ever sees the
        // header as body content. See [`CalloutHeader`] for why that matters.
        if self.opts.callouts
            && !self.indented
            && c == b'['
            && container == self.open.len() - 1
            && matches!(
                &self.open[container].kind,
                NodeKind::BlockQuote { header: None }
            )
            // The header belongs to the quote's first block, exactly as the
            // old post-hoc reading required: `> text` then `> [!note]` is not
            // a callout, and neither is a header after a heading.
            && !self.open[container].has_children()
        {
            if let Some((callout_type, fold, title_offset)) = parse_callout_header_raw(self.rest())
            {
                let title_start = self.next_nonspace + title_offset;
                let text = &self.src[title_start..self.line.end];
                let lead = text.len() - text.trim_start().len();
                let title = Span::new(title_start + lead, title_start + lead + text.trim().len());
                let fold = match fold {
                    Some(true) => Some(Fold::Collapsed),
                    Some(false) => Some(Fold::Expanded),
                    None => None,
                };
                let NodeKind::BlockQuote { header } = &mut self.open[container].kind else {
                    unreachable!()
                };
                *header = Some(CalloutHeader {
                    callout_type,
                    fold,
                    title,
                });
                self.offset = self.line.end;
                self.line_consumed = true;
                return Some(Start::Leaf);
            }
        }

        // ATX heading.
        if !self.indented && c == b'#' {
            let rest = self.rest();
            let hashes = rest.bytes().take_while(|&b| b == b'#').count();
            if (1..=6).contains(&hashes)
                && (rest.len() == hashes || is_space_or_tab(rest.as_bytes()[hashes]))
            {
                let start = self.next_nonspace;
                self.advance_next_nonspace();
                self.advance_offset(hashes, false);
                self.close_unmatched_blocks();
                let content = atx_content(&rest[hashes..]);
                let content_start = self.offset + content.0;
                let content_span = Span::new(content_start, content_start + content.1);
                self.add_child(
                    NodeKind::Heading {
                        level: hashes as u8,
                        lines: vec![content_span],
                        setext: false,
                    },
                    start,
                );
                self.offset = self.line.end;
                return Some(Start::Leaf);
            }
        }

        // Fenced code.
        if !self.indented && (c == b'`' || c == b'~') {
            let rest = self.rest();
            let run = rest.bytes().take_while(|&b| b == c).count();
            if run >= 3 && (c == b'~' || !rest[run..].contains('`')) {
                let start = self.next_nonspace;
                let fence = Fence {
                    marker: c,
                    length: run,
                    indent: self.indent,
                };
                let info_raw = rest[run..].trim();
                let info = (!info_raw.is_empty()).then(|| scan::unescape(info_raw));
                self.close_unmatched_blocks();
                self.add_child(
                    NodeKind::FencedCode {
                        fence,
                        info,
                        lines: Vec::new(),
                    },
                    start,
                );
                self.advance_next_nonspace();
                self.advance_offset(run, false);
                self.offset = self.line.end;
                self.line_consumed = true;
                return Some(Start::Leaf);
            }
        }

        // HTML block.
        if !self.indented && c == b'<' {
            if let Some(kind) = scan::html_block_start(self.rest(), tip_is_paragraph) {
                self.close_unmatched_blocks();
                let start = self.offset;
                self.add_child(
                    NodeKind::HtmlBlock {
                        kind,
                        lines: Vec::new(),
                    },
                    start,
                );
                return Some(Start::Leaf);
            }
        }

        // Table (GFM): a one-line paragraph followed by a delimiter row. Obsidian
        // does not let a table interrupt a longer paragraph.
        let single_line_paragraph = matches!(
            &self.open[container].kind,
            NodeKind::Paragraph { lines } if lines.len() == 1
        );
        if self.opts.tables && single_line_paragraph && matches!(c, b'|' | b'-' | b':') {
            if let Some(alignments) = parse_delimiter_row(self.rest()) {
                let (header_line, header_cells) = {
                    let NodeKind::Paragraph { lines } = &self.open[container].kind else {
                        unreachable!()
                    };
                    let last = *lines.last().unwrap();
                    (last, split_table_row(self.src, last))
                };
                let has_pipe = header_line.of(self.src).contains('|') || self.rest().contains('|');
                if has_pipe && header_cells.len() == alignments.len() {
                    self.close_unmatched_blocks();
                    let node = self.open.last_mut().unwrap();
                    let NodeKind::Paragraph { lines } = &mut node.kind else {
                        unreachable!()
                    };
                    lines.pop();
                    let table_start = header_line.start;
                    if lines.is_empty() {
                        self.open.pop();
                    } else {
                        self.finalize();
                    }
                    self.add_child(
                        NodeKind::Table {
                            alignments,
                            header: header_cells,
                            rows: Vec::new(),
                        },
                        table_start,
                    );
                    self.offset = self.line.end;
                    self.line_consumed = true;
                    return Some(Start::Leaf);
                }
            }
        }

        // Setext heading underline. Obsidian only underlines a one-line paragraph.
        let setext_ok = tip_is_paragraph && (!self.opts.quirks || single_line_paragraph);
        if !self.indented && setext_ok && (c == b'=' || c == b'-') {
            let rest = self.rest();
            let run = rest.bytes().take_while(|&b| b == c).count();
            if rest[run..].bytes().all(is_space_or_tab) {
                self.close_unmatched_blocks();
                // Reference definitions at the start of the paragraph are not heading text.
                let lines = {
                    let NodeKind::Paragraph { lines } = &self.open[container].kind else {
                        unreachable!()
                    };
                    lines.clone()
                };
                let mut lines = lines;
                while let Some(n) = self.extract_reference_definition(&lines) {
                    lines.drain(..n);
                }
                if !lines.is_empty() {
                    let node = self.open.last_mut().unwrap();
                    node.start = lines[0].start;
                    node.kind = NodeKind::Heading {
                        level: if c == b'=' { 1 } else { 2 },
                        lines,
                        setext: true,
                    };
                    self.offset = self.line.end;
                    return Some(Start::Leaf);
                }
                // Only definitions: the paragraph is empty and the underline is text.
                let node = self.open.last_mut().unwrap();
                if let NodeKind::Paragraph { lines } = &mut node.kind {
                    lines.clear();
                }
            }
        }

        // Thematic break.
        if !self.indented && matches!(c, b'*' | b'-' | b'_') && is_thematic_break(self.rest()) {
            let start = self.next_nonspace;
            self.close_unmatched_blocks();
            self.add_child(NodeKind::ThematicBreak, start);
            self.offset = self.line.end;
            return Some(Start::Leaf);
        }

        // Math block: `$$` opening a line without closing on it. A `$$...$$`
        // pair on one line is inline display math inside a paragraph.
        if self.opts.math && !self.indented && c == b'$' && self.rest().starts_with("$$") {
            let rest = &self.rest()[2..];
            let start = self.next_nonspace;
            if !rest.contains("$$") {
                self.close_unmatched_blocks();
                let lines = if rest.trim().is_empty() {
                    Vec::new()
                } else {
                    vec![RawLine {
                        span: Span::new(start + 2, self.line.end),
                        pad: 0,
                    }]
                };
                self.add_child(NodeKind::Math { lines }, start);
                self.offset = self.line.end;
                self.line_consumed = true;
                return Some(Start::Leaf);
            }
        }

        // Comment block: `%%` opening a line without closing on it. A closed
        // `%%...%%` on one line is an inline comment inside a paragraph.
        if self.opts.comments && !self.indented && c == b'%' && self.rest().starts_with("%%") {
            let rest = &self.rest()[2..];
            let start = self.next_nonspace;
            match rest.find("%%") {
                Some(_) => {}
                None => {
                    self.close_unmatched_blocks();
                    self.add_child(
                        NodeKind::Comment {
                            lines: vec![Span::new(start + 2, self.line.end)],
                        },
                        start,
                    );
                    self.offset = self.line.end;
                    self.line_consumed = true;
                    return Some(Start::Leaf);
                }
            }
        }

        // Footnote definition.
        if self.opts.footnotes && !self.indented && c == b'[' && !tip_is_paragraph {
            if let Some((label, len)) = scan_footnote_label(self.rest()) {
                let start = self.next_nonspace;
                self.advance_next_nonspace();
                self.advance_offset(len, false);
                while self.byte(self.offset).is_some_and(is_space_or_tab) {
                    self.advance_offset(1, true);
                }
                self.close_unmatched_blocks();
                if !self.footnote_labels.contains(&label) {
                    self.footnote_labels.push(label);
                }
                self.add_child(NodeKind::FootnoteDefinition { label }, start);
                return Some(Start::Container);
            }
        }

        // List item.
        let in_list = matches!(self.open[container].kind, NodeKind::List { .. });
        if (!self.indented || in_list) && (matches!(c, b'-' | b'+' | b'*') || c.is_ascii_digit()) {
            if let Some(item) = self.parse_list_marker(tip_is_paragraph, in_list) {
                self.close_unmatched_blocks();
                let needs_list = match &self.tip().kind {
                    NodeKind::List { kind, .. } => !lists_match(*kind, item.kind),
                    _ => true,
                };
                if needs_list {
                    self.add_child(
                        NodeKind::List {
                            kind: item.kind,
                            items: Vec::new(),
                            items_meta: Vec::new(),
                        },
                        item.marker.start,
                    );
                }
                self.add_child(
                    NodeKind::Item {
                        marker: item.marker,
                        content_indent: item.marker_offset + item.padding,
                        task: item.task,
                        task_span: item.task_span,
                    },
                    item.marker.start,
                );
                return Some(Start::Container);
            }
        }

        // Indented code: cannot interrupt a paragraph, even a lazily continued one.
        if self.indented && !self.tip().is_paragraph() && !self.blank {
            self.advance_offset(4, true);
            self.close_unmatched_blocks();
            let start = self.offset;
            self.add_child(NodeKind::IndentedCode { lines: Vec::new() }, start);
            return Some(Start::Leaf);
        }

        None
    }

    fn parse_list_marker(&mut self, tip_is_paragraph: bool, in_list: bool) -> Option<ListMarker> {
        if self.indent >= 4 && !(self.opts.quirks && in_list) {
            return None;
        }
        let rest = self.rest();
        let b = rest.as_bytes();
        let (kind, marker_len) = match b[0] {
            m @ (b'-' | b'+' | b'*') => (ListKind::Bullet { marker: m }, 1),
            _ => {
                let digits = b.iter().take_while(|c| c.is_ascii_digit()).count();
                if digits == 0 || digits > 9 {
                    return None;
                }
                let delimiter = *b.get(digits)?;
                if delimiter != b'.' && delimiter != b')' {
                    return None;
                }
                let start: u64 = rest[..digits].parse().ok()?;
                if tip_is_paragraph && start != 1 {
                    return None;
                }
                (ListKind::Ordered { start, delimiter }, digits + 1)
            }
        };
        // The marker must be followed by whitespace or the end of the line.
        match b.get(marker_len) {
            None => {}
            Some(&c) if is_space_or_tab(c) => {}
            _ => return None,
        }
        // A list item cannot interrupt a paragraph with a blank first line.
        if tip_is_paragraph && rest[marker_len..].bytes().all(is_space_or_tab) {
            return None;
        }
        let marker_offset = self.indent;
        let marker = Span::new(self.next_nonspace, self.next_nonspace + marker_len);
        self.advance_next_nonspace();
        self.advance_offset(marker_len, true);
        let spaces_start_col = self.column;
        let spaces_start_offset = self.offset;
        let spaces_start_tab = self.partially_consumed_tab;
        loop {
            self.advance_offset(1, true);
            let next = self.byte(self.offset);
            if !(self.column - spaces_start_col < 5 && next.is_some_and(is_space_or_tab)) {
                break;
            }
        }
        let blank_item = self.offset >= self.line.end;
        let spaces_after_marker = self.column - spaces_start_col;
        let padding = if !(1..5).contains(&spaces_after_marker) || blank_item {
            self.column = spaces_start_col;
            self.offset = spaces_start_offset;
            self.partially_consumed_tab = spaces_start_tab;
            if self.byte(self.offset).is_some_and(is_space_or_tab) {
                self.advance_offset(1, true);
            }
            marker_len + 1
        } else {
            marker_len + spaces_after_marker
        };
        let mut task = None;
        let mut task_span = None;
        if self.opts.task_lists && !blank_item {
            let after = &self.src[self.offset..self.line.end];
            let ab = after.as_bytes();
            if ab.len() > 3 && ab[0] == b'[' && ab[2] == b']' && ab[1] != b'\n' {
                // Obsidian needs whitespace after the bracket; `- [ ]` alone is text.
                let ok = is_space_or_tab(ab[3]);
                if ok {
                    task = Some(ab[1] as char);
                    // `self.offset` is absolute into `self.src`, and any
                    // container markers are already consumed, so a task inside
                    // a blockquote or callout gets a correct span too.
                    task_span = Some(Span::new(self.offset, self.offset + 3));
                    self.advance_offset(3, false);
                    if self.byte(self.offset).is_some_and(is_space_or_tab) {
                        self.advance_offset(1, true);
                    }
                }
            }
        }
        Some(ListMarker {
            kind,
            marker,
            marker_offset,
            padding,
            task,
            task_span,
        })
    }
}

struct ListMarker {
    kind: ListKind,
    marker: Span,
    marker_offset: usize,
    padding: usize,
    task: Option<char>,
    task_span: Option<Span>,
}

fn lists_match(a: ListKind, b: ListKind) -> bool {
    match (a, b) {
        (ListKind::Bullet { marker: x }, ListKind::Bullet { marker: y }) => x == y,
        (ListKind::Ordered { delimiter: x, .. }, ListKind::Ordered { delimiter: y, .. }) => x == y,
        _ => false,
    }
}

fn is_space_or_tab_char(c: char) -> bool {
    c == ' ' || c == '\t'
}

fn is_block_start_char(b: u8) -> bool {
    matches!(
        b,
        b'>' | b'#'
            | b'`'
            | b'~'
            | b'*'
            | b'+'
            | b'_'
            | b'='
            | b'<'
            | b'-'
            | b'|'
            | b':'
            | b'$'
            | b'%'
            | b'['
    ) || b.is_ascii_digit()
}

/// Returns `(offset, len)` of the heading text inside the text after the `#`s,
/// with the optional closing sequence and surrounding whitespace removed.
fn atx_content(after_hashes: &str) -> (usize, usize) {
    let trimmed_start =
        after_hashes.len() - after_hashes.trim_start_matches(is_space_or_tab_char).len();
    let mut text = after_hashes[trimmed_start..].trim_end_matches(is_space_or_tab_char);
    // Closing sequence: spaces, hashes, spaces at the end; or only hashes.
    let stripped = text.trim_end_matches('#');
    if stripped.len() != text.len() {
        if stripped.is_empty() {
            text = "";
        } else if stripped.ends_with(is_space_or_tab_char) {
            text = stripped.trim_end_matches(is_space_or_tab_char);
        }
    }
    (trimmed_start, text.len())
}

fn is_thematic_break(rest: &str) -> bool {
    let b = rest.as_bytes();
    let marker = b[0];
    if !matches!(marker, b'*' | b'-' | b'_') {
        return false;
    }
    let mut count = 0;
    for &c in b {
        if c == marker {
            count += 1;
        } else if !is_space_or_tab(c) {
            return false;
        }
    }
    count >= 3
}

/// `[^label]:` with the label free of whitespace and brackets.
fn scan_footnote_label(rest: &str) -> Option<(&str, usize)> {
    let inner = rest.strip_prefix("[^")?;
    let end = inner.find(']')?;
    let label = &inner[..end];
    if label.is_empty() || label.bytes().any(|b| b.is_ascii_whitespace() || b == b'[') {
        return None;
    }
    let after = &inner[end + 1..];
    if !after.starts_with(':') {
        return None;
    }
    Some((label, 2 + end + 2))
}

/// Parses a GFM delimiter row such as `| :--- | ---: |`.
fn parse_delimiter_row(rest: &str) -> Option<Vec<Alignment>> {
    let mut s = rest.trim();
    if s.is_empty() {
        return None;
    }
    if let Some(r) = s.strip_prefix('|') {
        s = r;
    }
    if let Some(r) = s.strip_suffix('|') {
        s = r;
    }
    let mut out = Vec::new();
    for cell in s.split('|') {
        let cell = cell.trim();
        let left = cell.starts_with(':');
        let right = cell.ends_with(':') && cell.len() > 1;
        let dashes = cell.trim_start_matches(':').trim_end_matches(':');
        if dashes.is_empty() || !dashes.bytes().all(|b| b == b'-') {
            return None;
        }
        out.push(match (left, right) {
            (true, true) => Alignment::Center,
            (true, false) => Alignment::Left,
            (false, true) => Alignment::Right,
            (false, false) => Alignment::None,
        });
    }
    Some(out)
}

/// Splits a table row into trimmed cell spans; `\|` does not split.
fn split_table_row(src: &str, line: Span) -> Vec<Span> {
    let text = line.of(src);
    let bytes = src.as_bytes();
    let mut start = line.start + (text.len() - text.trim_start().len());
    let mut end = line.start + text.trim_end().len();
    if start < end && bytes[start] == b'|' {
        start += 1;
    }
    if end > start && bytes[end - 1] == b'|' && !(end >= 2 && bytes[end - 2] == b'\\') {
        end -= 1;
    }
    let mut cells = Vec::new();
    let mut i = start;
    let mut cell_start = start;
    while i < end {
        match bytes[i] {
            b'\\' if i + 1 < end => i += 2,
            b'|' => {
                cells.push(trim_span(src, Span::new(cell_start, i)));
                i += 1;
                cell_start = i;
            }
            _ => i += 1,
        }
    }
    cells.push(trim_span(src, Span::new(cell_start, end)));
    cells
}

fn trim_span(src: &str, span: Span) -> Span {
    let text = span.of(src);
    let lead = text.len() - text.trim_start().len();
    let trail = text.len() - text.trim_end().len();
    if lead + trail >= text.len() {
        return Span::new(span.start, span.start);
    }
    Span::new(span.start + lead, span.end - trail)
}

/// Parses `[!type]- Title`; returns the type, fold flag, and byte offset of the title.
pub(crate) fn parse_callout_header_raw(line: &str) -> Option<(&str, Option<bool>, usize)> {
    let lead = line.len() - line.trim_start().len();
    let rest = line[lead..].strip_prefix("[!")?;
    let close = rest.find(']')?;
    let raw = &rest[..close];
    // Obsidian wants *something* between the brackets and then trims it, so
    // `[! bad]` is a `bad` callout and `[! ]` is one with an empty type (an
    // empty `data-callout` and no default title), while a bare `[!]` is not a
    // callout at all. Observed against 1.14.4 (tools/probe-gen, 2026-10-29).
    if raw.is_empty() || raw.contains('[') {
        return None;
    }
    let ty = raw.trim();
    let after = &rest[close + 1..];
    let (fold, skip) = match after.as_bytes().first() {
        Some(b'-') => (Some(true), 1),
        Some(b'+') => (Some(false), 1),
        _ => (None, 0),
    };
    // The header must end here or continue with a space before the title.
    let after = &after[skip..];
    if !after.is_empty() && !after.starts_with(is_space_or_tab_char) {
        return None;
    }
    Some((ty, fold, lead + 2 + close + 1 + skip))
}
