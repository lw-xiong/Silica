//! Raw HTML that a note draws.
//!
//! Obsidian renders the HTML written inside a note; Silica used to paint
//! every tag as monospace source. This pass draws the tags it can draw
//! faithfully — the inline formatting tags, their block containers, `<a
//! href>`, `<br>` and a raw `<table>` — and keeps the old source rendering
//! for everything else: scripts, comments, lists, an `<img>` mixed with
//! text, any tag it does not know. A block that meets one of those is kept
//! whole, source and all, so nothing is silently dropped or half-drawn.

use super::{InlineStyle, LinkTarget, RichBuilder};
use silica_markdown::html::{is_vault_destination, percent_decode};
use silica_markdown::Alignment;
use std::ops::Range;

/// What an open tag adds to the text inside it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Effect {
    Bold,
    Italic,
    Underline,
    Strike,
    Code,
    Kbd,
    Mark,
    /// Superscript: raised above the baseline with reduced size.
    Sup,
    /// Subscript: lowered below the baseline with reduced size.
    Sub,
    /// A heading, which the reading view can only show as bold: one run of
    /// text carries one size, and a heading inside raw HTML is inline here.
    Heading,
}

/// An open tag, and what closing it undoes.
pub(super) struct Open {
    effect: Option<Effect>,
    /// Where an `<a>`'s text started, and where it points.
    href: Option<(usize, LinkTarget)>,
}

/// The tag's effect, or `None` for a tag that is drawn but changes nothing
/// about the text (`<span>`, `<div>`, `<sub>` …).
fn effect(name: &str) -> Option<Effect> {
    Some(match name {
        "b" | "strong" => Effect::Bold,
        "i" | "em" | "cite" | "var" | "dfn" | "address" => Effect::Italic,
        "u" | "ins" => Effect::Underline,
        "s" | "del" | "strike" => Effect::Strike,
        "code" | "tt" | "samp" | "pre" => Effect::Code,
        "kbd" => Effect::Kbd,
        "mark" => Effect::Mark,
        "sup" => Effect::Sup,
        "sub" => Effect::Sub,
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => Effect::Heading,
        _ => return None,
    })
}

/// Drawn, but with no effect on the text; the tag itself disappears.
fn inert(name: &str) -> bool {
    matches!(
        name,
        "span"
            | "font"
            | "small"
            | "big"
            | "abbr"
            | "q"
            | "time"
            | "label"
            | "bdi"
            | "wbr"
            | "p"
            | "div"
            | "center"
            | "section"
            | "article"
            | "header"
            | "footer"
            | "main"
            | "aside"
            | "nav"
            | "blockquote"
            | "figure"
            | "figcaption"
            | "details"
            | "summary"
            | "dl"
            | "dt"
            | "dd"
    )
}

/// A container whose text starts on a line of its own.
fn is_block(name: &str) -> bool {
    matches!(
        name,
        "p" | "div"
            | "center"
            | "section"
            | "article"
            | "header"
            | "footer"
            | "main"
            | "aside"
            | "nav"
            | "blockquote"
            | "figure"
            | "figcaption"
            | "details"
            | "summary"
            | "dl"
            | "dt"
            | "dd"
    )
}

/// `(closing, name, attributes, bytes consumed)` for the tag at the start of
/// `src`, which must begin with `<`.
fn parse_tag(src: &str) -> Option<(bool, String, &str, usize)> {
    let after = src.strip_prefix('<')?;
    let (closing, rest) = match after.strip_prefix('/') {
        Some(r) => (true, r),
        None => (false, after),
    };
    let head = src.len() - rest.len();
    let end =
        rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == ':' || c == '_'))?;
    if end == 0 {
        return None;
    }
    let name = rest[..end].to_ascii_lowercase();
    let mut quote: Option<char> = None;
    let mut tag_len = None;
    for (at, c) in rest.char_indices().skip(end) {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '"' || c == '\'' => quote = Some(c),
            None if c == '>' => {
                tag_len = Some(at + 1);
                break;
            }
            None => {}
        }
    }
    let tag_len = tag_len?;
    let attrs = rest[end..tag_len - 1].trim_end();
    let attrs = match attrs.strip_suffix('/') {
        Some(a) => a.trim_end(),
        None => attrs,
    };
    Some((closing, name, attrs, head + tag_len))
}

/// The value of attribute `want`, or `None` when it is absent.
fn attr<'a>(attrs: &'a str, want: &str) -> Option<&'a str> {
    let name_end = |c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_');
    let mut rest = attrs;
    loop {
        rest = rest.trim_start_matches(name_end);
        if rest.is_empty() {
            return None;
        }
        let end = rest.find(name_end).unwrap_or(rest.len());
        let name = &rest[..end];
        let mut tail = rest[end..].trim_start();
        let mut value = "";
        if let Some(after) = tail.strip_prefix('=') {
            let after = after.trim_start();
            match after.chars().next() {
                Some(q @ ('"' | '\'')) => {
                    let close = after[1..].find(q)?;
                    value = &after[1..1 + close];
                    tail = &after[1 + close + q.len_utf8()..];
                }
                _ => {
                    let e = after.find(char::is_whitespace).unwrap_or(after.len());
                    value = &after[..e];
                    tail = &after[e..];
                }
            }
        }
        if name.eq_ignore_ascii_case(want) {
            return Some(value);
        }
        if tail.is_empty() {
            return None;
        }
        rest = tail;
    }
}

impl RichBuilder {
    /// The style of the run being built: the markdown style plus every open
    /// HTML tag.
    pub(super) fn html_style(&self) -> InlineStyle {
        let mut style = self.style;
        for open in &self.html_stack {
            if open.href.is_some() {
                // * An `<a>`'s text is what the link style paints.
                style.link = true;
            }
            match open.effect {
                Some(Effect::Bold | Effect::Heading) => style.bold = true,
                Some(Effect::Italic) => style.italic = true,
                Some(Effect::Underline) => style.underline = true,
                Some(Effect::Strike) => style.strike = true,
                Some(Effect::Code) => style.code = true,
                Some(Effect::Kbd) => style.kbd = true,
                Some(Effect::Mark) => style.highlight = true,
                Some(Effect::Sup) => style.sup = true,
                Some(Effect::Sub) => style.sub = true,
                None => {}
            }
        }
        style
    }

    /// Ends the current line if it is not already ended.
    fn html_break(&mut self) {
        if !self.text.is_empty() && !self.text.ends_with('\n') {
            self.push("\n");
        }
    }

    /// Opens `<a href>`: the text from here to `</a>` becomes clickable.
    fn html_open_anchor(&mut self, href: Option<&str>) {
        let start = self.text.len();
        let href = href.map(|h| {
            if is_vault_destination(h) {
                LinkTarget::Internal(percent_decode(h).into_owned())
            } else {
                LinkTarget::External(h.to_string())
            }
        });
        self.html_stack.push(Open {
            effect: None,
            href: href.map(|t| (start, t)),
        });
    }

    fn html_close_anchor(&mut self) {
        let Some(at) = self.html_stack.iter().rposition(|o| o.href.is_some()) else {
            return;
        };
        let open = self.html_stack.remove(at);
        if let Some((start, target)) = open.href {
            self.link(start, target);
        }
    }

    /// Opens a tag that only has to be remembered so its close can be
    /// matched.
    fn html_open_inert(&mut self, block: bool) {
        if block {
            self.html_break();
        }
        self.html_stack.push(Open {
            effect: None,
            href: None,
        });
    }

    /// Closes the innermost open tag with this effect. A close tag with no
    /// open counterpart (`</b>` on its own) is dropped.
    fn html_close_effect(&mut self, want: Effect) {
        let Some(at) = self.html_stack.iter().rposition(|o| o.effect == Some(want)) else {
            return;
        };
        self.html_stack.remove(at);
    }

    /// Closes the innermost inert tag.
    fn html_close_inert(&mut self, block: bool) {
        let Some(at) = self
            .html_stack
            .iter()
            .rposition(|o| o.effect.is_none() && o.href.is_none())
        else {
            return;
        };
        self.html_stack.remove(at);
        if block {
            self.html_break();
        }
    }
}

/// Draws one tag at the start of `src`. `None` when the tag is one this pass
/// does not draw, and the caller keeps its source.
fn draw_tag(acc: &mut RichBuilder, src: &str) -> Option<usize> {
    let (closing, name, attrs, len) = parse_tag(src)?;
    // * A self-closing `<br/>` and a bare `<br>` are the same break.
    if name == "br" || name == "hr" {
        acc.html_break();
        return Some(len);
    }
    if let Some(effect) = effect(&name) {
        if closing {
            acc.html_close_effect(effect);
        } else {
            acc.html_stack.push(Open {
                effect: Some(effect),
                href: None,
            });
        }
        return Some(len);
    }
    if name == "a" {
        if closing {
            acc.html_close_anchor();
        } else {
            acc.html_open_anchor(attr(attrs, "href"));
        }
        return Some(len);
    }
    if inert(&name) {
        if closing {
            acc.html_close_inert(is_block(&name));
        } else {
            acc.html_open_inert(is_block(&name));
        }
        return Some(len);
    }
    None
}

/// How much of an undrawable tag to keep as source: to its first `>`.
fn source_len(src: &str) -> usize {
    src.find('>').map(|at| at + 1).unwrap_or(1)
}

/// Inline HTML inside a paragraph. A tag the pass does not draw keeps its
/// own source and the text around it is drawn, so one unknown tag does not
/// cost the rest of the line.
pub(crate) fn inline(acc: &mut RichBuilder, src: &str) {
    let mut at = 0;
    while at < src.len() {
        let Some(lt) = src[at..].find('<').map(|i| at + i) else {
            acc.push(&src[at..]);
            return;
        };
        acc.push(&src[at..lt]);
        match draw_tag(acc, &src[lt..]) {
            Some(len) => at = lt + len,
            None => {
                let len = source_len(&src[lt..]);
                acc.push_html_source(&src[lt..lt + len]);
                at = lt + len;
            }
        }
    }
}

/// A whole HTML block. `None` when it holds a tag the pass does not draw:
/// a block is drawn all at once or kept as source, never half of each.
pub(crate) fn block(src: &str, id: usize) -> Option<super::RichText> {
    let mut acc = RichBuilder::default();
    let mut at = 0;
    while at < src.len() {
        let Some(lt) = src[at..].find('<').map(|i| at + i) else {
            acc.push(&src[at..]);
            break;
        };
        acc.push(&src[at..lt]);
        let len = draw_tag(&mut acc, &src[lt..])?;
        at = lt + len;
    }
    Some(acc.finish(id))
}

// ----- a raw table -----------------------------------------------------------

/// One cell of a raw `<table>`: its inner HTML and whether it was a `<th>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct HtmlCell {
    pub html: String,
    /// A `<th>` outside the header row, drawn bold.
    pub head: bool,
}

/// A raw `<table>` as rows of cells' inner HTML.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct HtmlTable {
    /// The `<thead>` row, or a first row of nothing but `<th>`; empty when
    /// the table has neither.
    pub header: Vec<HtmlCell>,
    pub rows: Vec<Vec<HtmlCell>>,
    /// Per column, from the `align` attribute of the first row's cells.
    pub alignments: Vec<Alignment>,
}

/// The attributes of a tag's `attrs` text, as `(name, value)` pairs. A bare
/// name carries an empty value, which is how `<td nowrap>` reads.
fn attrs_of(attrs: &str) -> Vec<(String, String)> {
    let name_end = |c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_');
    let mut out = Vec::new();
    let mut rest = attrs;
    while !rest.is_empty() {
        rest = rest.trim_start_matches(name_end);
        if rest.is_empty() {
            break;
        }
        let end = rest.find(name_end).unwrap_or(rest.len());
        let name = rest[..end].to_ascii_lowercase();
        let mut tail = rest[end..].trim_start();
        let mut value = String::new();
        if let Some(after) = tail.strip_prefix('=') {
            let after = after.trim_start();
            match after.chars().next() {
                Some(q @ ('"' | '\'')) => {
                    let close = after[1..].find(q).unwrap_or(after.len() - 1);
                    value = after[1..1 + close].to_string();
                    tail = &after[1 + close + q.len_utf8()..];
                }
                _ => {
                    let e = after.find(char::is_whitespace).unwrap_or(after.len());
                    value = after[..e].to_string();
                    tail = &after[e..];
                }
            }
        }
        out.push((name, value));
        rest = tail;
    }
    out
}

/// The cells of a raw `<table>` block, or `None` for one this grid cannot
/// hold: a table nested in a cell, or one with no cell at all. Markdown
/// between a cell's tags is not parsed — a cell shows what its HTML shows.
pub(super) fn parse_table(html: &str) -> Option<HtmlTable> {
    let mut table = HtmlTable::default();
    let mut row: Option<(Vec<HtmlCell>, bool)> = None;
    let mut cell: Option<(String, bool, Alignment)> = None;
    let mut in_head = false;
    let mut first_alignments: Option<Vec<Alignment>> = None;
    let mut depth = 0usize;
    let finish_cell = |cell: &mut Option<(String, bool, Alignment)>,
                       row: &mut Option<(Vec<HtmlCell>, bool)>,
                       aligns: &mut Vec<Alignment>| {
        if let Some((html, head, align)) = cell.take() {
            let (cells, _) = row.get_or_insert_with(|| (Vec::new(), false));
            cells.push(HtmlCell { html, head });
            aligns.push(align);
        }
    };
    let mut row_aligns: Vec<Alignment> = Vec::new();
    let finish_row = |row: &mut Option<(Vec<HtmlCell>, bool)>,
                      row_aligns: &mut Vec<Alignment>,
                      first: &mut Option<Vec<Alignment>>,
                      table: &mut HtmlTable| {
        let aligns = std::mem::take(row_aligns);
        let Some((cells, head_section)) = row.take() else {
            return;
        };
        if cells.is_empty() {
            return;
        }
        if first.is_none() {
            *first = Some(aligns);
        }
        let all_th = cells.iter().all(|c| c.head);
        if table.header.is_empty() && table.rows.is_empty() && (head_section || all_th) {
            table.header = cells
                .into_iter()
                .map(|c| HtmlCell { head: false, ..c })
                .collect();
        } else {
            table.rows.push(cells);
        }
    };
    let mut rest = html.trim();
    while !rest.is_empty() {
        let (token, text) = match rest.find('<') {
            Some(0) => {
                let end = rest.find('>').map(|i| i + 1).unwrap_or(rest.len());
                let token = &rest[..end];
                rest = &rest[end..];
                (Some(token), "")
            }
            Some(at) => {
                let text = &rest[..at];
                rest = &rest[at..];
                (None, text)
            }
            None => {
                let text = rest;
                rest = "";
                (None, text)
            }
        };
        let Some(token) = token else {
            if let Some((inner, ..)) = cell.as_mut() {
                inner.push_str(text);
            }
            continue;
        };
        // * A tag the pass does not recognise as one of its own — a comment,
        //   a doctype — is text inside whatever cell is open.
        let Some((closing, name, attrs, _)) = parse_tag(token) else {
            if let Some((inner, ..)) = cell.as_mut() {
                inner.push_str(token);
            }
            continue;
        };
        let name = name.to_ascii_lowercase();
        match (name.as_str(), closing) {
            ("table", false) => {
                if depth > 0 {
                    return None;
                }
                depth += 1;
            }
            ("table", true) => {
                finish_cell(&mut cell, &mut row, &mut row_aligns);
                finish_row(&mut row, &mut row_aligns, &mut first_alignments, &mut table);
                break;
            }
            ("thead", open_or_close) => {
                finish_cell(&mut cell, &mut row, &mut row_aligns);
                finish_row(&mut row, &mut row_aligns, &mut first_alignments, &mut table);
                in_head = !open_or_close;
            }
            ("tbody" | "tfoot", _) => {
                finish_cell(&mut cell, &mut row, &mut row_aligns);
                finish_row(&mut row, &mut row_aligns, &mut first_alignments, &mut table);
                in_head = false;
            }
            ("tr", false) => {
                finish_cell(&mut cell, &mut row, &mut row_aligns);
                finish_row(&mut row, &mut row_aligns, &mut first_alignments, &mut table);
                row = Some((Vec::new(), in_head));
            }
            ("tr", true) => {
                finish_cell(&mut cell, &mut row, &mut row_aligns);
                finish_row(&mut row, &mut row_aligns, &mut first_alignments, &mut table);
            }
            ("td" | "th", false) => {
                finish_cell(&mut cell, &mut row, &mut row_aligns);
                if row.is_none() {
                    row = Some((Vec::new(), in_head));
                }
                let align = attrs_of(attrs)
                    .into_iter()
                    .find(|(n, _)| n == "align")
                    .map(|(_, v)| match v.to_ascii_lowercase().as_str() {
                        "left" => Alignment::Left,
                        "center" => Alignment::Center,
                        "right" => Alignment::Right,
                        _ => Alignment::None,
                    })
                    .unwrap_or(Alignment::None);
                cell = Some((String::new(), name == "th", align));
            }
            ("td" | "th", true) => finish_cell(&mut cell, &mut row, &mut row_aligns),
            _ => {
                if let Some((inner, ..)) = cell.as_mut() {
                    inner.push_str(token);
                }
            }
        }
    }
    finish_cell(&mut cell, &mut row, &mut row_aligns);
    finish_row(&mut row, &mut row_aligns, &mut first_alignments, &mut table);
    if table.header.is_empty() && table.rows.is_empty() {
        return None;
    }
    table.alignments = first_alignments.unwrap_or_default();
    Some(table)
}

/// A cell's inner HTML as the rich text it draws: the tags through the
/// inline overlay, the text between them literal.
pub(super) fn rich_cell(html: &str, id: usize) -> super::RichText {
    let mut acc = RichBuilder::default();
    inline(&mut acc, html);
    acc.finish(id)
}

/// `runs` over a text of `len` bytes with bold laid over every byte, the
/// gaps between them included.
pub(super) fn embolden(
    runs: Vec<(Range<usize>, InlineStyle)>,
    len: usize,
) -> Vec<(Range<usize>, InlineStyle)> {
    let bold = InlineStyle {
        bold: true,
        ..InlineStyle::default()
    };
    let mut out = Vec::new();
    let mut at = 0;
    for (range, mut style) in runs {
        if range.start > at {
            out.push((at..range.start, bold));
        }
        style.bold = true;
        at = range.end;
        out.push((range, style));
    }
    if at < len {
        out.push((at..len, bold));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{block, inline};
    use crate::reading::model::{InlineStyle, LinkTarget, RichBuilder};

    /// The runs of a drawn block: its plain text, the styled spans, and the
    /// links it found.
    type Drawn = Option<(String, Vec<(String, InlineStyle)>, Vec<LinkTarget>)>;

    fn drawn(src: &str) -> Drawn {
        let rich = block(src, 1)?;
        let runs = rich
            .runs
            .iter()
            .map(|(r, s)| (rich.text[r.clone()].to_string(), *s))
            .collect();
        Some((
            rich.text,
            runs,
            rich.links.iter().map(|(_, t)| t.clone()).collect(),
        ))
    }

    #[test]
    fn formatting_tags_style_their_text() {
        let (text, runs, _) = drawn("<b>bold</b> and <i>it</i> and <code>c</code>").expect("drawn");
        assert_eq!(text, "bold and it and c");
        assert!(runs.iter().any(|(t, s)| t == "bold" && s.bold));
        assert!(runs.iter().any(|(t, s)| t == "it" && s.italic));
        assert!(runs.iter().any(|(t, s)| t == "c" && s.code));
    }

    /// A tag opened in one span of inline HTML and closed in a later one
    /// still styles everything between them.
    #[test]
    fn a_style_survives_across_inline_spans() {
        let mut acc = RichBuilder::default();
        inline(&mut acc, "<b>");
        inline(&mut acc, "middle");
        inline(&mut acc, "</b>after");
        let rich = acc.finish(1);
        assert_eq!(rich.text, "middleafter");
        assert_eq!(rich.runs.len(), 1, "one run: {:?}", rich.runs);
        assert_eq!(rich.text[rich.runs[0].0.clone()].to_string(), "middle");
        assert!(rich.runs[0].1.bold);
    }

    #[test]
    fn block_containers_and_brs_break_lines() {
        let (text, _, _) = drawn("<div>one</div><div>two</div>").expect("drawn");
        assert_eq!(text, "one\ntwo\n");
        let (text, _, _) = drawn("a<br>b<br/>c").expect("drawn");
        assert_eq!(text, "a\nb\nc");
    }

    #[test]
    fn anchors_carry_their_targets() {
        let (text, runs, links) = drawn("<a href=\"https://e.com/x\">link</a>").expect("drawn");
        assert_eq!(text, "link");
        assert!(runs[0].1.link || links.len() == 1);
        assert!(matches!(&links[0], LinkTarget::External(h) if h == "https://e.com/x"));

        // * A vault-relative destination is resolved like a markdown link.
        let (_, _, links) = drawn("<a href=\"Note.md\">n</a>").expect("drawn");
        assert!(matches!(&links[0], LinkTarget::Internal(p) if p == "Note.md"));
    }

    #[test]
    fn an_undrawable_tag_keeps_the_whole_block_as_source() {
        for src in [
            "<script>window.x = 1</script>",
            "<!-- a comment -->",
            "<table><tr><td>a</td></tr></table>",
            "<div>mixed</div>\n<img src=\"pic.png\">",
            "<img src=\"pic.png\">",
            "<ul><li>a</li></ul>",
        ] {
            assert!(drawn(src).is_none(), "{src:?} was drawn");
        }
    }

    /// Inline HTML is per tag: an unknown one keeps its own source and does
    /// not cost the text around it.
    #[test]
    fn an_undrawable_inline_tag_keeps_only_itself() {
        let mut acc = RichBuilder::default();
        inline(&mut acc, "before<marquee>after");
        let rich = acc.finish(1);
        assert_eq!(rich.text, "before<marquee>after");
        let html: Vec<_> = rich
            .runs
            .iter()
            .filter(|(_, s)| s.html)
            .map(|(r, _)| rich.text[r.clone()].to_string())
            .collect();
        assert_eq!(html, vec!["<marquee>".to_string()]);
    }

    #[test]
    fn kbd_boxes_and_sup_sub_carry_their_flags() {
        let mut acc = RichBuilder::default();
        inline(&mut acc, "<kbd>x</kbd>2<sup>nd</sup> H<sub>2</sub>O");
        let rich = acc.finish(1);
        assert_eq!(rich.text, "x2nd H2O");
        let styled = |flag: fn(&InlineStyle) -> bool| {
            rich.runs
                .iter()
                .filter(|(_, s)| flag(s))
                .map(|(r, _)| rich.text[r.clone()].to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(styled(|s| s.kbd), vec!["x"]);
        assert_eq!(styled(|s| s.sup), vec!["nd"]);
        assert_eq!(styled(|s| s.sub), vec!["2"]);
        // * Drawn tags are text, not muted source.
        assert!(rich.runs.iter().all(|(_, s)| !s.html));
    }

    #[test]
    fn attributes_are_read_past_quotes() {
        let (text, _, links) =
            drawn("<a title=\"a > b\" href=\"https://e.com\">t</a>").expect("drawn");
        assert_eq!(text, "t");
        assert!(matches!(&links[0], LinkTarget::External(h) if h == "https://e.com"));
    }
}
