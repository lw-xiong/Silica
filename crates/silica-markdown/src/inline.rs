//! Inline structure, the second parsing phase.
//!
//! Paragraph, heading, callout-title and table-cell content arrives as source
//! spans (one per line); they are joined with `\n` for scanning and every node
//! span is mapped back to the source. The algorithm is CommonMark's delimiter
//! stack for emphasis and bracket stack for links, with Obsidian's wikilinks,
//! tags, math, comments, highlights and footnotes scanned in the same pass.

use crate::ast::*;
use crate::scan::{
    self, is_ascii_punct, is_space_or_tab, is_unicode_punctuation, is_unicode_whitespace,
};
use crate::span::Span;
use crate::Options;
use std::borrow::Cow;
use std::collections::HashMap;

/// Document-wide state shared by every inline parse.
pub(crate) struct InlineContext<'a, 'o> {
    pub src: &'a str,
    pub opts: &'o Options,
    pub definitions: &'o HashMap<String, LinkDefinition<'a>>,
    pub footnote_labels: &'o [&'a str],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BracketKind {
    Link,
    Image,
    Footnote,
}

struct Bracket {
    /// Index of the `[` text node.
    node: usize,
    kind: BracketKind,
    /// Text position just after the opening bracket.
    pos_after: usize,
    active: bool,
    /// Top of the delimiter list when the bracket was pushed.
    delim_bottom: Option<usize>,
    bracket_after: bool,
}

struct Delim {
    node: usize,
    ch: u8,
    /// Text positions of the (remaining) delimiter run.
    text_start: usize,
    text_end: usize,
    orig_count: usize,
    can_open: bool,
    can_close: bool,
    prev: Option<usize>,
    next: Option<usize>,
    removed: bool,
}

struct Parser<'a, 'o, 'c> {
    cx: &'c InlineContext<'a, 'o>,
    text: Cow<'a, str>,
    /// `(text offset, source span)` for every line segment.
    segs: Vec<(usize, Span)>,
    pos: usize,
    end: usize,
    nodes: Vec<Inline<'a>>,
    delims: Vec<Delim>,
    delim_top: Option<usize>,
    brackets: Vec<Bracket>,
}

/// Parses the inline content of `lines`.
pub(crate) fn parse_inlines<'a>(cx: &InlineContext<'a, '_>, lines: &[Span]) -> Vec<Inline<'a>> {
    if lines.is_empty() {
        return Vec::new();
    }
    let mut segs = Vec::with_capacity(lines.len());
    let text: Cow<'a, str> = if lines.len() == 1 {
        segs.push((0, lines[0]));
        Cow::Borrowed(lines[0].of(cx.src))
    } else {
        let mut s = String::new();
        for (i, l) in lines.iter().enumerate() {
            if i > 0 {
                s.push('\n');
            }
            segs.push((s.len(), *l));
            s.push_str(l.of(cx.src));
        }
        Cow::Owned(s)
    };
    let trimmed_start = text.len() - text.trim_start().len();
    let trimmed_end = text.trim_end().len();
    let mut p = Parser {
        cx,
        text,
        segs,
        pos: trimmed_start,
        end: trimmed_end.max(trimmed_start),
        nodes: Vec::new(),
        delims: Vec::new(),
        delim_top: None,
        brackets: Vec::new(),
    };
    p.run();
    let mut nodes = p.nodes;
    merge_text(&mut nodes);
    nodes
}

/// The colour emoji a highlight opens with, taken off its first text node:
/// `==🔴word==` is a red highlight of `word`, and the emoji is not shown.
fn take_highlight_color(children: &mut Vec<Inline<'_>>) -> Option<HighlightColor> {
    let first = children.first_mut()?;
    let InlineKind::Text(text) = &mut first.kind else {
        return None;
    };
    let (color, len) = HighlightColor::leading(text)?;
    match text {
        Cow::Borrowed(t) => *t = &t[len..],
        Cow::Owned(t) => t.drain(..len).for_each(drop),
    }
    first.span.start += len;
    if text.is_empty() {
        children.remove(0);
    }
    Some(color)
}

/// Joins adjacent text nodes so escapes and entities do not fragment runs.
fn merge_text(nodes: &mut Vec<Inline<'_>>) {
    let mut i = 0;
    while i < nodes.len() {
        match &mut nodes[i].kind {
            InlineKind::Emphasis(c)
            | InlineKind::Strong(c)
            | InlineKind::Strikethrough(c)
            | InlineKind::Highlight { children: c, .. }
            | InlineKind::Link { children: c, .. }
            | InlineKind::Image { children: c, .. }
            | InlineKind::InlineFootnote(c) => merge_text(c),
            _ => {}
        }
        if i + 1 < nodes.len()
            && matches!(nodes[i].kind, InlineKind::Text(_))
            && matches!(nodes[i + 1].kind, InlineKind::Text(_))
        {
            let next = nodes.remove(i + 1);
            let InlineKind::Text(next_text) = next.kind else {
                unreachable!()
            };
            let cur = &mut nodes[i];
            if let InlineKind::Text(t) = &mut cur.kind {
                t.to_mut().push_str(&next_text);
            }
            cur.span.end = next.span.end;
        } else {
            i += 1;
        }
    }
}

fn is_special(b: u8) -> bool {
    matches!(
        b,
        b'\n'
            | b'\\'
            | b'`'
            | b'*'
            | b'_'
            | b'~'
            | b'='
            | b'['
            | b']'
            | b'!'
            | b'<'
            | b'&'
            | b'#'
            | b'$'
            | b'%'
            | b'^'
    )
}

impl<'a, 'o, 'c> Parser<'a, 'o, 'c> {
    // ----- position mapping --------------------------------------------------

    fn seg_index(&self, p: usize) -> usize {
        self.segs.partition_point(|s| s.0 <= p).saturating_sub(1)
    }

    fn src_pos(&self, p: usize) -> usize {
        let (ts, span) = self.segs[self.seg_index(p)];
        span.start + (p - ts).min(span.len())
    }

    fn span(&self, a: usize, b: usize) -> Span {
        Span::new(self.src_pos(a), self.src_pos(b))
    }

    /// Text between two positions, borrowed from the source when it does not cross a line.
    fn slice(&self, a: usize, b: usize) -> Cow<'a, str> {
        let (ts, span) = self.segs[self.seg_index(a)];
        if b <= ts + span.len() {
            Cow::Borrowed(&self.cx.src[span.start + (a - ts)..span.start + (b - ts)])
        } else {
            Cow::Owned(self.text[a..b].to_string())
        }
    }

    fn byte(&self, p: usize) -> Option<u8> {
        if p < self.end {
            Some(self.text.as_bytes()[p])
        } else {
            None
        }
    }

    fn rest(&self) -> &str {
        &self.text[self.pos..self.end]
    }

    fn push_text(&mut self, a: usize, b: usize) {
        if a < b {
            let t = self.slice(a, b);
            let span = self.span(a, b);
            self.nodes.push(Inline {
                kind: InlineKind::Text(t),
                span,
            });
        }
    }

    fn push_node(&mut self, kind: InlineKind<'a>, a: usize, b: usize) {
        let span = self.span(a, b);
        self.nodes.push(Inline { kind, span });
    }

    // ----- main loop ---------------------------------------------------------

    fn run(&mut self) {
        let opts = self.cx.opts;
        while self.pos < self.end {
            let c = self.text.as_bytes()[self.pos];
            let handled = match c {
                b'\n' => {
                    self.newline();
                    true
                }
                b'\\' => {
                    self.backslash();
                    true
                }
                b'`' => self.code_span(),
                b'*' | b'_' => {
                    self.delimiter(c);
                    true
                }
                b'~' if opts.strikethrough => self.double_delimiter(c),
                b'=' if opts.highlight => self.double_delimiter(c),
                b'[' => {
                    self.open_bracket();
                    true
                }
                b'!' => {
                    self.bang();
                    true
                }
                b']' => {
                    self.close_bracket();
                    true
                }
                b'<' => self.angle(),
                b'&' => self.entity(),
                b'#' if opts.tags => self.tag(),
                b'$' if opts.math => self.math(),
                b'%' if opts.comments => self.comment(),
                b'^' if opts.footnotes => self.inline_footnote(),
                _ => false,
            };
            if !handled {
                self.text_run();
            }
        }
        self.process_emphasis(None);
    }

    fn text_run(&mut self) {
        let start = self.pos;
        let b = self.text.as_bytes();
        let bare_urls = self.cx.opts.bare_urls;
        if bare_urls {
            if let Some((len, prefix)) = self.scan_bare_url(start) {
                self.emit_bare_link(start, start + len, prefix);
                return;
            }
        }
        let mut i = self.pos;
        // Always consume at least one byte so unhandled specials become text.
        i += self.text[i..].chars().next().map_or(1, char::len_utf8);
        while i < self.end {
            let c = b[i];
            if is_special(c) {
                break;
            }
            if bare_urls && (c == b'h' || c == b'w') && self.scan_bare_url(i).is_some() {
                break;
            }
            if bare_urls && c == b'@' {
                if let Some((local_start, len)) = self.scan_email(start, i) {
                    self.push_text(start, local_start);
                    self.emit_bare_link(local_start, local_start + len, "mailto:");
                    return;
                }
            }
            i += 1;
        }
        self.push_text(start, i);
        self.pos = i;
    }

    fn emit_bare_link(&mut self, start: usize, end: usize, dest_prefix: &str) {
        let dest = format!("{dest_prefix}{}", &self.text[start..end]);
        let text = self.slice(start, end);
        let span = self.span(start, end);
        self.nodes.push(Inline {
            kind: InlineKind::Link {
                destination: Cow::Owned(dest),
                title: None,
                kind: LinkKind::BareUrl,
                children: vec![Inline {
                    kind: InlineKind::Text(text),
                    span,
                }],
            },
            span,
        });
        self.pos = end;
    }

    /// An email address around the `@` at `at`; the local part may start no
    /// earlier than `run_start`. Returns its start and length.
    fn scan_email(&self, run_start: usize, at: usize) -> Option<(usize, usize)> {
        let b = self.text.as_bytes();
        let is_local = |c: u8| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-' | b'+');
        let mut ls = at;
        while ls > run_start && is_local(b[ls - 1]) {
            ls -= 1;
        }
        if ls == at {
            return None;
        }
        let mut de = at + 1;
        while de < self.end && (b[de].is_ascii_alphanumeric() || matches!(b[de], b'.' | b'-')) {
            de += 1;
        }
        while de > at + 1 && matches!(b[de - 1], b'.' | b'-') {
            de -= 1;
        }
        let domain = &self.text[at + 1..de];
        if domain.is_empty() || !domain.contains('.') || domain.starts_with('.') {
            return None;
        }
        Some((ls, de - ls))
    }

    /// Returns the URL length and the prefix to add to the destination.
    /// Obsidian links `https://`, `http://` and `www.` wherever they appear.
    fn scan_bare_url(&self, i: usize) -> Option<(usize, &'static str)> {
        let rest = &self.text[i..self.end];
        let (prefix, scheme_len) = if rest.starts_with("https://") {
            ("", 8)
        } else if rest.starts_with("http://") {
            ("", 7)
        } else if rest.starts_with("www.") {
            ("http://", 4)
        } else {
            return None;
        };
        let body = &rest[scheme_len..];
        let mut len = body
            .find(|c: char| is_unicode_whitespace(c) || c == '<')
            .unwrap_or(body.len());
        let domain_end = body[..len].find(['/', '?', '#']).unwrap_or(len);
        let domain = &body[..domain_end];
        if domain.is_empty()
            || domain.starts_with('.')
            || (prefix == "http://" && !domain.contains('.'))
            || domain
                .chars()
                .any(|c| !(c.is_alphanumeric() || matches!(c, '-' | '_' | '.')))
        {
            return None;
        }
        // Trailing punctuation and unbalanced parentheses are not part of the URL.
        loop {
            let b = body.as_bytes();
            if len == 0 {
                break;
            }
            let last = b[len - 1];
            if matches!(last, b'?' | b'!' | b'.' | b',' | b':' | b'*' | b'_' | b'~') {
                len -= 1;
            } else if last == b')' {
                let open = body[..len].bytes().filter(|&c| c == b'(').count();
                let close = body[..len].bytes().filter(|&c| c == b')').count();
                if close > open {
                    len -= 1;
                } else {
                    break;
                }
            } else if last == b';' {
                // `&amp;` style entity at the end is excluded.
                match body[..len - 1].rfind('&') {
                    Some(a)
                        if body[a + 1..len - 1]
                            .bytes()
                            .all(|c| c.is_ascii_alphanumeric()) =>
                    {
                        len = a;
                    }
                    _ => {
                        len -= 1;
                    }
                }
            } else {
                break;
            }
        }
        if len == 0 {
            return None;
        }
        Some((scheme_len + len, prefix))
    }

    fn newline(&mut self) {
        let nl = self.pos;
        self.pos += 1;
        let mut hard = false;
        if let Some(last) = self.nodes.last_mut() {
            if let InlineKind::Text(t) = &mut last.kind {
                let trimmed = t.trim_end_matches(' ');
                let removed = t.len() - trimmed.len();
                hard = removed >= 2;
                if removed > 0 {
                    let new_len = trimmed.len();
                    match t {
                        Cow::Borrowed(s) => *s = &s[..new_len],
                        Cow::Owned(s) => s.truncate(new_len),
                    }
                    last.span.end -= removed;
                    if new_len == 0 {
                        self.nodes.pop();
                    }
                }
            }
        }
        let kind = if hard {
            InlineKind::HardBreak
        } else {
            InlineKind::SoftBreak
        };
        self.push_node(kind, nl, nl + 1);
        while self.byte(self.pos).is_some_and(is_space_or_tab) {
            self.pos += 1;
        }
    }

    fn backslash(&mut self) {
        let start = self.pos;
        match self.byte(self.pos + 1) {
            Some(b'\n') => {
                self.push_node(InlineKind::HardBreak, start, start + 2);
                self.pos += 2;
                while self.byte(self.pos).is_some_and(is_space_or_tab) {
                    self.pos += 1;
                }
            }
            Some(c) if is_ascii_punct(c) => {
                let t = self.slice(start + 1, start + 2);
                self.push_node(InlineKind::Text(t), start, start + 2);
                self.pos += 2;
            }
            _ => {
                self.push_text(start, start + 1);
                self.pos += 1;
            }
        }
    }

    fn code_span(&mut self) -> bool {
        let start = self.pos;
        let b = self.text.as_bytes();
        let n = b[start..self.end]
            .iter()
            .take_while(|&&c| c == b'`')
            .count();
        let mut i = start + n;
        while i < self.end {
            if b[i] == b'`' {
                let run = b[i..self.end].iter().take_while(|&&c| c == b'`').count();
                if run == n {
                    let inner = self.slice(start + n, i);
                    let content = normalize_code(inner);
                    self.push_node(InlineKind::Code(content), start, i + run);
                    self.pos = i + run;
                    return true;
                }
                i += run;
            } else {
                i += 1;
            }
        }
        // No closer: the run is literal text. Obsidian retries from the next
        // backtick instead of skipping the whole run.
        let literal = if self.cx.opts.quirks { 1 } else { n };
        self.push_text(start, start + literal);
        self.pos = start + literal;
        true
    }

    fn flanking(&self, run_start: usize, run_end: usize) -> (bool, bool, bool, bool) {
        let before = self.text[..run_start].chars().next_back().unwrap_or('\n');
        let after = self.text[run_end..].chars().next().unwrap_or('\n');
        let before_ws = is_unicode_whitespace(before);
        let after_ws = is_unicode_whitespace(after);
        let before_punct = is_unicode_punctuation(before);
        let after_punct = is_unicode_punctuation(after);
        let left = !after_ws && (!after_punct || before_ws || before_punct);
        let right = !before_ws && (!before_punct || after_ws || after_punct);
        (left, right, before_punct, after_punct)
    }

    fn push_delim(&mut self, ch: u8, start: usize, end: usize, can_open: bool, can_close: bool) {
        self.push_text(start, end);
        let node = self.nodes.len() - 1;
        let idx = self.delims.len();
        self.delims.push(Delim {
            node,
            ch,
            text_start: start,
            text_end: end,
            orig_count: end - start,
            can_open,
            can_close,
            prev: self.delim_top,
            next: None,
            removed: false,
        });
        if let Some(top) = self.delim_top {
            self.delims[top].next = Some(idx);
        }
        self.delim_top = Some(idx);
    }

    fn delimiter(&mut self, ch: u8) {
        let start = self.pos;
        let n = self.text.as_bytes()[start..self.end]
            .iter()
            .take_while(|&&c| c == ch)
            .count();
        let end = start + n;
        let (left, right, before_punct, after_punct) = self.flanking(start, end);
        let (can_open, can_close) = if ch == b'_' {
            if self.cx.opts.quirks {
                // Obsidian drops CommonMark's bar on an underscore run
                // *opening* inside a word, and for a run of two or more it
                // drops the bar on closing there too. So `a_b_` is
                // `a<em>b</em>` and `x__y__z` is `x<strong>y</strong>z`, while
                // `a_b_c`, `snake_case_word` and `CONST_NAME_HERE` stay literal
                // -- a single `_` still cannot close before an alphanumeric,
                // which is what keeps ordinary identifiers out of it. Observed
                // against Obsidian 1.14.4 (tools/probe-gen, 2026-10-29).
                let can_close = if n >= 2 {
                    right
                } else {
                    right && (!left || after_punct)
                };
                (left, can_close)
            } else {
                (
                    left && (!right || before_punct),
                    right && (!left || after_punct),
                )
            }
        } else {
            (left, right)
        };
        self.push_delim(ch, start, end, can_open, can_close);
        self.pos = end;
    }

    /// `~~` and `==`: exactly two characters. With quirks on, a longer run
    /// delimits with its first two characters and the rest is text, as
    /// Obsidian renders `~~~x~~~` as `<del>~x</del>~`.
    fn double_delimiter(&mut self, ch: u8) -> bool {
        let start = self.pos;
        let n = self.text.as_bytes()[start..self.end]
            .iter()
            .take_while(|&&c| c == ch)
            .count();
        if n == 1 || (n > 2 && !self.cx.opts.quirks) {
            self.push_text(start, start + n);
            self.pos = start + n;
            return true;
        }
        let end = start + 2;
        let (left, right, _, _) = self.flanking(start, end);
        self.push_delim(ch, start, end, left, right);
        self.pos = end;
        true
    }

    fn remove_delim(&mut self, idx: usize) {
        let (prev, next) = (self.delims[idx].prev, self.delims[idx].next);
        self.delims[idx].removed = true;
        if let Some(p) = prev {
            self.delims[p].next = next;
        }
        match next {
            Some(n) => self.delims[n].prev = prev,
            None => self.delim_top = prev,
        }
    }

    /// Adjusts node indices after `nodes[range]` was replaced by `inserted` nodes.
    fn nodes_replaced(&mut self, range_start: usize, range_end: usize, inserted: usize) {
        let removed = range_end - range_start;
        let fix = |n: &mut usize| {
            if *n >= range_end {
                *n = *n - removed + inserted;
            }
        };
        for d in self.delims.iter_mut() {
            if !d.removed {
                fix(&mut d.node);
            }
        }
        for b in self.brackets.iter_mut() {
            fix(&mut b.node);
        }
    }

    fn remove_node(&mut self, idx: usize) {
        self.nodes.remove(idx);
        self.nodes_replaced(idx, idx + 1, 0);
    }

    fn process_emphasis(&mut self, stack_bottom: Option<usize>) {
        // openers_bottom keyed by (char, can_open, orig_count % 3).
        let mut openers_bottom: HashMap<(u8, bool, usize), Option<usize>> = HashMap::new();
        let mut closer = match stack_bottom {
            Some(b) => self.delims[b].next,
            None => first_delim(&self.delims, self.delim_top),
        };
        while let Some(c) = closer {
            if !self.delims[c].can_close {
                closer = self.delims[c].next;
                continue;
            }
            let (ch, c_open, c_orig) = {
                let d = &self.delims[c];
                (d.ch, d.can_open, d.orig_count)
            };
            let key = (ch, c_open, c_orig % 3);
            let bottom = *openers_bottom.entry(key).or_insert(stack_bottom);
            let mut opener = self.delims[c].prev;
            let mut found = None;
            while let Some(o) = opener {
                if Some(o) == stack_bottom || Some(o) == bottom {
                    break;
                }
                let od = &self.delims[o];
                let odd_match = (c_open || od.can_close)
                    && c_orig % 3 != 0
                    && (od.orig_count + c_orig) % 3 == 0;
                if od.ch == ch && od.can_open && !odd_match {
                    found = Some(o);
                    break;
                }
                opener = od.prev;
            }
            let old_closer = c;
            match found {
                Some(o) => {
                    let o_count = self.delims[o].text_end - self.delims[o].text_start;
                    let c_count = self.delims[c].text_end - self.delims[c].text_start;
                    let is_double = ch == b'~' || ch == b'=';
                    // Obsidian nests `***x***` as strong outside em, so with
                    // quirks on a three-on-three match takes one delimiter first.
                    let triple = self.cx.opts.quirks && o_count >= 3 && c_count >= 3;
                    let use_count = if is_double || (o_count >= 2 && c_count >= 2 && !triple) {
                        2
                    } else {
                        1
                    };
                    // Shrink the delimiter text nodes.
                    self.delims[o].text_end -= use_count;
                    self.delims[c].text_start += use_count;
                    let (o_node, c_node) = (self.delims[o].node, self.delims[c].node);
                    let (os, oe) = (self.delims[o].text_start, self.delims[o].text_end);
                    let (cs, ce) = (self.delims[c].text_start, self.delims[c].text_end);
                    self.nodes[o_node] = Inline {
                        kind: InlineKind::Text(self.slice(os, oe)),
                        span: self.span(os, oe),
                    };
                    self.nodes[c_node] = Inline {
                        kind: InlineKind::Text(self.slice(cs, ce)),
                        span: self.span(cs, ce),
                    };
                    // Wrap the nodes between the delimiters.
                    let mut children: Vec<Inline<'a>> =
                        self.nodes.drain(o_node + 1..c_node).collect();
                    let span = self.span(oe, cs);
                    let kind = match (ch, use_count) {
                        (b'~', _) => InlineKind::Strikethrough(children),
                        (b'=', _) => {
                            let color = take_highlight_color(&mut children);
                            InlineKind::Highlight { color, children }
                        }
                        (_, 2) => InlineKind::Strong(children),
                        _ => InlineKind::Emphasis(children),
                    };
                    self.nodes.insert(o_node + 1, Inline { kind, span });
                    self.nodes_replaced(o_node + 1, c_node, 1);
                    // Remove delimiters between opener and closer.
                    let mut between = self.delims[o].next;
                    while let Some(d) = between {
                        if d == c {
                            break;
                        }
                        let next = self.delims[d].next;
                        self.remove_delim(d);
                        between = next;
                    }
                    if self.delims[o].text_start == self.delims[o].text_end {
                        let n = self.delims[o].node;
                        self.remove_node(n);
                        self.remove_delim(o);
                    }
                    if self.delims[c].text_start == self.delims[c].text_end {
                        let n = self.delims[c].node;
                        let next = self.delims[c].next;
                        self.remove_node(n);
                        self.remove_delim(c);
                        closer = next;
                    } else {
                        closer = Some(c);
                    }
                }
                None => {
                    openers_bottom.insert(key, self.delims[old_closer].prev);
                    let next = self.delims[old_closer].next;
                    if !self.delims[old_closer].can_open {
                        self.remove_delim(old_closer);
                    }
                    closer = next;
                }
            }
        }
        // Remove all delimiters above stack_bottom.
        while let Some(top) = self.delim_top {
            if Some(top) == stack_bottom {
                break;
            }
            self.remove_delim(top);
        }
    }

    // ----- brackets ----------------------------------------------------------

    fn push_bracket(&mut self, kind: BracketKind, start: usize, end: usize) {
        self.push_text(start, end);
        let node = self.nodes.len() - 1;
        if let Some(prev) = self.brackets.last_mut() {
            prev.bracket_after = true;
        }
        self.brackets.push(Bracket {
            node,
            kind,
            pos_after: end,
            active: true,
            delim_bottom: self.delim_top,
            bracket_after: false,
        });
        self.pos = end;
    }

    fn open_bracket(&mut self) {
        let start = self.pos;
        if self.cx.opts.wikilinks && self.byte(start + 1) == Some(b'[') {
            if let Some(len) = self.scan_wikilink(start) {
                self.emit_wikilink(start, start + len, false);
                return;
            }
        }
        if self.cx.opts.footnotes && self.byte(start + 1) == Some(b'^') {
            if let Some(len) = self.scan_footnote_ref(start) {
                let label = self.slice(start + 2, start + len - 1);
                self.push_node(InlineKind::FootnoteReference(label), start, start + len);
                self.pos = start + len;
                return;
            }
        }
        self.push_bracket(BracketKind::Link, start, start + 1);
    }

    fn bang(&mut self) {
        let start = self.pos;
        if self.byte(start + 1) == Some(b'[') {
            if self.cx.opts.wikilinks && self.byte(start + 2) == Some(b'[') {
                if let Some(len) = self.scan_wikilink(start + 1) {
                    self.emit_wikilink(start, start + 1 + len, true);
                    return;
                }
            }
            self.push_bracket(BracketKind::Image, start, start + 2);
            return;
        }
        self.push_text(start, start + 1);
        self.pos = start + 1;
    }

    fn inline_footnote(&mut self) -> bool {
        let start = self.pos;
        if self.byte(start + 1) == Some(b'[') {
            self.push_bracket(BracketKind::Footnote, start, start + 2);
            return true;
        }
        false
    }

    /// Length of `[[...]]` starting at `start`, if well formed.
    fn scan_wikilink(&self, start: usize) -> Option<usize> {
        let rest = &self.text[start + 2..self.end];
        let close = rest.find("]]")?;
        let body = &rest[..close];
        if body.is_empty() || body.contains('\n') || body.contains("[[") {
            return None;
        }
        Some(2 + close + 2)
    }

    fn emit_wikilink(&mut self, start: usize, end: usize, embed: bool) {
        let body_start = if embed { start + 3 } else { start + 2 };
        let body = self.slice(body_start, end - 2);
        self.push_node(InlineKind::Wikilink { body, embed }, start, end);
        self.pos = end;
    }

    fn scan_footnote_ref(&self, start: usize) -> Option<usize> {
        let rest = &self.text[start + 2..self.end];
        let close = rest.find(']')?;
        let label = &rest[..close];
        if label.is_empty() || label.bytes().any(|b| b.is_ascii_whitespace() || b == b'[') {
            return None;
        }
        if !self.cx.opts.quirks && !self.cx.footnote_labels.contains(&label) {
            return None;
        }
        Some(2 + close + 1)
    }

    fn close_bracket(&mut self) {
        let close_pos = self.pos;
        self.pos += 1;
        let Some(opener) = self.brackets.pop() else {
            self.push_text(close_pos, close_pos + 1);
            return;
        };
        if !opener.active {
            self.push_text(close_pos, close_pos + 1);
            return;
        }

        let mut matched: Option<(Cow<'a, str>, Option<Cow<'a, str>>, LinkKind)> = None;
        if opener.kind == BracketKind::Footnote {
            matched = Some((Cow::Borrowed(""), None, LinkKind::Inline));
        } else if self.byte(self.pos) == Some(b'(') {
            if let Some((dest, title, len)) = self.scan_inline_link_tail(self.pos) {
                matched = Some((dest, title, LinkKind::Inline));
                self.pos += len;
            }
        }
        if matched.is_none() && opener.kind != BracketKind::Footnote {
            let rest = &self.text[self.pos..self.end];
            let mut label: Option<String> = None;
            let mut consumed = 0;
            if rest.starts_with("[]") {
                consumed = 2;
                label = Some(self.text[opener.pos_after..close_pos].to_string());
            } else if let Some(n) = scan::scan_link_label(rest) {
                consumed = n;
                label = Some(rest[1..n - 1].to_string());
            } else if !opener.bracket_after {
                label = Some(self.text[opener.pos_after..close_pos].to_string());
            }
            if let Some(l) = label {
                if let Some(def) = self.cx.definitions.get(&normalize_label(&l)) {
                    matched = Some((
                        def.destination.clone(),
                        def.title.clone(),
                        LinkKind::Reference,
                    ));
                    self.pos += consumed;
                }
            }
        }

        match matched {
            None => self.push_text(close_pos, close_pos + 1),
            Some((destination, title, kind)) => {
                self.process_emphasis(opener.delim_bottom);
                let children: Vec<Inline<'a>> = self.nodes.drain(opener.node + 1..).collect();
                let bracket_text_start = opener.pos_after
                    - match opener.kind {
                        BracketKind::Link => 1,
                        BracketKind::Image | BracketKind::Footnote => 2,
                    };
                self.nodes.truncate(opener.node);
                self.nodes_replaced(opener.node, opener.node + 1 + children.len(), 1);
                let node_kind = match opener.kind {
                    BracketKind::Link => InlineKind::Link {
                        destination,
                        title,
                        kind,
                        children,
                    },
                    BracketKind::Image => InlineKind::Image {
                        destination,
                        title,
                        kind,
                        children,
                    },
                    BracketKind::Footnote => InlineKind::InlineFootnote(children),
                };
                let end = self.pos;
                self.push_node(node_kind, bracket_text_start, end);
                if opener.kind == BracketKind::Link {
                    for b in self.brackets.iter_mut() {
                        if b.kind == BracketKind::Link {
                            b.active = false;
                        }
                    }
                }
            }
        }
    }

    /// Scans `(dest "title")` at `pos`; returns the destination, title and length.
    fn scan_inline_link_tail(
        &self,
        pos: usize,
    ) -> Option<(Cow<'a, str>, Option<Cow<'a, str>>, usize)> {
        let text = &self.text[..self.end];
        let b = text.as_bytes();
        let mut i = pos + 1;
        i = skip_spnl(b, i);
        let (dest_len, dest_raw) = scan::scan_link_destination(&text[i..])?;
        i += dest_len;
        let after_dest = i;
        i = skip_spnl(b, i);
        let mut title = None;
        if i > after_dest {
            if let Some((title_len, raw)) = scan::scan_link_title(&text[i..]) {
                title = Some(Cow::Owned(scan::unescape(raw).into_owned()));
                i += title_len;
                i = skip_spnl(b, i);
            }
        }
        if b.get(i) != Some(&b')') {
            return None;
        }
        let dest = Cow::Owned(scan::unescape(dest_raw).into_owned());
        Some((dest, title, i + 1 - pos))
    }

    // ----- other constructs --------------------------------------------------

    fn angle(&mut self) -> bool {
        let start = self.pos;
        let rest = self.rest();
        let min_scheme = if self.cx.opts.quirks { 1 } else { 2 };
        if let Some((len, email)) = scan::scan_autolink(rest, min_scheme) {
            let inner = self.slice(start + 1, start + len - 1);
            let dest = if email {
                Cow::Owned(format!("mailto:{inner}"))
            } else {
                inner.clone()
            };
            let text_span = self.span(start + 1, start + len - 1);
            let kind = InlineKind::Link {
                destination: dest,
                title: None,
                kind: LinkKind::Autolink,
                children: vec![Inline {
                    kind: InlineKind::Text(inner),
                    span: text_span,
                }],
            };
            self.push_node(kind, start, start + len);
            self.pos = start + len;
            return true;
        }
        if let Some(len) = scan::scan_inline_html(rest) {
            let html = self.slice(start, start + len);
            self.push_node(InlineKind::Html(html), start, start + len);
            self.pos = start + len;
            return true;
        }
        false
    }

    fn entity(&mut self) -> bool {
        let start = self.pos;
        if let Some((len, decoded)) = scan::scan_entity(self.rest()) {
            self.push_node(InlineKind::Text(decoded), start, start + len);
            self.pos = start + len;
            return true;
        }
        if self.cx.opts.quirks {
            if let Some((len, text)) = scan::legacy_entity(&self.rest()[1..]) {
                self.push_node(
                    InlineKind::Text(Cow::Borrowed(text)),
                    start,
                    start + 1 + len,
                );
                self.pos = start + 1 + len;
                return true;
            }
        }
        false
    }

    fn tag(&mut self) -> bool {
        let start = self.pos;
        let prev = self.text[..start].chars().next_back();
        // Obsidian starts a tag after whitespace or right after another
        // inline construct (a link, code span, tag, or emphasis delimiter).
        let after_node = self.cx.opts.quirks
            && self.nodes.last().is_some_and(|n| {
                n.span.end == self.src_pos(start)
                    && (!matches!(n.kind, InlineKind::Text(_))
                        || self
                            .delim_top
                            .is_some_and(|d| self.delims[d].node + 1 == self.nodes.len()))
            });
        let prev_ok = prev.is_none_or(is_unicode_whitespace) || after_node;
        if !prev_ok {
            return false;
        }
        let rest = &self.text[start + 1..self.end];
        let len = rest
            .chars()
            .take_while(|&c| is_tag_char(c))
            .map(char::len_utf8)
            .sum::<usize>();
        let name = &rest[..len];
        if name.is_empty() || !name.chars().any(|c| !c.is_ascii_digit()) {
            return false;
        }
        let name = &self.cx.src[self.src_pos(start + 1)..self.src_pos(start + 1 + len)];
        self.push_node(InlineKind::Tag(name), start, start + 1 + len);
        self.pos = start + 1 + len;
        true
    }

    fn math(&mut self) -> bool {
        let start = self.pos;
        let rest = self.rest();
        if let Some(inner) = rest.strip_prefix("$$") {
            if let Some(i) = inner.find("$$") {
                if i > 0 {
                    let content = self.slice(start + 2, start + 2 + i);
                    self.push_node(
                        InlineKind::Math {
                            content,
                            display: true,
                        },
                        start,
                        start + 4 + i,
                    );
                    self.pos = start + 4 + i;
                    return true;
                }
            }
            return false;
        }
        let inner = &rest[1..];
        let first = inner.chars().next();
        if first.is_none_or(|c| c.is_whitespace() || c == '$') {
            return false;
        }
        let mut search = 0;
        while let Some(rel) = inner[search..].find('$') {
            let i = search + rel;
            let before = inner[..i].chars().next_back();
            let after = inner[i + 1..].chars().next();
            let escaped = before == Some('\\');
            if !escaped
                && before.is_some_and(|c| !c.is_whitespace())
                && after.is_none_or(|c| !c.is_ascii_digit())
            {
                let content = self.slice(start + 1, start + 1 + i);
                self.push_node(
                    InlineKind::Math {
                        content,
                        display: false,
                    },
                    start,
                    start + 2 + i,
                );
                self.pos = start + 2 + i;
                return true;
            }
            search = i + 1;
        }
        false
    }

    fn comment(&mut self) -> bool {
        let start = self.pos;
        let rest = self.rest();
        let Some(inner) = rest.strip_prefix("%%") else {
            return false;
        };
        // An inline comment does not cross a line break.
        let line_end = inner.find('\n').unwrap_or(inner.len());
        let Some(i) = inner[..line_end].find("%%") else {
            return false;
        };
        let content = self.slice(start + 2, start + 2 + i);
        self.push_node(InlineKind::Comment(content), start, start + 4 + i);
        self.pos = start + 4 + i;
        true
    }
}

/// Tag characters: letters, digits, `_`, `-`, `/`, and any non-ASCII
/// character that is not whitespace (Obsidian allows emoji in tags).
pub fn is_tag_char(c: char) -> bool {
    c.is_alphanumeric()
        || matches!(c, '_' | '-' | '/')
        || (!c.is_ascii() && !is_unicode_whitespace(c))
}

fn first_delim(delims: &[Delim], top: Option<usize>) -> Option<usize> {
    let mut cur = top?;
    while let Some(p) = delims[cur].prev {
        cur = p;
    }
    Some(cur)
}

fn skip_spnl(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && is_space_or_tab(b[i]) {
        i += 1;
    }
    if i < b.len() && b[i] == b'\n' {
        i += 1;
        while i < b.len() && is_space_or_tab(b[i]) {
            i += 1;
        }
    }
    i
}

/// Code span content: line endings become spaces; one leading and trailing
/// space are stripped when both are present and the content is not all spaces.
/// Unlike GFM, `\|` inside a table cell's code span stays as written (Obsidian).
fn normalize_code(inner: Cow<'_, str>) -> Cow<'_, str> {
    let mut s = inner;
    if s.contains('\n') {
        s = Cow::Owned(s.replace('\n', " "));
    }
    let bytes = s.as_bytes();
    if bytes.len() >= 2
        && bytes[0] == b' '
        && bytes[bytes.len() - 1] == b' '
        && !bytes.iter().all(|&c| c == b' ')
    {
        return match s {
            Cow::Borrowed(b) => Cow::Borrowed(&b[1..b.len() - 1]),
            Cow::Owned(o) => Cow::Owned(o[1..o.len() - 1].to_string()),
        };
    }
    s
}
