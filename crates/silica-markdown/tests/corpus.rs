//! Obsidian conformance corpus.
//!
//! `tests/corpus/vault` is an Obsidian vault of 1,000 generated notes plus
//! hand-written edge cases; `tests/corpus/expected` holds the HTML Obsidian
//! 1.13.7 produced for each note through the `silica-corpus` plugin inside
//! the vault (open the vault in Obsidian, trust the plugin, and run its
//! "Render corpus" command to regenerate). Both sides are reduced to a
//! canonical token stream before comparison so that runtime chrome Obsidian
//! adds (copy buttons, MathJax output, icons, `dir="auto"`) does not count.
//!
//! The pass count is a ratchet: raise `MIN_PASSING` when it grows.
//! `CORPUS_VERBOSE=1` prints the first difference of every failing note;
//! `CORPUS_NOTE=<path substring>` prints the full canonical forms of one note;
//! `CORPUS_DUMP=<dir>` writes `<dir>/<note>.want` and `.got` for failing notes;
//! `CORPUS_DIR=<dir>` runs the whole comparison over another `vault/` +
//! `expected/` pair instead, which is how `tools/probe-gen` triages candidate
//! probes. Both ratchets are off for such a run.

use silica_markdown::{parse, render_html};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

const MIN_PASSING: usize = 1008;

/// `tests/corpus`, or whatever `CORPUS_DIR` names. A staging corpus of
/// candidate probes (`tools/probe-gen`) is the same `vault/` + `expected/`
/// pair, and triaging one must use this comparator rather than a second
/// implementation of it: canonicalizing Obsidian's HTML is subtle enough that
/// a second one would disagree with the gate it is meant to predict.
fn corpus_dir() -> PathBuf {
    match std::env::var_os("CORPUS_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus"),
    }
}

/// Whether this run is over a staging corpus. Such a run has its own note
/// count and no history, so neither gate below applies to it: it is a report,
/// not a ratchet.
fn staging() -> bool {
    std::env::var_os("CORPUS_DIR").is_some()
}

fn notes(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with('.'))
        {
            continue;
        }
        if path.is_dir() {
            notes(&path, out);
        } else if path.extension().is_some_and(|e| e == "md") {
            out.push(path);
        }
    }
}

// ---------------------------------------------------------------------------
// HTML canonicalization

#[derive(Debug)]
enum Tok {
    Start {
        name: String,
        attrs: Vec<(String, String)>,
    },
    End(String),
    Text(String),
}

fn tokenize(html: &str) -> Vec<Tok> {
    let b = html.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut text_start = 0;
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        let rest = &html[i..];
        if rest.starts_with("<!--") {
            flush_text(html, text_start, i, &mut out);
            let end = rest.find("-->").map(|e| i + e + 3).unwrap_or(b.len());
            i = end;
            text_start = i;
            continue;
        }
        if rest.starts_with("</") {
            let end = rest.find('>').map(|e| i + e + 1).unwrap_or(b.len());
            flush_text(html, text_start, i, &mut out);
            let name = html[i + 2..end - 1].trim().to_ascii_lowercase();
            out.push(Tok::End(name));
            i = end;
            text_start = i;
            continue;
        }
        if !b.get(i + 1).is_some_and(|c| c.is_ascii_alphabetic()) {
            i += 1;
            continue;
        }
        flush_text(html, text_start, i, &mut out);
        let mut j = i + 1;
        while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'-') {
            j += 1;
        }
        let name = html[i + 1..j].to_ascii_lowercase();
        let mut attrs = Vec::new();
        loop {
            while j < b.len() && b[j].is_ascii_whitespace() {
                j += 1;
            }
            if j >= b.len() {
                break;
            }
            if b[j] == b'>' {
                j += 1;
                break;
            }
            if b[j] == b'/' {
                j += 1;
                continue;
            }
            let ns = j;
            while j < b.len() && !b[j].is_ascii_whitespace() && !matches!(b[j], b'=' | b'>' | b'/')
            {
                j += 1;
            }
            let aname = html[ns..j].to_ascii_lowercase();
            while j < b.len() && b[j].is_ascii_whitespace() {
                j += 1;
            }
            let mut value = String::new();
            if b.get(j) == Some(&b'=') {
                j += 1;
                while j < b.len() && b[j].is_ascii_whitespace() {
                    j += 1;
                }
                match b.get(j) {
                    Some(&q) if q == b'"' || q == b'\'' => {
                        let vs = j + 1;
                        let ve = html[vs..]
                            .find(q as char)
                            .map(|e| vs + e)
                            .unwrap_or(b.len());
                        value = decode_entities(&html[vs..ve]);
                        j = (ve + 1).min(b.len());
                    }
                    _ => {
                        let vs = j;
                        while j < b.len() && !b[j].is_ascii_whitespace() && b[j] != b'>' {
                            j += 1;
                        }
                        value = decode_entities(&html[vs..j]);
                    }
                }
            }
            if !aname.is_empty() {
                attrs.push((aname, value));
            }
        }
        out.push(Tok::Start { name, attrs });
        i = j;
        text_start = i;
    }
    flush_text(html, text_start, b.len(), &mut out);
    out
}

fn flush_text(html: &str, start: usize, end: usize, out: &mut Vec<Tok>) {
    if start < end {
        out.push(Tok::Text(decode_entities(&html[start..end])));
    }
}

fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let after = &rest[i..];
        let end = after.find(';').filter(|&e| e <= 10);
        match end {
            Some(e) => {
                let name = &after[1..e];
                let decoded = match name {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "apos" => Some('\''),
                    "nbsp" => Some('\u{a0}'),
                    _ if name.starts_with("#x") || name.starts_with("#X") => {
                        u32::from_str_radix(&name[2..], 16)
                            .ok()
                            .and_then(char::from_u32)
                    }
                    _ if name.starts_with('#') => {
                        name[1..].parse::<u32>().ok().and_then(char::from_u32)
                    }
                    _ => None,
                };
                match decoded {
                    Some(c) => {
                        out.push(c);
                        rest = &after[e + 1..];
                    }
                    None => {
                        out.push('&');
                        rest = &after[1..];
                    }
                }
            }
            None => {
                out.push('&');
                rest = &after[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn is_void(name: &str) -> bool {
    matches!(
        name,
        "br" | "hr"
            | "img"
            | "input"
            | "wbr"
            | "source"
            | "col"
            | "meta"
            | "link"
            | "area"
            | "base"
            | "embed"
            | "param"
            | "track"
            | "path"
            | "rect"
            | "circle"
            | "line"
            | "polyline"
            | "polygon"
    )
}

/// Elements Obsidian's sanitizer keeps; anything else is unwrapped.
const KNOWN_TAGS: &[&str] = &[
    "a",
    "abbr",
    "b",
    "blockquote",
    "br",
    "button",
    "caption",
    "code",
    "col",
    "colgroup",
    "dd",
    "del",
    "details",
    "div",
    "dl",
    "dt",
    "em",
    "figcaption",
    "figure",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "hr",
    "i",
    "iframe",
    "img",
    "input",
    "ins",
    "kbd",
    "li",
    "mark",
    "ol",
    "p",
    "pre",
    "q",
    "s",
    "section",
    "small",
    "span",
    "strong",
    "sub",
    "summary",
    "sup",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "tr",
    "u",
    "ul",
    "video",
    "audio",
    "source",
    "svg",
    "path",
    "rect",
    "circle",
    "line",
    "polyline",
    "polygon",
    "mjx-container",
    "mjx-math",
    "script",
    "label",
    "var",
    "samp",
    "cite",
    "dfn",
    "time",
    "bdi",
    "bdo",
    "wbr",
    "center",
    "font",
    "big",
    "tt",
    "strike",
    "address",
    "article",
    "aside",
    "footer",
    "header",
    "main",
    "nav",
    "hgroup",
    "picture",
    "ruby",
    "rt",
    "rp",
    "progress",
    "meter",
    "output",
    "fieldset",
    "legend",
    "select",
    "option",
    "optgroup",
    "textarea",
    "form",
];

/// Block-level elements that close an open `<p>` when the browser parses them.
const BLOCK_TAGS: &[&str] = &[
    "div",
    "p",
    "ul",
    "ol",
    "table",
    "pre",
    "blockquote",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "hr",
    "section",
    "dl",
    "address",
    "article",
    "aside",
    "footer",
    "header",
    "main",
    "nav",
    "figure",
    "details",
    "fieldset",
    "form",
];

const DROPPED_ATTRS: &[&str] = &[
    "dir",
    "style",
    "data-callout-metadata",
    "data-tooltip-position",
    "aria-label",
    "referrerpolicy",
    "data-footref",
    "target",
    "rel",
    "data-line",
    "sandbox",
];

const DROPPED_CLASSES: &[&str] = &[
    "is-loaded",
    "node-insert-event",
    "markdown-embed",
    "inline-embed",
    "image-embed",
    "file-embed",
    "mod-empty-attachment",
    "mod-generic",
    "media-embed",
    "pdf-embed",
    "canvas-embed",
];

enum Mode {
    Emit,
    Skip,
    Unwrap,
    Math {
        tag: String,
        class: String,
        latex: Option<String>,
        text: String,
    },
    Embed,
}

struct Frame {
    name: String,
    mode: Mode,
}

fn canonical_start(name: &str, attrs: &[(String, String)]) -> Option<String> {
    let mut kept: Vec<(String, String)> = Vec::new();
    for (k, v) in attrs {
        if DROPPED_ATTRS.contains(&k.as_str()) {
            continue;
        }
        let v = match k.as_str() {
            "class" => {
                let mut classes: Vec<&str> = v
                    .split_whitespace()
                    .filter(|c| !DROPPED_CLASSES.contains(c))
                    .collect();
                classes.sort_unstable();
                classes.dedup();
                if classes.is_empty() {
                    continue;
                }
                classes.join(" ")
            }
            "id" | "href" | "data-footnote-id" => strip_footnote_hash(v),
            "data-callout-fold" if v.is_empty() => continue,
            _ => v.clone(),
        };
        kept.push((k.clone(), v));
    }
    kept.sort();
    let mut s = format!("<{name}");
    for (k, v) in kept {
        s.push(' ');
        s.push_str(&k);
        s.push_str("=\"");
        s.push_str(&v);
        s.push('"');
    }
    s.push('>');
    Some(s)
}

/// `fn-1-0723cf36e9df043a` -> `fn-1`.
fn strip_footnote_hash(v: &str) -> String {
    let mut out = String::new();
    let mut rest = v;
    while let Some(i) = rest.find("fn") {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let prefix_len = if tail.starts_with("fnref-") {
            6
        } else if tail.starts_with("fn-") {
            3
        } else {
            out.push_str("fn");
            rest = &tail[2..];
            continue;
        };
        let digits = tail[prefix_len..]
            .bytes()
            .take_while(|b| b.is_ascii_digit())
            .count();
        let mut end = prefix_len + digits;
        let hash = &tail[end..];
        if hash.starts_with('-')
            && hash.len() >= 17
            && hash[1..17].bytes().all(|b| b.is_ascii_hexdigit())
        {
            end += 17;
        }
        out.push_str(&tail[..prefix_len + digits]);
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

fn class_list(attrs: &[(String, String)]) -> Vec<String> {
    attrs
        .iter()
        .find(|(k, _)| k == "class")
        .map(|(_, v)| v.split_whitespace().map(str::to_string).collect())
        .unwrap_or_default()
}

fn attr<'a>(attrs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
}

/// Reduces HTML to a canonical line-per-token form.
fn canonicalize(html: &str) -> Vec<String> {
    let toks = tokenize(html);
    let mut out: Vec<String> = Vec::new();
    let mut stack: Vec<Frame> = Vec::new();
    let mut pre_depth = 0usize;
    let mut text = String::new();

    fn flush(text: &mut String, out: &mut Vec<String>, pre: bool) {
        if text.is_empty() {
            return;
        }
        if pre {
            out.push(format!("text:{}", text));
        } else {
            let collapsed: Vec<&str> = text.split_whitespace().collect();
            if !collapsed.is_empty() {
                out.push(format!("text:{}", collapsed.join(" ")));
            }
        }
        text.clear();
    }

    for tok in toks {
        let skipping = stack
            .iter()
            .any(|f| matches!(f.mode, Mode::Skip | Mode::Embed));
        let in_math = stack.iter().any(|f| matches!(f.mode, Mode::Math { .. }));
        match tok {
            Tok::Start { name, attrs } => {
                let void = is_void(&name);
                if skipping {
                    if !void {
                        stack.push(Frame {
                            name,
                            mode: Mode::Skip,
                        });
                    }
                    continue;
                }
                if in_math {
                    if name == "mjx-math" {
                        if let Some(Frame {
                            mode: Mode::Math { latex, .. },
                            ..
                        }) = stack
                            .iter_mut()
                            .rev()
                            .find(|f| matches!(f.mode, Mode::Math { .. }))
                        {
                            *latex = attr(&attrs, "data-latex").map(str::to_string);
                        }
                    }
                    if !void {
                        stack.push(Frame {
                            name,
                            mode: Mode::Unwrap,
                        });
                    }
                    continue;
                }
                let classes = class_list(&attrs);
                let has = |c: &str| classes.iter().any(|x| x == c);
                if (name == "span" && has("token")) || !KNOWN_TAGS.contains(&name.as_str()) {
                    stack.push(Frame {
                        name,
                        mode: Mode::Unwrap,
                    });
                    continue;
                }
                // A block element inside <p> closes the paragraph, as the browser does.
                if BLOCK_TAGS.contains(&name.as_str()) && name != "p" {
                    if let Some(pos) = stack
                        .iter()
                        .rposition(|f| f.name == "p" && matches!(f.mode, Mode::Emit))
                    {
                        if stack[pos + 1..]
                            .iter()
                            .all(|f| matches!(f.mode, Mode::Unwrap))
                        {
                            flush(&mut text, &mut out, pre_depth > 0);
                            out.push("</p>".to_string());
                            stack[pos].mode = Mode::Unwrap;
                            stack[pos].name = "p-closed".to_string();
                        }
                    }
                }
                let drop = name == "svg"
                    || name == "script"
                    || (name == "button" && has("copy-code-button"))
                    || has("callout-icon")
                    || has("callout-fold")
                    || (name == "pre" && has("frontmatter"))
                    || name == "mjx-container";
                if drop {
                    if !void {
                        stack.push(Frame {
                            name,
                            mode: Mode::Skip,
                        });
                    }
                    continue;
                }
                flush(&mut text, &mut out, pre_depth > 0);
                if has("math") && (name == "span" || name == "div") {
                    let class = if has("math-block") {
                        "math math-block"
                    } else {
                        "math math-inline"
                    };
                    stack.push(Frame {
                        name: name.clone(),
                        mode: Mode::Math {
                            tag: name,
                            class: class.to_string(),
                            latex: None,
                            text: String::new(),
                        },
                    });
                    continue;
                }
                if has("internal-embed") {
                    let alt = attr(&attrs, "alt").unwrap_or("");
                    let src = attr(&attrs, "src").unwrap_or("");
                    out.push(format!(
                        "<span alt=\"{alt}\" class=\"internal-embed\" src=\"{src}\">"
                    ));
                    stack.push(Frame {
                        name,
                        mode: Mode::Embed,
                    });
                    continue;
                }
                if let Some(line) = canonical_start(&name, &attrs) {
                    out.push(line);
                }
                if name == "pre" {
                    pre_depth += 1;
                }
                if !void {
                    stack.push(Frame {
                        name,
                        mode: Mode::Emit,
                    });
                }
            }
            Tok::End(name) => {
                if name == "p" && !stack.iter().any(|f| f.name == "p") {
                    if let Some(pos) = stack.iter().rposition(|f| f.name == "p-closed") {
                        stack.drain(pos..);
                        flush(&mut text, &mut out, pre_depth > 0);
                        out.push("<p>".to_string());
                        out.push("</p>".to_string());
                        continue;
                    }
                }
                let Some(pos) = stack.iter().rposition(|f| f.name == name) else {
                    continue;
                };
                let frames: Vec<Frame> = stack.drain(pos..).collect();
                let frame = frames.into_iter().next().unwrap();
                match frame.mode {
                    Mode::Emit => {
                        flush(&mut text, &mut out, pre_depth > 0);
                        if name == "pre" {
                            pre_depth = pre_depth.saturating_sub(1);
                        }
                        out.push(format!("</{name}>"));
                    }
                    Mode::Skip | Mode::Unwrap => {}
                    Mode::Math {
                        tag,
                        class,
                        latex,
                        text: captured,
                    } => {
                        let latex = latex.unwrap_or(captured);
                        out.push(format!("<{tag} class=\"{class}\">"));
                        out.push(format!("text:{}", latex.trim()));
                        out.push(format!("</{tag}>"));
                    }
                    Mode::Embed => out.push("</span>".to_string()),
                }
            }
            Tok::Text(t) => {
                if skipping {
                    continue;
                }
                if let Some(Frame {
                    mode: Mode::Math { text: captured, .. },
                    ..
                }) = stack
                    .iter_mut()
                    .rev()
                    .find(|f| matches!(f.mode, Mode::Math { .. }))
                {
                    captured.push_str(&t);
                    continue;
                }
                text.push_str(&t);
            }
        }
    }
    flush(&mut text, &mut out, pre_depth > 0);
    out
}

#[test]
fn obsidian_corpus() {
    let root = corpus_dir();
    let vault = root.join("vault");
    let expected_dir = root.join("expected");
    let mut paths = Vec::new();
    notes(&vault, &mut paths);
    paths.sort();
    assert!(
        staging() || paths.len() >= 1000,
        "corpus has {} notes",
        paths.len()
    );

    let verbose = std::env::var_os("CORPUS_VERBOSE").is_some();
    let focus = std::env::var("CORPUS_NOTE").ok();
    let dump = std::env::var("CORPUS_DUMP").ok().map(PathBuf::from);
    let mut passed = 0;
    let mut failures: Vec<(String, String)> = Vec::new();
    let mut by_folder: HashMap<String, (usize, usize)> = HashMap::new();

    for path in &paths {
        let rel = path.strip_prefix(&vault).unwrap();
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        let expected_path = expected_dir.join(rel).with_extension("html");
        let folder = rel_str.split('/').next().unwrap_or("").to_string();
        let entry = by_folder.entry(folder).or_insert((0, 0));
        entry.1 += 1;
        let Ok(expected_html) = fs::read_to_string(&expected_path) else {
            failures.push((rel_str, "missing expected HTML".to_string()));
            continue;
        };
        let src = fs::read_to_string(path).unwrap();
        let html = render_html(&parse(&src));
        let want = canonicalize(&expected_html);
        let got = canonicalize(&html);
        if focus.as_deref().is_some_and(|f| rel_str.contains(f)) {
            eprintln!(
                "==== {rel_str}: expected\n{}\n==== got\n{}",
                want.join("\n"),
                got.join("\n")
            );
        }
        if want == got {
            passed += 1;
            entry.0 += 1;
        } else {
            if let Some(dir) = &dump {
                let base = dir.join(&rel_str);
                fs::create_dir_all(base.parent().unwrap()).unwrap();
                fs::write(base.with_extension("want"), want.join("\n")).unwrap();
                fs::write(base.with_extension("got"), got.join("\n")).unwrap();
            }
            let first = want
                .iter()
                .zip(got.iter())
                .position(|(a, b)| a != b)
                .unwrap_or(want.len().min(got.len()));
            let ctx = |v: &[String]| -> String {
                let lo = first.saturating_sub(2);
                let hi = (first + 3).min(v.len());
                v[lo..hi].join("\n    ")
            };
            failures.push((
                rel_str,
                format!(
                    "line {first}\n  expected:\n    {}\n  got:\n    {}",
                    ctx(&want),
                    ctx(&got)
                ),
            ));
        }
    }

    if verbose {
        for (name, detail) in &failures {
            eprintln!("--- {name}: {detail}");
        }
    } else {
        for (name, _) in failures.iter().take(40) {
            eprintln!("failed: {name}");
        }
    }
    let mut folders: Vec<_> = by_folder.into_iter().collect();
    folders.sort();
    for (folder, (ok, total)) in folders {
        eprintln!("{folder}: {ok}/{total}");
    }
    eprintln!("corpus: {passed}/{} passing", paths.len());
    assert!(
        staging() || passed >= MIN_PASSING,
        "{passed} passing, below the ratchet of {MIN_PASSING}"
    );
}
