//! CommonMark 0.31.2 spec conformance.
//!
//! Every example from `tests/data/commonmark-0.31.2.json` is parsed with the
//! extensions off and rendered without Obsidian attributes; the HTML must
//! match the spec byte for byte. The pass count is a ratchet: raise
//! `MIN_PASSING` when more examples pass, and never lower it.
//!
//! Set `COMMONMARK_VERBOSE=1` to print every failing example.

use silica_markdown::{parse_with, render_html_with, HtmlOptions, Options};

const SPEC: &str = include_str!("data/commonmark-0.31.2.json");
const MIN_PASSING: usize = 652;

struct Example {
    markdown: String,
    html: String,
    example: usize,
    section: String,
}

/// Minimal reader for the spec's flat JSON array of string-valued objects.
fn load() -> Vec<Example> {
    let mut out = Vec::new();
    let mut rest = SPEC;
    while let Some(start) = rest.find('{') {
        let end = find_object_end(&rest[start..]);
        let obj = &rest[start..start + end];
        let get = |key: &str| -> String {
            let k = format!("\"{key}\":");
            let i = obj.find(&k).expect(key) + k.len();
            let s = obj[i..].trim_start();
            if s.starts_with('"') {
                unescape_json(&s[1..json_string_end(s)])
            } else {
                s.split(|c: char| !c.is_ascii_digit())
                    .next()
                    .unwrap()
                    .to_string()
            }
        };
        out.push(Example {
            markdown: get("markdown"),
            html: get("html"),
            example: get("example").parse().unwrap(),
            section: get("section"),
        });
        rest = &rest[start + end..];
    }
    out
}

/// Index of the closing quote of the JSON string starting at `s[0] == '"'`.
fn json_string_end(s: &str) -> usize {
    let b = s.as_bytes();
    let mut i = 1;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'"' => return i,
            _ => i += 1,
        }
    }
    b.len()
}

/// Byte length of the JSON object starting at `s[0] == '{'`, honoring strings.
fn find_object_end(s: &str) -> usize {
    let b = s.as_bytes();
    let mut i = 1;
    let mut in_str = false;
    while i < b.len() {
        match b[i] {
            b'\\' if in_str => i += 1,
            b'"' => in_str = !in_str,
            b'}' if !in_str => return i + 1,
            _ => {}
        }
        i += 1;
    }
    b.len()
}

fn unescape_json(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('b') => out.push('\u{8}'),
            Some('f') => out.push('\u{c}'),
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some('/') => out.push('/'),
            Some('u') => {
                let hex: String = chars.by_ref().take(4).collect();
                let code = u32::from_str_radix(&hex, 16).unwrap();
                if (0xd800..0xdc00).contains(&code) {
                    // Surrogate pair.
                    let _ = chars.next();
                    let _ = chars.next();
                    let hex2: String = chars.by_ref().take(4).collect();
                    let low = u32::from_str_radix(&hex2, 16).unwrap();
                    let full = 0x10000 + ((code - 0xd800) << 10) + (low - 0xdc00);
                    out.push(char::from_u32(full).unwrap());
                } else {
                    out.push(char::from_u32(code).unwrap());
                }
            }
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

#[test]
fn commonmark_spec() {
    let examples = load();
    assert_eq!(examples.len(), 652, "spec example count");
    let opts = Options::commonmark();
    let html_opts = HtmlOptions::commonmark();
    let verbose = std::env::var_os("COMMONMARK_VERBOSE").is_some();
    let mut passed = 0;
    let mut failed_by_section: Vec<(String, Vec<usize>)> = Vec::new();
    for ex in &examples {
        let doc = parse_with(&ex.markdown, &opts);
        let html = render_html_with(&doc, &html_opts);
        if html == ex.html {
            passed += 1;
        } else {
            if verbose {
                eprintln!(
                    "--- example {} ({})\n{}\n--- expected\n{}\n--- got\n{}",
                    ex.example, ex.section, ex.markdown, ex.html, html
                );
            }
            match failed_by_section.iter_mut().find(|(s, _)| *s == ex.section) {
                Some((_, v)) => v.push(ex.example),
                None => failed_by_section.push((ex.section.clone(), vec![ex.example])),
            }
        }
    }
    for (section, ids) in &failed_by_section {
        eprintln!("{section}: {} failing {ids:?}", ids.len());
    }
    eprintln!("commonmark: {passed}/{} passing", examples.len());
    assert!(
        passed >= MIN_PASSING,
        "{passed} passing, below the ratchet of {MIN_PASSING}"
    );
}
