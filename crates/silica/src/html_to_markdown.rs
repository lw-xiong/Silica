//! Copied rich text (`text/html`) as Markdown: what a paste writes when
//! "Convert pasted HTML to Markdown" is on (Obsidian's default), held to
//! Obsidian 1.15.0's own output for every fixture the 2026-11-15 probe
//! pasted (the fixtures are in `tests/obsidian_html_paste.json`).
//!
//! The HTML is parsed by html5ever, the WHATWG parser, so a browser's
//! fragment — Chromium's `<meta charset='utf-8'>` and fragment markers, an
//! unclosed `<li>` — is recovered the way the page itself was. That is HTML
//! input, not Markdown: the one Markdown parser is not involved.
//!
//! What Obsidian writes, in short:
//!
//! - Blocks (`p`, `div`, headings, lists, quotes, tables, `pre`, `hr`) are
//!   a blank line apart, and the result is trimmed of leading newlines and
//!   trailing whitespace. `<br>` is a hard break, two spaces and a newline.
//! - Headings are ATX (`## Two`); `<hr>` is `---`.
//! - `b`/`strong` are `**`, `i`/`em` are `_`, `del`/`s` are `~~`, `mark`
//!   is `==`; spaces just inside a mark go outside it (`<b> x </b>` is
//!   ` **x** `), and an empty one writes nothing. `strike`, `u`, `sup`,
//!   `sub`, `kbd` and any other inline element write their text alone.
//! - `<code>` is a code span whose backtick run is the shortest the text
//!   does not contain, padded with a space when the text starts or ends
//!   with a backtick; `<pre><code>` is a fenced block, three backticks plus
//!   a `language-x` class's `x`, the fence one longer than any fence inside.
//! - A link is `[text](href "title")`, the href's spaces as `%20`; an
//!   anchor without an href writes its text. An image is `![alt](src
//!   "title")`, a `data:` source included — nothing is saved as a file.
//! - Lists use `- ` and `1. `, children indented four spaces; a task item's
//!   checkbox is `[x] ` or `[ ] `.
//! - Tables are pipe tables with no padding (`|A|B|`), the alignment row
//!   from the header cells' `align`, a cell's `|` escaped; a table with no
//!   header row gets an empty one (`|   |   |`).
//! - Nothing is escaped: `*stars*` and `[[wiki]]` in the text stay as they
//!   are. Entities are decoded, a no-break space kept as U+00A0.
//! - `script`, `style`, `noscript`, comments, `iframe`, `video` and the
//!   document head write nothing.
//!
//! One rule is Silica's own and stricter than Obsidian's: a link whose
//! scheme is one a browser would run (`javascript:`, `vbscript:`) or one
//! that wraps a document (`data:`) keeps its text and loses its link, so a
//! pasted page cannot hand the reader a live script.

use html5ever::tendril::TendrilSink;
use markup5ever_rcdom::{Handle, NodeData, RcDom};

/// The Markdown a pasted `html` becomes. `None` when the input holds no
/// markup worth converting — a plain string that happens to be on the
/// clipboard as `text/html`, or an empty document.
pub fn convert(html: &str) -> Option<String> {
    if !looks_like_html(html) {
        return None;
    }
    let markdown = to_markdown(html);
    (!markdown.is_empty()).then_some(markdown)
}

/// Whether `s` is markup rather than a string that merely arrived as
/// `text/html`: a tag somewhere in it, not only at its start — a page's
/// fragment can begin with text (`a<br>b`) — and not one of the entities a
/// page writes into a plain-text clipboard by mistake.
fn looks_like_html(s: &str) -> bool {
    let bytes = s.as_bytes();
    let mut i = 0;
    while let Some(at) = s[i..].find('<') {
        let at = i + at;
        let after = bytes.get(at + 1).copied();
        let tag = matches!(
            after,
            Some(c) if c.is_ascii_alphabetic() || matches!(c, b'/' | b'!' | b'?')
        );
        if tag && s[at..].contains('>') {
            return true;
        }
        i = at + 1;
    }
    false
}

/// The Markdown the HTML becomes.
fn to_markdown(html: &str) -> String {
    let dom = html5ever::parse_document(RcDom::default(), Default::default()).one(html);
    let mut texts = Vec::new();
    let body = find_body(&dom.document).map(|b| build_children(&b, &mut texts));
    let Some(children) = body else {
        return String::new();
    };
    let root = Element {
        name: "body".to_string(),
        attrs: Vec::new(),
        children,
    };
    collapse_whitespace(&root, &mut texts);
    let out = Converter { texts: &texts }.children(&root, Context::default());
    trim_output(&out)
}

// ----- the tree ---------------------------------------------------------------

enum Node {
    Element(Element),
    /// An index into the text arena, so the whitespace pass can rewrite
    /// text while it walks the tree.
    Text(usize),
}

struct Element {
    name: String,
    attrs: Vec<(String, String)>,
    children: Vec<Node>,
}

impl Element {
    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

fn find_body(handle: &Handle) -> Option<Handle> {
    if let NodeData::Element { name, .. } = &handle.data {
        if &*name.local == "body" {
            return Some(handle.clone());
        }
    }
    handle.children.borrow().iter().find_map(find_body)
}

/// The elements whose content never reaches the Markdown.
const DROPPED: &[&str] = &[
    "script", "style", "noscript", "template", "head", "title", "meta", "link", "iframe", "video",
    "audio", "object", "embed", "canvas", "svg", "math", "select", "button", "textarea",
];

fn build_children(handle: &Handle, texts: &mut Vec<String>) -> Vec<Node> {
    let mut out = Vec::new();
    for child in handle.children.borrow().iter() {
        match &child.data {
            NodeData::Text { contents } => {
                texts.push(contents.borrow().to_string());
                out.push(Node::Text(texts.len() - 1));
            }
            NodeData::Element { name, attrs, .. } => {
                let name = name.local.to_string();
                if DROPPED.contains(&name.as_str()) {
                    continue;
                }
                let attrs = attrs
                    .borrow()
                    .iter()
                    .map(|a| (a.name.local.to_string(), a.value.to_string()))
                    .collect();
                let children = build_children(child, texts);
                out.push(Node::Element(Element {
                    name,
                    attrs,
                    children,
                }));
            }
            _ => {}
        }
    }
    out
}

const BLOCKS: &[&str] = &[
    "address",
    "article",
    "aside",
    "blockquote",
    "body",
    "center",
    "dd",
    "details",
    "dialog",
    "dir",
    "div",
    "dl",
    "dt",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hgroup",
    "hr",
    "html",
    "legend",
    "li",
    "main",
    "menu",
    "nav",
    "ol",
    "output",
    "p",
    "pre",
    "section",
    "summary",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "tr",
    "ul",
];

const VOIDS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

fn is_block(name: &str) -> bool {
    BLOCKS.contains(&name)
}

fn is_void(name: &str) -> bool {
    VOIDS.contains(&name)
}

// ----- whitespace ---------------------------------------------------------------

/// HTML's whitespace rule, applied to the text before it is converted: a
/// run of spaces, tabs and newlines is one space; a space that follows
/// another, or opens a block, goes; a space before a block boundary or a
/// `<br>` goes. A no-break space is not whitespace here. `pre` keeps its
/// text exactly.
fn collapse_whitespace(root: &Element, texts: &mut [String]) {
    #[derive(Default)]
    struct State {
        prev: Option<usize>,
        keep_leading: bool,
    }
    fn strip_trailing(texts: &mut [String], st: &mut State) {
        if let Some(p) = st.prev.take() {
            if texts[p].ends_with(' ') {
                texts[p].pop();
            }
        }
    }
    fn walk(el: &Element, texts: &mut [String], st: &mut State) {
        for child in &el.children {
            match child {
                Node::Text(i) => {
                    let mut text = collapse_run(&texts[*i]);
                    let after_space = st.prev.is_none_or(|p| texts[p].ends_with(' '));
                    if after_space && !st.keep_leading && text.starts_with(' ') {
                        text.remove(0);
                    }
                    texts[*i] = text;
                    if !texts[*i].is_empty() {
                        st.prev = Some(*i);
                        st.keep_leading = false;
                    }
                }
                Node::Element(e) => {
                    let name = e.name.as_str();
                    if is_block(name) || name == "br" {
                        strip_trailing(texts, st);
                        st.keep_leading = false;
                    } else if is_void(name) {
                        st.prev = None;
                        st.keep_leading = true;
                    }
                    if name == "pre" {
                        // Its text is kept; what follows starts afresh.
                        st.prev = None;
                        st.keep_leading = false;
                        continue;
                    }
                    walk(e, texts, st);
                    if is_block(name) {
                        strip_trailing(texts, st);
                        st.keep_leading = false;
                    }
                }
            }
        }
    }
    let mut st = State::default();
    walk(root, texts, &mut st);
    strip_trailing(texts, &mut st);
}

fn collapse_run(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_ws = false;
    for ch in text.chars() {
        if matches!(ch, ' ' | '\t' | '\n' | '\r' | '\x0c') {
            if !in_ws {
                out.push(' ');
            }
            in_ws = true;
        } else {
            out.push(ch);
            in_ws = false;
        }
    }
    out
}

// ----- conversion ---------------------------------------------------------------

#[derive(Clone, Copy, Default)]
struct Context {
    /// Inside `pre`: text is written as it stands.
    pre: bool,
}

struct Converter<'a> {
    texts: &'a [String],
}

impl Converter<'_> {
    fn children(&self, el: &Element, cx: Context) -> String {
        let mut out = String::new();
        for (ix, child) in el.children.iter().enumerate() {
            let piece = match child {
                Node::Text(i) => self.text(*i, cx),
                Node::Element(e) => self.element(e, el, ix, cx),
            };
            out = join(&out, &piece);
        }
        out
    }

    fn text(&self, i: usize, cx: Context) -> String {
        let t = &self.texts[i];
        if cx.pre {
            t.replace("\r\n", "\n")
        } else {
            t.clone()
        }
    }

    /// The text of `el` and everything in it, markup ignored.
    fn plain(&self, el: &Element) -> String {
        let mut out = String::new();
        for child in &el.children {
            match child {
                Node::Text(i) => out.push_str(&self.texts[*i]),
                Node::Element(e) if e.name == "br" => out.push('\n'),
                Node::Element(e) => out.push_str(&self.plain(e)),
            }
        }
        out
    }

    fn element(&self, el: &Element, parent: &Element, ix: usize, cx: Context) -> String {
        let name = el.name.as_str();
        match name {
            "br" => "  \n".to_string(),
            "hr" => "\n\n---\n\n".to_string(),
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                let level = (name.as_bytes()[1] - b'0') as usize;
                let content = self.children(el, cx);
                format!("\n\n{} {}\n\n", "#".repeat(level), content)
            }
            "b" | "strong" => self.delimited(el, "**", cx),
            "i" | "em" => self.delimited(el, "_", cx),
            "del" | "s" => self.delimited(el, "~~", cx),
            "mark" => self.delimited(el, "==", cx),
            "code" if !cx.pre => code_span(&self.children(el, cx)),
            "pre" => self.pre(el),
            "a" => self.link(el, cx),
            "img" => image(el),
            "input" => {
                if el
                    .attr("type")
                    .is_some_and(|t| t.eq_ignore_ascii_case("checkbox"))
                    && parent.name == "li"
                {
                    if el.attr("checked").is_some() {
                        "[x] ".to_string()
                    } else {
                        "[ ] ".to_string()
                    }
                } else {
                    String::new()
                }
            }
            "ul" | "ol" => {
                let content = self.children(el, cx);
                let last_in_item =
                    parent.name == "li" && !has_content_after(parent, ix, self.texts);
                if last_in_item {
                    format!("\n{content}")
                } else {
                    format!("\n\n{content}\n\n")
                }
            }
            "li" => self.list_item(el, parent, ix, cx),
            "blockquote" => {
                let content = self.children(el, cx);
                let content = trim_newlines(&content);
                let quoted: Vec<String> = content.split('\n').map(|l| format!("> {l}")).collect();
                format!("\n\n{}\n\n", quoted.join("\n"))
            }
            "table" => self.table(el, cx),
            _ if is_block(name) => {
                let content = self.children(el, cx);
                format!("\n\n{content}\n\n")
            }
            _ => self.children(el, cx),
        }
    }

    /// `**x**`, `_x_`: the spaces just inside move outside, and an empty
    /// mark writes nothing.
    fn delimited(&self, el: &Element, delimiter: &str, cx: Context) -> String {
        let content = self.children(el, cx);
        if content.trim().is_empty() {
            return String::new();
        }
        let inner = content.trim_matches(' ');
        let lead = &content[..content.len() - content.trim_start_matches(' ').len()];
        let trail = &content[content.trim_end_matches(' ').len()..];
        format!("{lead}{delimiter}{inner}{delimiter}{trail}")
    }

    fn pre(&self, el: &Element) -> String {
        let code = el.children.iter().find_map(|c| match c {
            Node::Element(e) if e.name == "code" => Some(e),
            _ => None,
        });
        let only_code = code.is_some()
            && el.children.iter().all(|c| match c {
                Node::Element(e) => e.name == "code",
                Node::Text(i) => self.texts[*i].trim().is_empty(),
            });
        match code {
            Some(code) if only_code => {
                let lang = code
                    .attr("class")
                    .and_then(|c| {
                        c.split_whitespace()
                            .find_map(|w| w.strip_prefix("language-"))
                    })
                    .unwrap_or("");
                let text = self.plain(code).replace("\r\n", "\n");
                let text = text.strip_suffix('\n').unwrap_or(&text);
                let fence = "`".repeat(fence_len(text));
                format!("\n\n{fence}{lang}\n{text}\n{fence}\n\n")
            }
            _ => {
                let content = self.children(el, Context { pre: true });
                format!("\n\n{content}\n\n")
            }
        }
    }

    fn link(&self, el: &Element, cx: Context) -> String {
        let content = self.children(el, cx);
        let Some(href) = el.attr("href").filter(|h| !h.is_empty()) else {
            return content;
        };
        // * A scheme a browser would run, or one that wraps a document,
        //   keeps its text and loses its link.
        if is_unsafe_destination(href) {
            return content;
        }
        let href = destination(href);
        let title = el
            .attr("title")
            .map(|t| format!(" \"{}\"", t.replace('"', "\\\"")))
            .unwrap_or_default();
        format!("[{content}]({href}{title})")
    }

    fn list_item(&self, el: &Element, parent: &Element, ix: usize, cx: Context) -> String {
        let prefix = if parent.name == "ol" {
            let start = parent
                .attr("start")
                .and_then(|s| s.trim().parse::<i64>().ok())
                .unwrap_or(1);
            let index = parent.children[..ix]
                .iter()
                .filter(|c| matches!(c, Node::Element(e) if e.name == "li"))
                .count() as i64;
            format!("{}. ", start + index)
        } else {
            "- ".to_string()
        };
        let content = self.children(el, cx);
        let content = content.trim_start_matches('\n');
        let trimmed = content.trim_end_matches('\n');
        let content = if trimmed.len() < content.len() {
            format!("{trimmed}\n")
        } else {
            trimmed.to_string()
        };
        let content = content.replace('\n', "\n    ");
        let more = has_content_after(parent, ix, self.texts);
        let newline = if more && !content.ends_with('\n') {
            "\n"
        } else {
            ""
        };
        format!("{prefix}{content}{newline}")
    }

    fn table(&self, el: &Element, cx: Context) -> String {
        let mut rows: Vec<&Element> = Vec::new();
        collect_rows(el, &mut rows);
        if rows.is_empty() {
            return String::new();
        }
        let line = |row: &Element| -> String {
            let mut parts: Vec<String> = Vec::new();
            for cell in cells(row) {
                let content = self.children(cell, cx);
                parts.push(content.trim().replace('\n', "<br>").replace('|', "\\|"));
                // A spanning cell is followed by the empty ones it covers.
                let span = cell
                    .attr("colspan")
                    .and_then(|s| s.trim().parse::<usize>().ok())
                    .unwrap_or(1);
                for _ in 1..span {
                    parts.push("   ".to_string());
                }
            }
            format!("|{}|", parts.join("|"))
        };
        let first = rows[0];
        let first_cells = cells(first);
        let header_row = !first_cells.is_empty()
            && (first_cells.iter().all(|c| c.name == "th") || in_thead(el, first));
        let mut out = Vec::new();
        let (header, body) = if header_row {
            (line(first), &rows[1..])
        } else {
            let blank = vec!["   "; first_cells.len()].join("|");
            (format!("|{blank}|"), &rows[..])
        };
        let aligns: Vec<&str> = if header_row {
            first_cells
                .iter()
                .map(|c| match c.attr("align").map(|a| a.to_ascii_lowercase()) {
                    Some(a) if a == "left" => ":--",
                    Some(a) if a == "center" => ":-:",
                    Some(a) if a == "right" => "--:",
                    _ => "---",
                })
                .collect()
        } else {
            vec!["---"; first_cells.len()]
        };
        out.push(header);
        out.push(format!("|{}|", aligns.join("|")));
        for row in body {
            out.push(line(row));
        }
        format!("\n\n{}\n\n", out.join("\n"))
    }
}

/// A row's `td` and `th` cells.
fn cells(row: &Element) -> Vec<&Element> {
    row.children
        .iter()
        .filter_map(|c| match c {
            Node::Element(e) if e.name == "td" || e.name == "th" => Some(e),
            _ => None,
        })
        .collect()
}

fn collect_rows<'a>(el: &'a Element, rows: &mut Vec<&'a Element>) {
    for child in &el.children {
        if let Node::Element(e) = child {
            match e.name.as_str() {
                "tr" => rows.push(e),
                "thead" | "tbody" | "tfoot" => collect_rows(e, rows),
                _ => {}
            }
        }
    }
}

fn in_thead(table: &Element, row: &Element) -> bool {
    table.children.iter().any(|c| match c {
        Node::Element(e) if e.name == "thead" => e
            .children
            .iter()
            .any(|r| matches!(r, Node::Element(r) if std::ptr::eq(r, row))),
        _ => false,
    })
}

/// Whether anything is left in `parent` after its child `ix`: an element,
/// or text the whitespace pass kept.
fn has_content_after(parent: &Element, ix: usize, texts: &[String]) -> bool {
    parent.children[ix + 1..].iter().any(|c| match c {
        Node::Element(_) => true,
        Node::Text(i) => !texts[*i].is_empty(),
    })
}

fn image(el: &Element) -> String {
    let Some(src) = el.attr("src").filter(|s| !s.is_empty()) else {
        return String::new();
    };
    let src = destination(src);
    let alt = el.attr("alt").unwrap_or("");
    let title = el
        .attr("title")
        .map(|t| format!(" \"{}\"", t.replace('"', "\\\"")))
        .unwrap_or_default();
    format!("![{alt}]({src}{title})")
}

/// Whether a destination carries a scheme that must not survive into a
/// Markdown link.
///
/// Schemes are case-insensitive (RFC 3986), so `JaVaScRiPt:` is the same
/// scheme as `javascript:`. `data:` is refused too: a `data:text/html`
/// link is a document in a wrapper.
fn is_unsafe_destination(href: &str) -> bool {
    let scheme = match href.split_once(':') {
        Some((scheme, _)) => scheme.trim().to_ascii_lowercase(),
        None => return false,
    };
    // * A colon before any `/`, `?` or `#` opens a scheme; anything else is
    //   a path segment that happens to contain a colon.
    if scheme.contains('/') || scheme.contains('?') || scheme.contains('#') {
        return false;
    }
    matches!(scheme.as_str(), "javascript" | "vbscript" | "data")
}

/// A link's or image's destination as written: trimmed, its spaces as
/// `%20` and its parentheses escaped; anything else (`"`, `<`, non-ASCII,
/// an escape already there) stays.
fn destination(url: &str) -> String {
    let mut out = String::with_capacity(url.len());
    for ch in url.trim().chars() {
        match ch {
            ' ' => out.push_str("%20"),
            '(' | ')' => {
                out.push('\\');
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    out
}

/// A code span around `text`: the shortest backtick run the text does not
/// hold, with a space inside each end when the text starts or ends with a
/// backtick.
fn code_span(text: &str) -> String {
    let mut runs: Vec<usize> = Vec::new();
    let mut run = 0;
    for ch in text.chars() {
        if ch == '`' {
            run += 1;
        } else if run > 0 {
            runs.push(run);
            run = 0;
        }
    }
    if run > 0 {
        runs.push(run);
    }
    let mut n = 1;
    while runs.contains(&n) {
        n += 1;
    }
    let ticks = "`".repeat(n);
    let pad = if text.starts_with('`') || text.ends_with('`') {
        " "
    } else {
        ""
    };
    format!("{ticks}{pad}{text}{pad}{ticks}")
}

/// Three backticks, or one more than the longest run of three or more
/// that opens a line of `code`.
fn fence_len(code: &str) -> usize {
    let mut len = 3;
    for line in code.split('\n') {
        let run = line.chars().take_while(|&c| c == '`').count();
        if run >= len {
            len = run + 1;
        }
    }
    len
}

/// Two pieces put together: the newlines where they meet are the larger
/// of the two sides' runs, two at most.
fn join(out: &str, piece: &str) -> String {
    let left = out.trim_end_matches('\n');
    let right = piece.trim_start_matches('\n');
    let newlines = (out.len() - left.len())
        .max(piece.len() - right.len())
        .min(2);
    let mut s = String::with_capacity(left.len() + newlines + right.len());
    s.push_str(left);
    s.push_str(&"\n".repeat(newlines));
    s.push_str(right);
    s
}

fn trim_newlines(s: &str) -> &str {
    s.trim_start_matches('\n').trim_end_matches('\n')
}

/// No newline or tab at the start, no whitespace at the end.
fn trim_output(s: &str) -> String {
    let s = s.trim_start_matches(['\t', '\r', '\n']);
    s.trim_end_matches([' ', '\t', '\r', '\n']).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_code_span_takes_the_shortest_run_its_text_lacks() {
        assert_eq!(code_span("x"), "`x`");
        assert_eq!(code_span("a`b"), "``a`b``");
        assert_eq!(code_span("``x"), "` ``x `");
        assert_eq!(code_span("a ` b `` c"), "```a ` b `` c```");
    }

    #[test]
    fn a_fence_outgrows_the_fences_inside() {
        assert_eq!(fence_len("plain"), 3);
        assert_eq!(fence_len("```\ninner\n```"), 4);
        assert_eq!(fence_len("x\n`````"), 6);
        // A run that does not open a line does not count.
        assert_eq!(fence_len("a ````"), 3);
    }

    #[test]
    fn pieces_meet_at_two_newlines_at_most() {
        assert_eq!(join("a", "b"), "ab");
        assert_eq!(join("a\n\n", "\n\nb"), "a\n\nb");
        assert_eq!(join("a\n", "b"), "a\nb");
        assert_eq!(join("a\n\n\n\n", "b"), "a\n\nb");
        assert_eq!(join("", "\n\nb"), "\n\nb");
    }

    #[test]
    fn a_destination_escapes_spaces_and_parentheses_only() {
        assert_eq!(destination(" https://e.com/a b "), "https://e.com/a%20b");
        assert_eq!(
            destination("https://e.com/a_(b)"),
            "https://e.com/a_\\(b\\)"
        );
        assert_eq!(
            destination("https://e.com/caf\u{e9}?x=%20y"),
            "https://e.com/caf\u{e9}?x=%20y"
        );
    }

    #[test]
    fn nothing_in_gives_nothing_out() {
        assert_eq!(convert(""), None);
        assert_eq!(convert("<!-- only a comment -->"), None);
        assert_eq!(convert("<script>x()</script>"), None);
        // * A plain string is not markup, and is left to the plain paste.
        assert_eq!(convert("just text"), None);
    }

    /// Every HTML fixture the 2026-11-15 probe pasted into Obsidian
    /// 1.15.0, converted here and compared with the text Obsidian's editor
    /// held after the paste: the paste a reader gets is the one Obsidian
    /// gives.
    #[test]
    fn every_fixture_converts_to_the_markdown_obsidian_writes() {
        let doc: serde_json::Value =
            serde_json::from_str(include_str!("../tests/obsidian_html_paste.json"))
                .expect("the fixture is json");
        let cases = doc["cases"].as_array().expect("cases");
        assert!(cases.len() >= 50, "{} cases", cases.len());
        let mut failures = Vec::new();
        for case in cases {
            let id = case["id"].as_str().unwrap();
            let html = case["html"].as_str().unwrap();
            let want = case["markdown"].as_str().unwrap();
            let got = convert(html).unwrap_or_default();
            if got != want {
                failures.push(format!("{id}:\n  want {want:?}\n  got  {got:?}"));
            }
        }
        assert!(
            failures.is_empty(),
            "{} of {} differ:\n{}",
            failures.len(),
            cases.len(),
            failures.join("\n")
        );
    }

    #[test]
    fn a_live_scheme_keeps_its_text_and_loses_its_link() {
        for scheme in ["javascript", "JAVASCRIPT", "JaVaScRiPt", "vbscript", "data"] {
            let html = format!("<p><a href=\"{scheme}:x()\">bad</a></p>");
            assert_eq!(convert(&html).as_deref(), Some("bad"), "{scheme}");
        }
        // * A colon inside a path is not a scheme.
        let html = "<p><a href=\"/a/b:c\">ok</a></p>";
        assert_eq!(convert(html).as_deref(), Some("[ok](/a/b:c)"));
    }
}
