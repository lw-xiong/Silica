//! HTML to Markdown, for "Convert pasted HTML to Markdown".
//!
//! A page's clipboard carries `text/html` beside `text/plain`, and the plain
//! copy has lost everything the page expressed in markup — a heading comes
//! back as a bare line, a list as lines with no marker. With `autoConvertHtml`
//! on, Obsidian reads the HTML and writes Markdown instead; this is that.
//!
//! It is a small converter on purpose. The input is one page's clipboard, not
//! a document to round-trip: it handles the block elements a page is written
//! with (headings, paragraphs, lists, quotes, code, rules, tables) and the
//! inline ones that carry meaning (emphasis, code, links, images), and drops
//! what Markdown cannot say — attributes, scripts, styles, layout tables.
//! Anything it does not know is unwrapped to its text rather than dropped.

/// Convert `html` to Markdown. `None` when the input holds no markup worth
/// converting — a plain string that happens to be on the clipboard as
/// `text/html`, or an empty document.
pub fn convert(html: &str) -> Option<String> {
    read(html)
        .filter(|page| !page.markdown.is_empty())
        .map(|page| page.markdown)
}

/// A page read for more than its text: the title it declares, the Markdown
/// its body becomes, and the files that body names.
///
/// An import needs all three. The note is named after the page's title
/// rather than after whatever the file was called, and every image the page
/// shows is a file sitting next to it that has to be copied across, or the
/// note points at a picture the vault does not have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    /// The title the page declares — its `<title>`, else its first
    /// heading. `None` when it declares neither.
    pub title: Option<String>,
    /// The body as Markdown, exactly what [`convert`] answers.
    pub markdown: String,
    /// Every local file the body's markup names, in document order, once
    /// each, spelled the way the markup spells it: `pictures/a.png`,
    /// `../files/b.png`. A remote target is not a file on this machine, so
    /// it stays in the Markdown as it is and is not listed.
    pub assets: Vec<String>,
}

/// Read `html` as a page. `None` when the input is not markup, or when it
/// says nothing at all — no title, no body, no file.
pub fn read(html: &str) -> Option<Page> {
    if !looks_like_html(html) {
        return None;
    }
    let mut p = Parser::new(html);
    let nodes = p.parse_nodes(None);
    let mut out = String::new();
    write_blocks(&nodes, &mut out, 0);
    let markdown = tidy(&out);
    let assets = assets_of(&nodes);
    let title = title_of(html, &nodes);
    let empty = markdown.is_empty() && assets.is_empty() && title.is_none();
    (!empty).then_some(Page {
        title,
        markdown,
        assets,
    })
}

/// Whether `s` is markup rather than a string that merely arrived as
/// `text/html`: a tag, and not one of the entities a page writes into a
/// plain-text clipboard by mistake.
fn looks_like_html(s: &str) -> bool {
    let s = s.trim_start();
    s.starts_with('<') && s.contains('>')
}

/// Every local file the body's markup names, in document order, once each.
///
/// A target with a scheme — `https:`, `data:` — is a file somewhere else:
/// it stays in the Markdown as it is and is not a file to copy. A relative
/// path is a file sitting next to the page, and the note that points at it
/// is broken until that file is copied across too.
fn assets_of(nodes: &[Node]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    collect_assets(nodes, &mut out);
    out
}

/// Walks the tree in document order, taking each image's local file once.
fn collect_assets(nodes: &[Node], out: &mut Vec<String>) {
    for node in nodes {
        let Node::Element(el) = node else { continue };
        if el.tag == "img" {
            if let Some(src) = el.attr("src") {
                let src = src.trim();
                if !src.is_empty() && is_local_target(src) && !out.iter().any(|a| a == src) {
                    out.push(src.to_string());
                }
            }
        }
        collect_assets(&el.children, out);
    }
}

/// Whether a target names a file on this machine. A scheme says otherwise —
/// `https:` and `data:` are somewhere else — while a colon inside a path
/// segment (`pics/2:30.png`) is still this machine.
fn is_local_target(target: &str) -> bool {
    if target.starts_with("//") {
        // * A scheme-relative URL: the file is on a host, not next to the page.
        return false;
    }
    match target.split_once(':') {
        Some((scheme, _)) => {
            let scheme = scheme.trim();
            scheme.is_empty()
                || scheme.contains('/')
                || scheme.contains('?')
                || scheme.contains('#')
        }
        None => true,
    }
}

/// The title the page declares — its `<title>`, else its first heading.
/// `None` when it declares neither.
///
/// The tree drops `<head>` whole, so the `<title>` is read out of the
/// source itself; the heading is already in the tree.
fn title_of(html: &str, nodes: &[Node]) -> Option<String> {
    html_title(html).or_else(|| first_heading(nodes))
}

/// The `<title>`'s text, when the source declares one. The name has to end
/// at the tag's end — a space, the `>` or a `/` — so a `<titles>` is not
/// read as a title.
fn html_title(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let open = find_open_tag(&lower, "title")?;
    let name = "<title".len();
    let gt = lower[open + name..].find('>')?;
    let body = open + name + gt + 1;
    let end = lower[body..].find("</title>")? + body;
    let text = collapse(&decode_entities(&html[body..end]));
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// The offset of `<tag` in a lowercased source, when the tag name ends
/// there. Lowercasing leaves every other byte alone, so the offsets still
/// point into the original.
fn find_open_tag(lower: &str, tag: &str) -> Option<usize> {
    let needle = format!("<{tag}");
    let mut at = 0usize;
    while let Some(hit) = lower[at..].find(needle.as_str()) {
        let start = at + hit;
        let end = start + needle.len();
        match lower.as_bytes().get(end) {
            Some(b) if is_space(*b) || *b == b'>' || *b == b'/' => return Some(start),
            _ => at = end,
        }
    }
    None
}

/// The first heading with text, in document order.
fn first_heading(nodes: &[Node]) -> Option<String> {
    for node in nodes {
        let Node::Element(el) = node else { continue };
        if matches!(el.tag.as_str(), "h1" | "h2" | "h3" | "h4" | "h5" | "h6") {
            let text = inline_of(&el.children);
            let text = text.trim();
            if !text.is_empty() {
                return Some(text.to_string());
            }
            continue;
        }
        if let Some(found) = first_heading(&el.children) {
            return Some(found);
        }
    }
    None
}

// ---------------------------------------------------------------- the tree

#[derive(Debug, PartialEq)]
enum Node {
    Text(String),
    Element(Element),
}

#[derive(Debug, PartialEq)]
struct Element {
    tag: String,
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

/// Elements whose content is not text and never becomes Markdown.
const SKIP: &[&str] = &["script", "style", "head", "title", "noscript", "template"];

/// Elements that open a line of their own.
const BLOCK: &[&str] = &[
    "p",
    "div",
    "section",
    "article",
    "header",
    "footer",
    "main",
    "aside",
    "nav",
    "figure",
    "figcaption",
    "form",
    "fieldset",
    "address",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "ul",
    "ol",
    "li",
    "blockquote",
    "pre",
    "hr",
    "table",
    "thead",
    "tbody",
    "tfoot",
    "tr",
    "td",
    "th",
    "dl",
    "dt",
    "dd",
    "details",
    "summary",
];

/// Every HTML void element: these never have a close tag, so a parser that
/// waits for `</img>` swallows everything after it. The list is the whole
/// standard set, because a browser's clipboard is full of them.
const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

fn is_block(tag: &str) -> bool {
    BLOCK.contains(&tag)
}

// ---------------------------------------------------------------- the parser

/// How deeply elements may nest before the parser stops descending. The
/// parser recurses once per open element, so an input nested thousands deep
/// would exhaust the stack and take the app with it; a real page nests a few
/// dozen at most. Deeper elements are read as empty, which loses their text
/// rather than the whole paste.
const MAX_DEPTH: usize = 100;

struct Parser<'a> {
    src: &'a [u8],
    pos: usize,
    depth: usize,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str) -> Parser<'a> {
        Parser {
            src: src.as_bytes(),
            pos: 0,
            depth: 0,
        }
    }

    /// Parses until `close` (a tag name) or the end, consuming the closing
    /// tag. `None` means "to the end of the input".
    fn parse_nodes(&mut self, close: Option<&str>) -> Vec<Node> {
        let mut out: Vec<Node> = Vec::new();
        while self.pos < self.src.len() {
            if self.starts_with("<!--") {
                self.skip_past("-->");
                continue;
            }
            if self.starts_with("<!") || self.starts_with("<?") {
                self.skip_past(">");
                continue;
            }
            if self.starts_with("</") {
                let name = self.peek_tag_name(2);
                self.skip_past(">");
                if close.is_some_and(|c| c.eq_ignore_ascii_case(&name)) {
                    return out;
                }
                // * A stray close for a different element: its content has
                //   already been read, so there is nothing to do.
                continue;
            }
            if self.starts_with("<") {
                if let Some(el) = self.parse_element() {
                    if SKIP.contains(&el.tag.as_str()) {
                        continue;
                    }
                    out.push(Node::Element(el));
                }
                continue;
            }
            out.push(Node::Text(self.parse_text()));
        }
        out
    }

    /// `<tag attr="v">children</tag>` or `<tag/>`.
    fn parse_element(&mut self) -> Option<Element> {
        let open_len = self.peek_tag_name(1).len();
        if open_len == 0 {
            // * A stray `<`: it is text.
            self.pos += 1;
            return None;
        }
        let tag = self.peek_tag_name(1).to_ascii_lowercase();
        let (attrs, self_closing) = self.parse_attrs();
        if self_closing {
            return Some(Element {
                tag,
                attrs,
                children: Vec::new(),
            });
        }
        // * A void element has no children by definition: treating it as empty
        //   is both what Markdown wants and what stops it eating the next
        //   paragraph.
        if VOID.contains(&tag.as_str()) {
            return Some(Element {
                tag,
                attrs,
                children: Vec::new(),
            });
        }
        if self.depth >= MAX_DEPTH {
            // * Past the limit: consume this element's content without
            //   recursing into it, so the input is still read to the end and
            //   the elements around it survive.
            self.skip_past_close(&tag);
            return Some(Element {
                tag,
                attrs,
                children: Vec::new(),
            });
        }
        self.depth += 1;
        let children = self.parse_nodes(Some(&tag));
        self.depth -= 1;
        Some(Element {
            tag,
            attrs,
            children,
        })
    }

    /// Consumes through the matching `</tag>`, counting nested opens of the
    /// same name so the right close is the one that stops it.
    fn skip_past_close(&mut self, tag: &str) {
        let mut open = 1usize;
        while self.pos < self.src.len() {
            if self.starts_with("</") {
                let name = self.peek_tag_name(2);
                self.skip_past(">");
                if name.eq_ignore_ascii_case(tag) {
                    open -= 1;
                    if open == 0 {
                        return;
                    }
                }
                continue;
            }
            if self.starts_with("<") {
                let name = self.peek_tag_name(1);
                self.skip_past(">");
                if name.eq_ignore_ascii_case(tag) && !VOID.contains(&tag) {
                    open += 1;
                }
                continue;
            }
            self.pos += 1;
        }
    }

    /// Reads `<tag` … `>` and answers the attributes plus whether the tag
    /// closed itself. Leaves `pos` after the `>`.
    fn parse_attrs(&mut self) -> (Vec<(String, String)>, bool) {
        let mut attrs = Vec::new();
        self.pos += 1; // * the '<'
                       // * Skip the tag name, stopping at the `>` or `/` that ends it: a tag
                       //   with no attributes has no whitespace to stop on.
        while self.pos < self.src.len() {
            let c = self.src[self.pos];
            if is_space(c) || c == b'>' || c == b'/' {
                break;
            }
            self.pos += 1;
        }
        loop {
            while self.pos < self.src.len() && is_space(self.src[self.pos]) {
                self.pos += 1;
            }
            if self.pos >= self.src.len() {
                return (attrs, false);
            }
            match self.src[self.pos] {
                b'>' => {
                    self.pos += 1;
                    return (attrs, false);
                }
                b'/' => {
                    self.pos += 1;
                    if self.pos < self.src.len() && self.src[self.pos] == b'>' {
                        self.pos += 1;
                        return (attrs, true);
                    }
                }
                _ => {
                    let name = self.read_name();
                    while self.pos < self.src.len() && is_space(self.src[self.pos]) {
                        self.pos += 1;
                    }
                    if self.pos < self.src.len() && self.src[self.pos] == b'=' {
                        self.pos += 1;
                        while self.pos < self.src.len() && is_space(self.src[self.pos]) {
                            self.pos += 1;
                        }
                        let value = self.read_value();
                        if !name.is_empty() {
                            attrs.push((name.to_ascii_lowercase(), value));
                        }
                    } else if !name.is_empty() {
                        attrs.push((name.to_ascii_lowercase(), String::new()));
                    }
                }
            }
        }
    }

    fn read_name(&mut self) -> String {
        let start = self.pos;
        while self.pos < self.src.len() {
            let c = self.src[self.pos];
            if is_space(c) || c == b'=' || c == b'>' || c == b'/' {
                break;
            }
            self.pos += 1;
        }
        String::from_utf8_lossy(&self.src[start..self.pos]).into_owned()
    }

    fn read_value(&mut self) -> String {
        if self.pos >= self.src.len() {
            return String::new();
        }
        let quote = self.src[self.pos];
        if quote == b'"' || quote == b'\'' {
            self.pos += 1;
            let start = self.pos;
            while self.pos < self.src.len() && self.src[self.pos] != quote {
                self.pos += 1;
            }
            let v = String::from_utf8_lossy(&self.src[start..self.pos]).into_owned();
            if self.pos < self.src.len() {
                self.pos += 1;
            }
            decode_entities(&v)
        } else {
            let start = self.pos;
            while self.pos < self.src.len()
                && !is_space(self.src[self.pos])
                && self.src[self.pos] != b'>'
            {
                self.pos += 1;
            }
            decode_entities(&String::from_utf8_lossy(&self.src[start..self.pos]))
        }
    }

    fn parse_text(&mut self) -> String {
        let start = self.pos;
        while self.pos < self.src.len() && self.src[self.pos] != b'<' {
            self.pos += 1;
        }
        decode_entities(&String::from_utf8_lossy(&self.src[start..self.pos]))
    }

    fn starts_with(&self, s: &str) -> bool {
        self.src[self.pos..].starts_with(s.as_bytes())
    }

    /// The tag name starting `skip` bytes ahead of the `<`.
    fn peek_tag_name(&self, skip: usize) -> String {
        let mut i = self.pos + skip;
        let start = i;
        while i < self.src.len() {
            let c = self.src[i];
            if is_space_or_end(c) || c == b'>' || c == b'/' {
                break;
            }
            i += 1;
        }
        String::from_utf8_lossy(&self.src[start..i]).into_owned()
    }

    fn skip_past(&mut self, needle: &str) {
        match find(&self.src[self.pos..], needle.as_bytes()) {
            Some(ix) => self.pos += ix + needle.len(),
            None => self.pos = self.src.len(),
        }
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    (0..=haystack.len() - needle.len()).find(|&i| &haystack[i..i + needle.len()] == needle)
}

fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0c)
}

fn is_space_or_end(c: u8) -> bool {
    is_space(c)
}

/// The named entities that show up in prose, plus numeric ones. Anything
/// else is left as written rather than guessed at.
fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'&' {
            if let Some((text, len)) = entity_at(&bytes[i..]) {
                out.push_str(&text);
                i += len;
                continue;
            }
        }
        // * Copy one whole character, not one byte.
        let ch = s[i..].chars().next().unwrap_or('\u{fffd}');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn entity_at(bytes: &[u8]) -> Option<(String, usize)> {
    let semi = bytes.iter().take(12).position(|&b| b == b';')?;
    let name = std::str::from_utf8(&bytes[1..semi]).ok()?;
    let ch = if let Some(hex) = name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
        char::from_u32(u32::from_str_radix(hex, 16).ok()?)?
    } else if let Some(dec) = name.strip_prefix('#') {
        char::from_u32(dec.parse::<u32>().ok()?)?
    } else {
        match name {
            "amp" => '&',
            "lt" => '<',
            "gt" => '>',
            "quot" => '"',
            "apos" => '\'',
            "nbsp" => '\u{a0}',
            "mdash" => '—',
            "ndash" => '–',
            "hellip" => '…',
            "ldquo" => '“',
            "rdquo" => '”',
            "lsquo" => '‘',
            "rsquo" => '’',
            "copy" => '©',
            "reg" => '®',
            "trade" => '™',
            "deg" => '°',
            "middot" => '·',
            "bull" => '•',
            "laquo" => '«',
            "raquo" => '»',
            "times" => '×',
            "divide" => '÷',
            "minus" => '−',
            "plusmn" => '±',
            "frac12" => '½',
            "euro" => '€',
            "pound" => '£',
            "yen" => '¥',
            "cent" => '¢',
            "sect" => '§',
            "para" => '¶',
            "dagger" => '†',
            "prime" => '′',
            "Prime" => '″',
            "larr" => '←',
            "rarr" => '→',
            "uarr" => '↑',
            "darr" => '↓',
            "harr" => '↔',
            "infin" => '∞',
            "ne" => '≠',
            "le" => '≤',
            "ge" => '≥',
            "asymp" => '≈',
            "sum" => '∑',
            "prod" => '∏',
            "radic" => '√',
            "int" => '∫',
            "part" => '∂',
            "nabla" => '∇',
            "isin" => '∈',
            "notin" => '∉',
            "cap" => '∩',
            "cup" => '∪',
            "and" => '∧',
            "or" => '∨',
            "forall" => '∀',
            "exist" => '∃',
            "empty" => '∅',
            "alpha" => 'α',
            "beta" => 'β',
            "gamma" => 'γ',
            "delta" => 'δ',
            "epsilon" => 'ε',
            "lambda" => 'λ',
            "mu" => 'μ',
            "pi" => 'π',
            "sigma" => 'σ',
            "tau" => 'τ',
            "phi" => 'φ',
            "omega" => 'ω',
            "Delta" => 'Δ',
            "Omega" => 'Ω',
            "Sigma" => 'Σ',
            "Pi" => 'Π',
            _ => return None,
        }
    };
    Some((ch.to_string(), semi + 1))
}

// ---------------------------------------------------------------- the writer

/// Writes `nodes` as Markdown blocks, each separated by one blank line.
fn write_blocks(nodes: &[Node], out: &mut String, depth: usize) {
    let mut inline = String::new();
    for node in nodes {
        match node {
            Node::Text(t) => inline.push_str(t),
            Node::Element(el) if is_block(&el.tag) => {
                flush_inline(&mut inline, out);
                write_block(el, out, depth);
            }
            Node::Element(el) => write_inline(el, &mut inline),
        }
    }
    flush_inline(&mut inline, out);
}

/// Emits a pending run of inline content as its own paragraph.
fn flush_inline(inline: &mut String, out: &mut String) {
    let text = collapse(inline);
    inline.clear();
    let text = text.trim();
    if !text.is_empty() {
        push_block(out, text);
    }
}

fn push_block(out: &mut String, text: &str) {
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    out.push_str(text);
}

fn write_block(el: &Element, out: &mut String, depth: usize) {
    match el.tag.as_str() {
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            let level = el.tag.as_bytes()[1] - b'0';
            let text = inline_of(&el.children);
            let text = text.trim();
            if !text.is_empty() {
                push_block(out, &format!("{} {text}", "#".repeat(level as usize)));
            }
        }
        "hr" => push_block(out, "---"),
        "pre" => write_pre(el, out),
        "blockquote" => {
            let mut inner = String::new();
            write_blocks(&el.children, &mut inner, depth);
            let quoted: Vec<String> = inner
                .lines()
                .map(|l| {
                    if l.is_empty() {
                        ">".to_string()
                    } else {
                        format!("> {l}")
                    }
                })
                .collect();
            if !quoted.is_empty() {
                push_block(out, &quoted.join("\n"));
            }
        }
        "ul" | "ol" => write_list(el, out, depth),
        "table" => write_table(el, out),
        // * A definition list has no Markdown of its own: the term becomes a
        //   bold line and the definitions sit under it.
        "dl" => {
            for child in &el.children {
                if let Node::Element(e) = child {
                    match e.tag.as_str() {
                        "dt" => {
                            let t = inline_of(&e.children);
                            if !t.trim().is_empty() {
                                push_block(out, &format!("**{}**", t.trim()));
                            }
                        }
                        "dd" => {
                            let t = inline_of(&e.children);
                            if !t.trim().is_empty() {
                                push_block(out, t.trim());
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        "li" => {
            // * An `li` outside a list: write it as a bullet rather than
            //   dropping the marker.
            let text = inline_of(&el.children);
            if !text.trim().is_empty() {
                push_block(out, &format!("- {}", text.trim()));
            }
        }
        _ => {
            // * `p`, `div`, `section` and the rest: their content is blocks of
            //   its own. A paragraph is one line.
            let mut inner = String::new();
            write_blocks(&el.children, &mut inner, depth);
            let inner = inner.trim();
            if !inner.is_empty() {
                push_block(out, inner);
            }
        }
    }
}

fn write_pre(el: &Element, out: &mut String) {
    // * `<pre><code class="language-rust">` names the fence's language.
    let mut lang = String::new();
    if let Some(Node::Element(code)) = el
        .children
        .iter()
        .find(|n| matches!(n, Node::Element(e) if e.tag == "code"))
    {
        if let Some(class) = code.attr("class") {
            if let Some(name) = class.strip_prefix("language-") {
                lang = name.split_whitespace().next().unwrap_or("").to_string();
            }
        }
    }
    let mut body = raw_text(&el.children);
    while body.ends_with('\n') {
        body.pop();
    }
    // * A fence must be longer than any run of backticks inside the body, or
    //   the code closes the fence and the rest leaks out as Markdown.
    let fence = "`".repeat(longest_backtick_run(&body).max(2) + 1);
    push_block(out, &format!("{fence}{lang}\n{body}\n{fence}"));
}

fn write_list(el: &Element, out: &mut String, depth: usize) {
    let ordered = el.tag == "ol";
    let mut items: Vec<String> = Vec::new();
    for child in &el.children {
        let Node::Element(li) = child else { continue };
        if li.tag != "li" {
            continue;
        }
        let mut body = String::new();
        // * The item's own text comes first, then any nested list indented
        //   under it.
        let mut inline = String::new();
        for node in &li.children {
            match node {
                Node::Text(t) => inline.push_str(t),
                Node::Element(e) if e.tag == "ul" || e.tag == "ol" => {
                    flush_inline_in_place(&mut inline, &mut body);
                    let mut nested = String::new();
                    write_list(e, &mut nested, depth + 1);
                    if !nested.trim().is_empty() {
                        if !body.is_empty() {
                            body.push('\n');
                        }
                        for line in nested.lines() {
                            body.push_str("    ");
                            body.push_str(line);
                            body.push('\n');
                        }
                        while body.ends_with('\n') {
                            body.pop();
                        }
                    }
                }
                Node::Element(e) if is_block(&e.tag) => {
                    flush_inline_in_place(&mut inline, &mut body);
                    let mut inner = String::new();
                    write_block(e, &mut inner, depth + 1);
                    if !inner.trim().is_empty() {
                        if !body.is_empty() {
                            body.push('\n');
                        }
                        body.push_str(inner.trim());
                    }
                }
                Node::Element(e) => write_inline(e, &mut inline),
            }
        }
        flush_inline_in_place(&mut inline, &mut body);
        let body = body.trim();
        if body.is_empty() {
            continue;
        }
        let marker = if ordered { "1." } else { "-" };
        let mut lines = body.lines();
        let first = lines.next().unwrap_or("");
        let mut item = format!("{marker} {first}");
        // * A continuation line is indented to the marker's width, but a
        //   line that already starts with spaces is a nested list's own
        //   indent and keeps it.
        for line in lines {
            item.push('\n');
            if line.starts_with(' ') {
                item.push_str(line);
            } else {
                item.push_str("   ");
                item.push_str(line);
            }
        }
        items.push(item);
    }
    if !items.is_empty() {
        push_block(out, &items.join("\n"));
    }
}

/// The list-item counterpart of [`flush_inline`], appending into a body
/// rather than emitting a block.
fn flush_inline_in_place(inline: &mut String, body: &mut String) {
    let text = collapse(inline);
    inline.clear();
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    if !body.is_empty() {
        body.push('\n');
    }
    body.push_str(text);
}

fn write_table(el: &Element, out: &mut String) {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut header_rows = 0usize;
    for section in &el.children {
        match section {
            Node::Element(s) if s.tag == "thead" || s.tag == "tbody" || s.tag == "tfoot" => {
                for row in &s.children {
                    if let Node::Element(tr) = row {
                        if tr.tag == "tr" {
                            let cells = table_row(tr);
                            if s.tag == "thead" && header_rows == rows.len() {
                                header_rows += 1;
                            }
                            rows.push(cells);
                        }
                    }
                }
            }
            Node::Element(tr) if tr.tag == "tr" => rows.push(table_row(tr)),
            _ => {}
        }
    }
    rows.retain(|r| !r.is_empty());
    if rows.is_empty() {
        return;
    }
    let cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    if cols == 0 {
        return;
    }
    // * A `th` row is the header even without a `<thead>`.
    let has_th = el
        .children
        .iter()
        .any(|n| matches!(n, Node::Element(s) if s.tag == "thead"));
    let header_at = if has_th { 0 } else { header_rows };
    let mut lines: Vec<String> = Vec::new();
    for (ix, row) in rows.iter().enumerate() {
        let mut cells = row.clone();
        cells.resize(cols, String::new());
        let joined = cells
            .iter()
            .map(|c| pipe_safe(c.trim()))
            .collect::<Vec<_>>()
            .join(" | ");
        lines.push(format!("| {joined} |"));
        if ix == header_at {
            lines.push(format!(
                "| {} |",
                (0..cols).map(|_| "---").collect::<Vec<_>>().join(" | ")
            ));
        }
    }
    push_block(out, &lines.join("\n"));
}

fn table_row(tr: &Element) -> Vec<String> {
    tr.children
        .iter()
        .filter_map(|n| match n {
            Node::Element(td) if td.tag == "td" || td.tag == "th" => Some(inline_of(&td.children)),
            _ => None,
        })
        .collect()
}

/// A cell's text with the pipe escaped, which is the one character GFM
/// tables cannot hold raw.
fn pipe_safe(cell: &str) -> String {
    cell.replace('|', "\\|")
}

/// Writes an inline element into `out`.
fn write_inline(el: &Element, out: &mut String) {
    match el.tag.as_str() {
        "br" => out.push(BREAK),
        "strong" | "b" => wrap(el, out, "**"),
        "em" | "i" => wrap(el, out, "*"),
        "del" | "s" | "strike" => wrap(el, out, "~~"),
        "code" | "kbd" | "samp" | "tt" => {
            let text = raw_text(&el.children);
            let text = text.trim();
            if !text.is_empty() {
                // * Same rule as a block fence, one level down: one backtick
                //   more than the longest run inside, and a space on each side
                //   when the text itself starts or ends with one.
                let fence = "`".repeat(longest_backtick_run(text) + 1);
                let pad = if text.starts_with('`') || text.ends_with('`') {
                    " "
                } else {
                    ""
                };
                out.push_str(&format!("{fence}{pad}{text}{pad}{fence}"));
            }
        }
        "a" => {
            let text = collapsed_inline(&el.children);
            let href = el.attr("href").unwrap_or("").trim();
            if text.is_empty() {
                return;
            }
            if href.is_empty() || href.starts_with('#') || is_unsafe_target(href) {
                out.push_str(&text);
            } else if is_image_target(href) && text == href {
                out.push_str(&format!("![]({})", escape_target(href)));
            } else {
                out.push_str(&format!("[{}]({})", text, escape_target(href)));
            }
        }
        "img" => {
            let src = el.attr("src").unwrap_or("").trim();
            if src.is_empty() {
                return;
            }
            let alt = el.attr("alt").unwrap_or("").trim();
            out.push_str(&format!("![{}]({})", alt, escape_target(src)));
        }
        "sup" => {
            // * Markdown has no superscript; the caret reads the same in text.
            let text = collapsed_inline(&el.children);
            if !text.is_empty() {
                out.push_str(&format!("^{text}^"));
            }
        }
        "sub" => {
            let text = collapsed_inline(&el.children);
            if !text.is_empty() {
                out.push_str(&format!("~{text}~"));
            }
        }
        "input" => {
            if el.attr("type") == Some("checkbox") {
                let checked = el.attrs.iter().any(|(k, _)| k == "checked");
                out.push_str(if checked { "[x] " } else { "[ ] " });
            }
        }
        _ => {
            // * A span, a font, a label: its text, without the wrapper.
            for child in &el.children {
                match child {
                    Node::Text(t) => out.push_str(t),
                    Node::Element(e) if is_block(&e.tag) => {
                        out.push('\n');
                        out.push_str(&collapsed_inline(&e.children));
                        out.push('\n');
                    }
                    Node::Element(e) => write_inline(e, out),
                }
            }
        }
    }
}

fn wrap(el: &Element, out: &mut String, marker: &str) {
    let text = collapsed_inline(&el.children);
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    // * Leading or trailing spaces inside emphasis are lost by every Markdown
    //   parser; move them outside the markers.
    let lead =
        collapsed_inline(&el.children).len() - collapsed_inline(&el.children).trim_start().len();
    let spaced = " ".repeat(lead);
    out.push_str(&spaced);
    out.push_str(marker);
    out.push_str(text);
    out.push_str(marker);
}

fn is_image_target(href: &str) -> bool {
    let lower = href.to_ascii_lowercase();
    [".png", ".jpg", ".jpeg", ".gif", ".webp", ".svg", ".bmp"]
        .iter()
        .any(|ext| lower.contains(ext))
}

/// A link or image target: spaces and parentheses would end the target
/// early, so they are escaped or encoded.
/// Whether a URL carries a scheme that must not survive into a Markdown link.
///
/// Schemes are case-insensitive (RFC 3986), so `JaVaScRiPt:` is the same
/// scheme as `javascript:`. `data:` is refused too: a `data:text/html` link
/// is a document in a wrapper, and nothing a clipboard needs to carry.
fn is_unsafe_target(href: &str) -> bool {
    let scheme = match href.split_once(':') {
        Some((scheme, _)) => scheme.trim().to_ascii_lowercase(),
        None => return false,
    };
    // * A colon before any `/`, `?` or `#` opens a scheme; anything else is a
    //   path segment that happens to contain a colon.
    if scheme.contains('/') || scheme.contains('?') || scheme.contains('#') {
        return false;
    }
    matches!(scheme.as_str(), "javascript" | "vbscript" | "data")
}

fn escape_target(href: &str) -> String {
    href.replace(' ', "%20").replace(')', "%29")
}

/// The longest run of consecutive backticks in `text`.
fn longest_backtick_run(text: &str) -> usize {
    let mut longest = 0;
    let mut run = 0;
    for ch in text.chars() {
        if ch == '`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    longest
}

/// The plain text of `nodes`, with inline markup flattened.
fn inline_of(nodes: &[Node]) -> String {
    let mut out = String::new();
    for node in nodes {
        match node {
            Node::Text(t) => out.push_str(t),
            Node::Element(e) if e.tag == "br" => out.push(' '),
            Node::Element(e) => out.push_str(&inline_of(&e.children)),
        }
    }
    collapse(&out)
}

/// The same, with the inline markup kept: what goes inside an emphasis or a
/// link.
fn collapsed_inline(nodes: &[Node]) -> String {
    let mut out = String::new();
    for node in nodes {
        match node {
            Node::Text(t) => out.push_str(t),
            Node::Element(e) => write_inline(e, &mut out),
        }
    }
    collapse(&out)
}

/// The text of `nodes` verbatim: `<pre>` keeps its spaces and newlines.
fn raw_text(nodes: &[Node]) -> String {
    let mut out = String::new();
    for node in nodes {
        match node {
            Node::Text(t) => out.push_str(t),
            Node::Element(e) => out.push_str(&raw_text(&e.children)),
        }
    }
    out
}

/// The character `<br>` leaves behind: a newline the source cannot spell,
/// so collapsing the page's own whitespace cannot eat it.
const BREAK: char = '\u{0}';

/// Runs of whitespace become one space, the way a browser lays text out.
/// The page's own newlines are whitespace too — a paragraph does not break
/// because its source did — but a [`BREAK`] is a real line break and is
/// turned back into a newline here.
fn collapse(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = false;
    for ch in s.chars() {
        if ch == BREAK {
            out.push('\n');
            space = true;
        } else if ch.is_whitespace() {
            if !space {
                out.push(' ');
                space = true;
            }
        } else {
            out.push(ch);
            space = false;
        }
    }
    out
}

/// Trims every line, drops the blank ones a page's whitespace left behind,
/// and keeps exactly one blank line between blocks.
fn tidy(s: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut blank = false;
    for line in s.lines() {
        let line = line.trim_end();
        if line.trim().is_empty() {
            if !out.is_empty() {
                blank = true;
            }
            continue;
        }
        if blank && !out.is_empty() {
            out.push(String::new());
            blank = false;
        }
        out.push(line.to_string());
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn md(html: &str) -> String {
        convert(html).expect("markup was not recognised")
    }

    #[test]
    fn headings_and_paragraphs() {
        assert_eq!(md("<h1>Title</h1><p>Body.</p>"), "# Title\n\nBody.");
        assert_eq!(md("<h3>Deep</h3>"), "### Deep");
    }

    #[test]
    fn emphasis_and_code() {
        assert_eq!(
            md("<p><strong>bold</strong> and <em>it</em> and <code>x = 1</code></p>"),
            "**bold** and *it* and `x = 1`"
        );
    }

    #[test]
    fn links_keep_their_target() {
        assert_eq!(
            md(r#"<p><a href="https://example.com/a">text</a></p>"#),
            "[text](https://example.com/a)"
        );
        // * An anchor to nowhere, or a script, keeps only its text.
        assert_eq!(md(r##"<p><a href="#top">top</a></p>"##), "top");
        assert_eq!(md(r##"<p><a href="javascript:x()">bad</a></p>"##), "bad");
    }

    #[test]
    fn every_void_element_stops_where_it_ends() {
        // ! A void element must not eat what follows it. The second paragraph
        //   is the canary: it only survives if `<img>`, `<input>`, `<wbr>` and
        //   the rest are recognised as childless.
        assert_eq!(
            md(r##"<p>first</p><img src="p.png" alt="p"><p>second</p>"##),
            "first\n\n![p](p.png)\n\nsecond"
        );
        for void in [
            "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param",
            "source", "track", "wbr",
        ] {
            let html = format!("<div><{void}><p>after</p></div>");
            let out = md(&html);
            assert!(
                out.contains("after"),
                "<{void}> swallowed the paragraph after it: {out:?}"
            );
        }
    }

    #[test]
    fn dangerous_url_schemes_are_refused_whatever_their_case() {
        for scheme in ["javascript", "JAVASCRIPT", "JaVaScRiPt", "vbscript", "data"] {
            let html = format!(r##"<a href="{scheme}:alert(1)">click</a>"##);
            assert_eq!(md(&html), "click", "{scheme} survived as a link");
        }
        // * An ordinary scheme and a path containing a colon are untouched.
        assert_eq!(
            md(r##"<a href="HTTPS://example.com/a:b">x</a>"##),
            "[x](HTTPS://example.com/a:b)"
        );
    }

    #[test]
    fn deep_nesting_does_not_blow_the_stack() {
        // * Far past MAX_DEPTH; a recursive parser without a limit dies here.
        let mut html = String::new();
        for _ in 0..5_000 {
            html.push_str("<div>");
        }
        html.push_str("deep");
        for _ in 0..5_000 {
            html.push_str("</div>");
        }
        // * Nothing beyond the limit is kept, so there is nothing to convert —
        //   `None` is the honest answer, and the point is that it returns.
        let out = convert(&html).unwrap_or_default();
        assert!(out.len() < 64, "the limit did not hold: {out:?}");
        // * Shallow-but-deep input still converts: the limit is not a hair
        //   trigger.
        let mut shallow = String::new();
        for _ in 0..40 {
            shallow.push_str("<div>");
        }
        shallow.push_str("kept");
        for _ in 0..40 {
            shallow.push_str("</div>");
        }
        assert_eq!(convert(&shallow).unwrap_or_default(), "kept");
    }

    #[test]
    fn a_fence_outgrows_the_backticks_it_holds() {
        // * Three backticks in the body would close a three-backtick fence.
        assert_eq!(
            md("<pre><code>a```b</code></pre>"),
            "````\na```b\n````",
            "the fence must be longer than the run inside it"
        );
        assert_eq!(md("<pre>plain</pre>"), "```\nplain\n```");
        // * Inline code likewise, and it pads when the text touches a backtick.
        assert_eq!(md("<code>a``b</code>"), "```a``b```");
        assert_eq!(md("<code>`x`</code>"), "`` `x` ``");
    }

    #[test]
    fn lists_nest() {
        assert_eq!(
            md("<ul><li>one</li><li>two<ul><li>inner</li></ul></li></ul>"),
            "- one\n- two\n    - inner"
        );
        assert_eq!(
            md("<ol><li>first</li><li>second</li></ol>"),
            "1. first\n1. second"
        );
    }

    #[test]
    fn pre_keeps_its_language_and_whitespace() {
        assert_eq!(
            md("<pre><code class=\"language-rust\">fn main() {\n    1\n}</code></pre>"),
            "```rust\nfn main() {\n    1\n}\n```"
        );
        // * No language: a bare fence.
        assert_eq!(md("<pre>plain</pre>"), "```\nplain\n```");
    }

    #[test]
    fn quote_is_prefixed_per_line() {
        assert_eq!(
            md("<blockquote><p>one</p><p>two</p></blockquote>"),
            "> one\n>\n> two"
        );
    }

    #[test]
    fn table_becomes_gfm() {
        assert_eq!(
            md("<table><thead><tr><th>A</th><th>B</th></tr></thead>\
                <tbody><tr><td>1</td><td>2</td></tr></tbody></table>"),
            "| A | B |\n| --- | --- |\n| 1 | 2 |"
        );
        // * A pipe in a cell is escaped, or the row would gain a column.
        assert_eq!(
            md("<table><tr><th>A</th></tr><tr><td>a|b</td></tr></table>"),
            "| A |\n| --- |\n| a\\|b |"
        );
    }

    #[test]
    fn images_and_breaks() {
        assert_eq!(md(r#"<p><img src="a.png" alt="pic"></p>"#), "![pic](a.png)");
        assert_eq!(md("<p>one<br>two</p>"), "one\ntwo");
        assert_eq!(md("<p>a</p><hr><p>b</p>"), "a\n\n---\n\nb");
    }

    #[test]
    fn entities_are_decoded() {
        assert_eq!(md("<p>a &amp; b &lt; c &#8212; d</p>"), "a & b < c — d");
        // * An entity the table does not know stays as written.
        assert_eq!(md("<p>&weird;</p>"), "&weird;");
    }

    #[test]
    fn unknown_elements_are_unwrapped_not_dropped() {
        assert_eq!(
            md(r#"<div class="x"><span style="color:red">kept</span></div>"#),
            "kept"
        );
    }

    #[test]
    fn scripts_and_styles_never_reach_the_output() {
        assert_eq!(
            md("<style>p{color:red}</style><script>x()</script><p>only</p>"),
            "only"
        );
    }

    #[test]
    fn a_plain_string_is_not_converted() {
        // * A page that puts its plain copy on the clipboard as text/html.
        assert_eq!(convert("just some text"), None);
        assert_eq!(convert(""), None);
        assert_eq!(convert("   \n  "), None);
    }

    #[test]
    fn whitespace_collapses_the_way_a_browser_draws_it() {
        assert_eq!(md("<p>a\n      b</p>"), "a b");
        // * But a <br> is a real break.
        assert_eq!(md("<p>a<br>\n     b</p>"), "a\nb");
    }

    #[test]
    fn a_page_is_read_for_more_than_its_text() {
        let html = r#"<title>Page Title</title><h1>Heading</h1><p>Body.</p><img src="pics/a.png">"#;
        let page = read(html).expect("markup was not recognised");
        // * The declared title wins over the heading the body opens with.
        assert_eq!(page.title.as_deref(), Some("Page Title"));
        assert_eq!(page.markdown, "# Heading\n\nBody.\n\n![](pics/a.png)");
        assert_eq!(page.assets, vec!["pics/a.png".to_string()]);
    }

    #[test]
    fn the_title_falls_back_to_the_first_heading() {
        let page = read("<h2>Section</h2><p>text</p>").expect("markup was recognised");
        assert_eq!(page.title.as_deref(), Some("Section"));
        // * A heading with no text is not a title, and neither is nothing.
        assert_eq!(read("<h1></h1><p>text</p>").unwrap().title, None);
        assert_eq!(read("<p>text</p>").unwrap().title, None);
        // * A `<title>` with attributes is still a title.
        assert_eq!(
            read(r#"<title id="x">Named</title><p>text</p>"#)
                .unwrap()
                .title
                .as_deref(),
            Some("Named")
        );
    }

    #[test]
    fn the_assets_are_the_local_files_in_document_order() {
        let html = r#"<div><img src="pictures/a.png"><img src="../files/b.png"></div>
            <p><img src="pictures/a.png"></p>
            <img src="https://example.com/c.png">
            <img src="//cdn.example.com/d.png">"#;
        let page = read(html).expect("markup was recognised");
        // * Once each, spelled the way the markup spells it. A remote
        //   target stays in the Markdown and is not a file to copy.
        assert_eq!(
            page.assets,
            vec!["pictures/a.png".to_string(), "../files/b.png".to_string()]
        );
    }

    #[test]
    fn a_page_of_one_image_still_names_its_file() {
        let page = read(r#"<img src="only.png">"#).expect("markup was recognised");
        assert_eq!(page.assets, vec!["only.png".to_string()]);
        assert_eq!(page.title, None);
    }

    #[test]
    fn a_page_with_nothing_to_say_is_not_a_page() {
        // * A document whose only content is a title has no body to import,
        //   so there is nothing to paste either.
        assert_eq!(convert("<title>Only a title</title>"), None);
        // * A plain string, an empty document: not markup at all.
        assert_eq!(read("just text"), None);
        assert_eq!(read("<title> </title>"), None);
        assert_eq!(read("<style>a{}</style>"), None);
    }
}
