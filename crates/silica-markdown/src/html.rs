//! HTML rendering of a [`Document`].
//!
//! With `obsidian_attributes` off the output matches the CommonMark reference
//! implementation byte for byte on CommonMark input, which is what the spec
//! conformance tests compare against. With it on, the markup follows what
//! Obsidian's reading view produces (observed with the corpus under
//! `tests/corpus`): `internal-link` anchors with `data-href`, `tag` anchors,
//! `internal-embed` spans, callout and task-list structure, `data-heading`,
//! footnote sections, and soft line breaks rendered as `<br>` because
//! Obsidian's "strict line breaks" setting is off by default.

use crate::ast::*;
use std::borrow::Cow;
use std::fmt::Write;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HtmlOptions {
    /// Emit Obsidian's reading-view classes and attributes.
    pub obsidian_attributes: bool,
    /// Render a single newline as a soft break (CommonMark) rather than a
    /// line break (Obsidian's default).
    pub strict_line_breaks: bool,
}

impl Default for HtmlOptions {
    fn default() -> HtmlOptions {
        HtmlOptions {
            obsidian_attributes: true,
            strict_line_breaks: false,
        }
    }
}

impl HtmlOptions {
    /// Plain CommonMark output, as the spec tests expect.
    pub const fn commonmark() -> HtmlOptions {
        HtmlOptions {
            obsidian_attributes: false,
            strict_line_breaks: true,
        }
    }
}

/// Renders `doc` with the default (Obsidian) options.
pub fn render_html(doc: &Document<'_>) -> String {
    render_html_with(doc, &HtmlOptions::default())
}

pub fn render_html_with(doc: &Document<'_>, opts: &HtmlOptions) -> String {
    let mut r = Renderer {
        out: String::new(),
        src: doc.source,
        opts,
        footnote_numbers: Vec::new(),
        inline_footnotes: Vec::new(),
        definitions: Vec::new(),
    };
    collect_footnote_definitions(&doc.blocks, &mut r.definitions);
    r.blocks(&doc.blocks, false);
    r.footnote_section();
    r.out
}

fn collect_footnote_definitions<'b, 'a>(
    blocks: &'b [Block<'a>],
    out: &mut Vec<(&'a str, &'b [Block<'a>])>,
) {
    walk_blocks(blocks, &mut |b| {
        if let BlockKind::FootnoteDefinition { label, children } = &b.kind {
            if !out.iter().any(|(l, _)| l == label) {
                out.push((label, children));
            }
        }
    });
}

struct Renderer<'r, 'a> {
    out: String,
    src: &'a str,
    opts: &'r HtmlOptions,
    /// Labels in order of first reference; index + 1 is the footnote number.
    footnote_numbers: Vec<String>,
    /// Inline footnotes keyed by their generated label.
    inline_footnotes: Vec<(String, &'r [Inline<'a>])>,
    definitions: Vec<(&'a str, &'r [Block<'a>])>,
}

pub(crate) fn escape_html(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
}

/// Percent-encodes a URL the way the CommonMark reference implementation does.
pub(crate) fn escape_href(s: &str, out: &mut String) {
    for &b in s.as_bytes() {
        match b {
            b'&' => out.push_str("&amp;"),
            b'\'' => out.push_str("&#x27;"),
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' => out.push(b as char),
            b'!' | b'#' | b'$' | b'%' | b'(' | b')' | b'*' | b'+' | b',' | b'-' | b'.' | b'/'
            | b':' | b';' | b'=' | b'?' | b'@' | b'_' | b'~' => out.push(b as char),
            _ => {
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
}

/// External `href`: a browser-style serialization that percent-encodes
/// quotes and spaces and leaves everything else as written.
fn escape_external_href(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '"' => out.push_str("%22"),
            ' ' => out.push_str("%20"),
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
}

/// Whether this inline renders as an `<a>`, and so cannot sit inside one.
///
/// A nested markdown link is deliberately not counted: Obsidian resolves that
/// case in the parser instead, keeping the outer link and leaving the inner one
/// as literal text, which `silica-markdown` does not do yet (see the link
/// nesting entry in `docs/silica/Specification.md`).
pub fn renders_as_anchor(inline: &Inline<'_>) -> bool {
    match &inline.kind {
        InlineKind::Wikilink { embed, .. } => !*embed,
        InlineKind::Tag(_) => true,
        _ => false,
    }
}

/// Whether `dest` starts with a URL scheme such as `https:` or `mailto:`.
pub fn has_scheme(dest: &str) -> bool {
    let b = dest.as_bytes();
    if !b.first().is_some_and(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    let scheme_len = b
        .iter()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'.' | b'-'))
        .count();
    scheme_len >= 2 && b.get(scheme_len) == Some(&b':')
}

/// Whether a markdown link destination points into the vault: no URL scheme,
/// not root-relative, not empty.
pub fn is_vault_destination(dest: &str) -> bool {
    !dest.is_empty() && !dest.starts_with('/') && !has_scheme(dest)
}

/// Decodes `%XX` escapes.
pub fn percent_decode(s: &str) -> Cow<'_, str> {
    if !s.contains('%') {
        return Cow::Borrowed(s);
    }
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    let hex = |c: Option<&u8>| (*c? as char).to_digit(16).map(|d| d as u8);
    while i < b.len() {
        if b[i] == b'%' {
            if let (Some(h), Some(l)) = (hex(b.get(i + 1)), hex(b.get(i + 2))) {
                out.push(h << 4 | l);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    match String::from_utf8(out) {
        Ok(s) => Cow::Owned(s),
        Err(e) => Cow::Owned(String::from_utf8_lossy(e.as_bytes()).into_owned()),
    }
}

impl<'r, 'a> Renderer<'r, 'a> {
    fn cr(&mut self) {
        if !self.out.is_empty() && !self.out.ends_with('\n') {
            self.out.push('\n');
        }
    }

    fn obsidian(&self) -> bool {
        self.opts.obsidian_attributes
    }

    /// Source text covered by a run of inlines, as written.
    fn raw(&self, inlines: &[Inline<'a>]) -> &'a str {
        match (inlines.first(), inlines.last()) {
            (Some(f), Some(l)) => &self.src[f.span.start..l.span.end],
            _ => "",
        }
    }

    fn blocks(&mut self, blocks: &'r [Block<'a>], tight: bool) {
        for b in blocks {
            self.block(b, tight);
        }
    }

    fn block(&mut self, block: &'r Block<'a>, tight: bool) {
        match &block.kind {
            BlockKind::Yaml { .. } => {}
            BlockKind::Paragraph { content } => {
                if tight {
                    self.inlines(content);
                } else {
                    self.cr();
                    self.out.push_str("<p>");
                    self.inlines(content);
                    self.out.push_str("</p>\n");
                }
            }
            BlockKind::Heading { level, content, .. } => {
                self.cr();
                if self.obsidian() {
                    let _ = write!(self.out, "<h{level} data-heading=\"");
                    escape_html(self.raw(content), &mut self.out);
                    self.out.push_str("\">");
                } else {
                    let _ = write!(self.out, "<h{level}>");
                }
                self.inlines(content);
                let _ = writeln!(self.out, "</h{level}>");
            }
            BlockKind::ThematicBreak => {
                self.cr();
                self.out.push_str("<hr />\n");
            }
            BlockKind::CodeBlock { info, content, .. } => {
                self.cr();
                let lang = info
                    .as_deref()
                    .and_then(|i| i.split_whitespace().next())
                    .unwrap_or("");
                // Obsidian resolves the info string's first word to a
                // highlighter language: the token is lowercased, and a token
                // holding anything but `[A-Za-z0-9_-]` resolves to `none` --
                // which is what `c#`, `f#`, `C++` and `.net` all do -- with the
                // raw token kept on the `<code>` beside `language-none`.
                // CommonMark has neither rule, and puts the token on the
                // `<code>` as written. Observed against Obsidian 1.14.4
                // (tools/probe-gen, 2026-10-29).
                let obsidian = self.obsidian();
                let resolves = !lang.is_empty()
                    && lang
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
                let lowered = lang.to_ascii_lowercase();
                self.out.push_str("<pre");
                if obsidian && !lang.is_empty() {
                    self.out.push_str(" class=\"language-");
                    escape_html(if resolves { &lowered } else { "none" }, &mut self.out);
                    self.out.push('"');
                }
                self.out.push_str("><code");
                if !lang.is_empty() {
                    self.out.push_str(" class=\"language-");
                    match (obsidian, resolves) {
                        (true, true) => escape_html(&lowered, &mut self.out),
                        (true, false) => {
                            escape_html(lang, &mut self.out);
                            self.out.push_str(" language-none");
                        }
                        (false, _) => escape_html(lang, &mut self.out),
                    }
                    self.out.push('"');
                }
                self.out.push('>');
                escape_html(content, &mut self.out);
                self.out.push_str("</code></pre>\n");
            }
            BlockKind::HtmlBlock { content } => {
                self.cr();
                self.out.push_str(content);
                self.cr();
            }
            BlockKind::BlockQuote { children } => {
                self.cr();
                self.out.push_str("<blockquote>\n");
                self.blocks(children, false);
                self.cr();
                self.out.push_str("</blockquote>\n");
            }
            BlockKind::Callout {
                callout_type,
                fold,
                title,
                children,
            } => {
                self.cr();
                self.out
                    .push_str("<div data-callout-metadata=\"\" data-callout-fold=\"");
                match fold {
                    Some(Fold::Collapsed) => self.out.push('-'),
                    Some(Fold::Expanded) => self.out.push('+'),
                    None => {}
                }
                self.out.push_str("\" data-callout=\"");
                escape_html(&callout_type.to_lowercase(), &mut self.out);
                self.out.push_str("\" class=\"callout");
                if fold.is_some() {
                    self.out.push_str(" is-collapsible");
                }
                if *fold == Some(Fold::Collapsed) {
                    self.out.push_str(" is-collapsed");
                }
                self.out.push_str(
                    "\"><div class=\"callout-title\"><div class=\"callout-title-inner\">",
                );
                if title.is_empty() {
                    escape_html(&crate::callout_default_title(callout_type), &mut self.out);
                } else {
                    // Tight, so the usual one-paragraph title renders as bare
                    // text while a heading or a list in it renders as itself.
                    self.blocks(title, true);
                }
                self.out.push_str("</div></div>");
                if !children.is_empty() {
                    self.out.push_str("<div class=\"callout-content\">\n");
                    self.blocks(children, false);
                    self.cr();
                    self.out.push_str("</div>");
                }
                self.out.push_str("</div>\n");
            }
            BlockKind::List { kind, tight, items } => {
                self.cr();
                let task_list = self.obsidian() && items.iter().any(|i| i.task.is_some());
                let class = if task_list {
                    " class=\"contains-task-list\""
                } else {
                    ""
                };
                match kind {
                    ListKind::Bullet { .. } => {
                        let _ = writeln!(self.out, "<ul{class}>");
                    }
                    ListKind::Ordered { start, .. } if *start == 1 => {
                        let _ = writeln!(self.out, "<ol{class}>");
                    }
                    ListKind::Ordered { start, .. } => {
                        let _ = writeln!(self.out, "<ol start=\"{start}\"{class}>");
                    }
                }
                for item in items {
                    self.cr();
                    let mut children: &[Block<'a>] = &item.children;
                    match item.task {
                        Some(c) if self.obsidian() => {
                            self.out.push_str("<li data-task=\"");
                            if c != ' ' {
                                escape_html(&c.to_string(), &mut self.out);
                            }
                            self.out.push_str("\" class=\"task-list-item");
                            if c != ' ' {
                                self.out.push_str(" is-checked");
                            }
                            self.out.push_str("\">");
                            // In a loose list the checkbox sits inside the first paragraph.
                            let loose_paragraph = !*tight
                                && matches!(
                                    children.first(),
                                    Some(Block {
                                        kind: BlockKind::Paragraph { .. },
                                        ..
                                    })
                                );
                            if loose_paragraph {
                                self.out.push_str("<p>");
                            }
                            self.out.push_str("<input");
                            if c != ' ' {
                                self.out.push_str(" checked=\"\"");
                            }
                            self.out
                                .push_str(" type=\"checkbox\" class=\"task-list-item-checkbox\">");
                            if loose_paragraph {
                                if let Some(Block {
                                    kind: BlockKind::Paragraph { content },
                                    ..
                                }) = children.first()
                                {
                                    self.inlines(content);
                                }
                                self.out.push_str("</p>\n");
                                children = &children[1..];
                            }
                        }
                        Some(c) => {
                            self.out.push_str("<li><input type=\"checkbox\"");
                            if c != ' ' {
                                self.out.push_str(" checked=\"\"");
                            }
                            self.out.push_str(" disabled=\"\" /> ");
                        }
                        None => self.out.push_str("<li>"),
                    }
                    self.blocks(children, *tight);
                    self.out.push_str("</li>\n");
                }
                match kind {
                    ListKind::Bullet { .. } => self.out.push_str("</ul>\n"),
                    ListKind::Ordered { .. } => self.out.push_str("</ol>\n"),
                }
            }
            BlockKind::Table {
                alignments,
                header,
                rows,
            } => {
                self.cr();
                self.out.push_str("<table>\n<thead>\n<tr>\n");
                for (i, cell) in header.iter().enumerate() {
                    self.out.push_str("<th");
                    self.alignment(alignments[i]);
                    self.out.push('>');
                    self.inlines(&cell.content);
                    self.out.push_str("</th>\n");
                }
                self.out.push_str("</tr>\n</thead>\n");
                if !rows.is_empty() {
                    self.out.push_str("<tbody>\n");
                    for row in rows {
                        self.out.push_str("<tr>\n");
                        for (i, cell) in row.iter().enumerate() {
                            self.out.push_str("<td");
                            self.alignment(alignments[i]);
                            self.out.push('>');
                            self.inlines(&cell.content);
                            self.out.push_str("</td>\n");
                        }
                        self.out.push_str("</tr>\n");
                    }
                    self.out.push_str("</tbody>\n");
                }
                self.out.push_str("</table>\n");
            }
            BlockKind::Math { content } => {
                self.cr();
                self.out.push_str("<div class=\"math math-block\">");
                escape_html(content.trim_matches('\n'), &mut self.out);
                self.out.push_str("</div>\n");
            }
            BlockKind::Comment { .. } => {}
            BlockKind::FootnoteDefinition { .. } => {}
        }
    }

    fn alignment(&mut self, a: Alignment) {
        match a {
            Alignment::None => {}
            Alignment::Left => self.out.push_str(" align=\"left\""),
            Alignment::Center => self.out.push_str(" align=\"center\""),
            Alignment::Right => self.out.push_str(" align=\"right\""),
        }
    }

    fn inlines(&mut self, inlines: &'r [Inline<'a>]) {
        for i in inlines {
            self.inline(i);
        }
    }

    fn inline(&mut self, inline: &'r Inline<'a>) {
        match &inline.kind {
            InlineKind::Text(t) => escape_html(t, &mut self.out),
            InlineKind::SoftBreak => {
                if self.opts.strict_line_breaks {
                    self.out.push('\n');
                } else {
                    self.out.push_str("<br>\n");
                }
            }
            InlineKind::HardBreak => self.out.push_str("<br />\n"),
            InlineKind::Code(c) => {
                self.out.push_str("<code>");
                escape_html(c, &mut self.out);
                self.out.push_str("</code>");
            }
            InlineKind::Html(h) => self.out.push_str(h),
            InlineKind::Emphasis(c) => self.wrap("em", c),
            InlineKind::Strong(c) => self.wrap("strong", c),
            InlineKind::Strikethrough(c) => self.wrap("del", c),
            InlineKind::Highlight { color, children } => {
                if self.obsidian() {
                    self.out.push_str("<mark data-highlight=\"");
                    self.out
                        .push_str(color.map(HighlightColor::name).unwrap_or(""));
                    self.out.push_str("\">");
                    self.inlines(children);
                    self.out.push_str("</mark>");
                } else {
                    self.wrap("mark", children);
                }
            }
            InlineKind::Link {
                destination,
                title,
                kind,
                children,
            } => self.link(destination, title.as_deref(), *kind, children),
            InlineKind::Image {
                destination,
                title,
                kind,
                children,
            } => {
                // Only an inline `![alt](src)` without a scheme is a vault embed;
                // reference-style images render as `<img>` whatever the path.
                if self.obsidian() && *kind == LinkKind::Inline && !has_scheme(destination) {
                    let alt = self.raw(children);
                    let alt = alt.split('|').next().unwrap_or("").trim_end();
                    self.out.push_str("<span alt=\"");
                    escape_html(alt, &mut self.out);
                    self.out.push_str("\" src=\"");
                    escape_html(&percent_decode(destination), &mut self.out);
                    self.out.push_str("\" class=\"internal-embed\"></span>");
                    return;
                }
                self.out.push_str("<img src=\"");
                escape_href(destination, &mut self.out);
                self.out.push_str("\" alt=\"");
                escape_html(&plain_text(children), &mut self.out);
                self.out.push('"');
                if let Some(t) = title {
                    self.out.push_str(" title=\"");
                    escape_html(t, &mut self.out);
                    self.out.push('"');
                }
                if self.obsidian() {
                    self.out.push_str(" referrerpolicy=\"no-referrer\">");
                } else {
                    self.out.push_str(" />");
                }
            }
            InlineKind::Wikilink { body, embed } => {
                let link = crate::parse_linktext(body);
                let target = crate::link_target(body);
                if *embed {
                    // The alt is the first alias part that is not a size hint
                    // (`200`, `200x100`), else the display text.
                    let alt = match link
                        .alias
                        .and_then(|a| a.split('|').map(str::trim).find(|p| !is_size_hint(p)))
                    {
                        Some(a) => Cow::Borrowed(a),
                        None => Cow::Owned(wikilink_display(&link)),
                    };
                    self.out.push_str("<span alt=\"");
                    escape_html(&alt, &mut self.out);
                    self.out.push_str("\" src=\"");
                    escape_html(target, &mut self.out);
                    self.out.push_str("\" class=\"internal-embed\"></span>");
                } else {
                    self.out.push_str("<a ");
                    if link.alias.is_some() {
                        self.out
                            .push_str("data-tooltip-position=\"top\" aria-label=\"");
                        escape_html(target, &mut self.out);
                        self.out.push_str("\" ");
                    }
                    self.out.push_str("data-href=\"");
                    escape_html(target, &mut self.out);
                    self.out.push_str("\" href=\"");
                    escape_html(target, &mut self.out);
                    self.out.push_str(
                        "\" class=\"internal-link\" target=\"_blank\" rel=\"noopener nofollow\">",
                    );
                    let shown = match link.alias {
                        Some(a) => Cow::Borrowed(a),
                        None => Cow::Owned(wikilink_display(&link)),
                    };
                    escape_html(&shown, &mut self.out);
                    self.out.push_str("</a>");
                }
            }
            InlineKind::Tag(name) => {
                self.out.push_str("<a href=\"#");
                escape_html(name, &mut self.out);
                self.out
                    .push_str("\" class=\"tag\" target=\"_blank\" rel=\"noopener nofollow\">#");
                escape_html(name, &mut self.out);
                self.out.push_str("</a>");
            }
            InlineKind::Math { content, display } => {
                let class = if *display {
                    "math math-block"
                } else {
                    "math math-inline"
                };
                let _ = write!(self.out, "<span class=\"{class}\">");
                escape_html(content, &mut self.out);
                self.out.push_str("</span>");
            }
            InlineKind::Comment(_) => {}
            InlineKind::FootnoteReference(label) => {
                if self.definitions.iter().any(|(l, _)| *l == label.as_ref()) {
                    let n = self.footnote_number(label);
                    self.footnote_ref(n, label);
                } else {
                    escape_html(label, &mut self.out);
                }
            }
            InlineKind::InlineFootnote(children) => {
                let label = format!("inline-{}", self.inline_footnotes.len() + 1);
                self.inline_footnotes.push((label.clone(), children));
                let n = self.footnote_number(&label);
                self.footnote_ref(n, &label);
            }
        }
    }

    fn link(
        &mut self,
        destination: &str,
        title: Option<&str>,
        kind: LinkKind,
        children: &'r [Inline<'a>],
    ) {
        if !self.obsidian() {
            self.out.push_str("<a href=\"");
            escape_href(destination, &mut self.out);
            self.out.push('"');
            if let Some(t) = title {
                self.out.push_str(" title=\"");
                escape_html(t, &mut self.out);
                self.out.push('"');
            }
            self.out.push('>');
            self.inlines(children);
            self.out.push_str("</a>");
            return;
        }
        let internal = matches!(kind, LinkKind::Inline | LinkKind::Reference)
            && is_vault_destination(destination);
        if internal {
            let dest = percent_decode(destination);
            self.out.push_str("<a class=\"internal-link\" data-href=\"");
            escape_html(&dest, &mut self.out);
            self.out.push_str("\" href=\"");
            escape_html(&dest, &mut self.out);
            self.out.push('"');
            if let Some(t) = title {
                self.out.push_str(" title=\"");
                escape_html(t, &mut self.out);
                self.out.push('"');
            }
            self.out
                .push_str(" target=\"_blank\" rel=\"noopener nofollow\">");
            self.anchor_body(children);
            return;
        }
        let mut shown = plain_text(children);
        if kind == LinkKind::Autolink {
            if let Some(rest) = shown.strip_prefix("mailto:") {
                shown = rest.to_string();
            }
        }
        self.out.push_str("<a ");
        if shown != destination {
            self.out
                .push_str("data-tooltip-position=\"top\" aria-label=\"");
            escape_html(destination, &mut self.out);
            self.out.push_str("\" ");
        }
        self.out
            .push_str("rel=\"noopener nofollow\" class=\"external-link\"");
        if let Some(t) = title {
            self.out.push_str(" title=\"");
            escape_html(t, &mut self.out);
            self.out.push('"');
        }
        self.out.push_str(" href=\"");
        escape_external_href(destination, &mut self.out);
        self.out.push_str("\" target=\"_blank\">");
        if kind == LinkKind::Autolink {
            escape_html(&shown, &mut self.out);
            self.out.push_str("</a>");
        } else {
            self.anchor_body(children);
        }
    }

    /// Renders a link's children and closes it, ending the anchor early when a
    /// child renders as an anchor of its own.
    ///
    /// Obsidian never nests `<a>`: it closes the link at the first such child
    /// and does not reopen it, so the text *after* that child is not part of
    /// the link. `[md [[Links]] link](url)` gives `<a>md </a>`, then the
    /// wikilink's own anchor, then a bare ` link`; with two wikilinks the text
    /// between them is bare as well. A tag counts, since a tag is an anchor
    /// too; an embed or an image does not, being a `<span>`. Observed against
    /// Obsidian 1.14.4 (tools/probe-gen, 2026-10-29).
    fn anchor_body(&mut self, children: &'r [Inline<'a>]) {
        let split = if self.obsidian() {
            children.iter().position(renders_as_anchor)
        } else {
            None
        };
        match split {
            None => {
                self.inlines(children);
                self.out.push_str("</a>");
            }
            Some(at) => {
                self.inlines(&children[..at]);
                self.out.push_str("</a>");
                self.inlines(&children[at..]);
            }
        }
    }

    fn wrap(&mut self, tag: &str, children: &'r [Inline<'a>]) {
        let _ = write!(self.out, "<{tag}>");
        self.inlines(children);
        let _ = write!(self.out, "</{tag}>");
    }

    fn footnote_number(&mut self, label: &str) -> usize {
        if let Some(i) = self.footnote_numbers.iter().position(|l| l == label) {
            return i + 1;
        }
        self.footnote_numbers.push(label.to_string());
        self.footnote_numbers.len()
    }

    fn footnote_ref(&mut self, n: usize, label: &str) {
        let _ = write!(
            self.out,
            "<sup data-footnote-id=\"fnref-{n}\" class=\"footnote-ref\" id=\"fnref-{n}\"><a data-footref=\""
        );
        escape_html(label, &mut self.out);
        let _ = write!(
            self.out,
            "\" href=\"#fn-{n}\" class=\"footnote-link\" target=\"_blank\" rel=\"noopener nofollow\">[{n}]</a></sup>"
        );
    }

    fn footnote_section(&mut self) {
        if self.footnote_numbers.is_empty() {
            return;
        }
        self.cr();
        self.out.push_str("<section class=\"footnotes\"><hr><ol>\n");
        let labels = self.footnote_numbers.clone();
        for (i, label) in labels.iter().enumerate() {
            let n = i + 1;
            let _ = write!(self.out, "<li data-footnote-id=\"fn-{n}\" id=\"fn-{n}\">");
            if let Some((_, inlines)) = self.inline_footnotes.iter().find(|(l, _)| l == label) {
                let inlines = *inlines;
                self.inlines(inlines);
                self.backref(n);
            } else if let Some((_, blocks)) = self.definitions.iter().find(|(l, _)| *l == label) {
                let blocks = *blocks;
                let split = blocks.len().saturating_sub(1);
                self.blocks(&blocks[..split], false);
                match blocks.last() {
                    Some(Block {
                        kind: BlockKind::Paragraph { content },
                        ..
                    }) => {
                        self.cr();
                        self.out.push_str("<p>");
                        self.inlines(content);
                        self.backref(n);
                        self.out.push_str("</p>\n");
                    }
                    Some(last) => {
                        self.block(last, false);
                        self.backref(n);
                    }
                    None => self.backref(n),
                }
            } else {
                self.backref(n);
            }
            self.out.push_str("</li>\n");
        }
        self.out.push_str("</ol></section>\n");
    }

    fn backref(&mut self, n: usize) {
        let _ = write!(
            self.out,
            "<a href=\"#fnref-{n}\" class=\"footnote-backref footnote-link\" target=\"_blank\" rel=\"noopener nofollow\">\u{21a9}\u{fe0e}</a>"
        );
    }
}

/// `200` or `200x100`: an embed size hint rather than alt text.
fn is_size_hint(s: &str) -> bool {
    !s.is_empty()
        && s.split('x').count() <= 2
        && s.split('x')
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

/// Display text of a wikilink without an alias: `Note`, `Note > Heading`, `Note > ^id`.
fn wikilink_display(link: &crate::LinkText<'_>) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !link.path.is_empty() {
        parts.push(link.path.to_string());
    }
    match &link.subpath {
        Some(crate::Subpath::Headings(hs)) => parts.extend(hs.iter().map(|h| h.to_string())),
        Some(crate::Subpath::Block(b)) => parts.push(format!("^{b}")),
        None => {}
    }
    parts.join(" > ")
}
