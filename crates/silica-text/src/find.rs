//! Search within the open note: literal or regular-expression matching over
//! the editor's buffer, case-insensitive unless asked, the way Obsidian's
//! find panel matches.

use regex::{Regex, RegexBuilder};
use std::ops::Range;

/// Matches never exceed this many, so a one-letter query on a huge note
/// stays cheap to highlight and page through.
pub const MAX_MATCHES: usize = 20_000;

/// The panel's two toggles, both off by default like Obsidian's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Options {
    pub match_case: bool,
    pub regex: bool,
}

/// Compiles `query` under `opts`; `None` for an empty query or a regex
/// that does not parse (the panel then shows no results).
pub fn compile(query: &str, opts: Options) -> Option<Regex> {
    if query.is_empty() {
        return None;
    }
    let pattern = if opts.regex {
        query.to_string()
    } else {
        regex::escape(query)
    };
    RegexBuilder::new(&pattern)
        .case_insensitive(!opts.match_case)
        .multi_line(true)
        .size_limit(1 << 22)
        .build()
        .ok()
}

/// Non-overlapping byte ranges of `query` in `text`. Empty matches (a
/// regex like `a*`) are skipped so stepping always moves.
pub fn find_matches(text: &str, query: &str, opts: Options) -> Vec<Range<usize>> {
    let Some(re) = compile(query, opts) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for m in re.find_iter(text) {
        if m.is_empty() {
            continue;
        }
        out.push(m.range());
        if out.len() >= MAX_MATCHES {
            break;
        }
    }
    out
}

/// The text that replaces the match at `m`: the replacement verbatim for a
/// literal query, and with `$&` (the match), `$1` to `$9` (groups) and
/// `$$` expanded for a regex one, as CodeMirror's panel does.
pub fn replacement_for(
    text: &str,
    m: &Range<usize>,
    re: &Regex,
    opts: Options,
    with: &str,
) -> String {
    if !opts.regex || !with.contains('$') {
        return with.to_string();
    }
    let Some(caps) = re
        .captures_at(text, m.start)
        .filter(|c| c.get(0).is_some_and(|g| g.range() == *m))
    else {
        return with.to_string();
    };
    let mut out = String::with_capacity(with.len());
    let mut chars = with.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '$' {
            out.push(c);
            continue;
        }
        match chars.peek().copied() {
            Some('$') => {
                chars.next();
                out.push('$');
            }
            Some('&') => {
                chars.next();
                out.push_str(&text[m.clone()]);
            }
            Some(d) if d.is_ascii_digit() => {
                chars.next();
                let n = d.to_digit(10).unwrap_or(0) as usize;
                if let Some(g) = caps.get(n) {
                    out.push_str(g.as_str());
                }
            }
            _ => out.push('$'),
        }
    }
    out
}

/// The match to select when searching forward from `from`: the first one
/// starting at or after it, wrapping to the first.
pub fn next_from(matches: &[Range<usize>], from: usize) -> Option<usize> {
    if matches.is_empty() {
        return None;
    }
    Some(matches.iter().position(|m| m.start >= from).unwrap_or(0))
}

/// The match to select when searching backward from `from`: the last one
/// starting before it, wrapping to the last.
pub fn previous_from(matches: &[Range<usize>], from: usize) -> Option<usize> {
    if matches.is_empty() {
        return None;
    }
    Some(
        matches
            .iter()
            .rposition(|m| m.start < from)
            .unwrap_or(matches.len() - 1),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn literal(text: &str, q: &str) -> Vec<Range<usize>> {
        find_matches(text, q, Options::default())
    }

    #[test]
    fn matches_ignore_case_and_do_not_overlap() {
        assert_eq!(literal("Aa aA xa", "aa"), vec![0..2, 3..5]);
        assert_eq!(literal("aaa", "aa"), vec![0..2]);
        assert_eq!(literal("abc", ""), Vec::<Range<usize>>::new());
        assert_eq!(literal("Ärger ärger", "ärg"), vec![0..4, 7..11]);
        assert_eq!(literal("naïve", "ÏV"), vec![2..5]);
        // ASCII query over non-ASCII text stays on char boundaries.
        assert_eq!(literal("é a", "a"), vec![3..4]);
        // Literal mode takes regex metacharacters as themselves.
        assert_eq!(literal("a.c abc", "a.c"), vec![0..3]);
    }

    #[test]
    fn match_case_and_regex_toggles() {
        let case = Options {
            match_case: true,
            regex: false,
        };
        assert_eq!(
            find_matches("Aa aA", "aa", case),
            Vec::<Range<usize>>::new()
        );
        assert_eq!(find_matches("Aa aA", "Aa", case), vec![0..2]);
        let re = Options {
            match_case: false,
            regex: true,
        };
        assert_eq!(find_matches("a1 b22 c", r"[a-z]\d+", re), vec![0..2, 3..6]);
        assert_eq!(find_matches("x\ny", "^y", re), vec![2..3]);
        // Empty matches are skipped, an invalid pattern finds nothing.
        assert_eq!(find_matches("aaa", "b*", re), Vec::<Range<usize>>::new());
        assert_eq!(find_matches("aaa", "(", re), Vec::<Range<usize>>::new());
    }

    #[test]
    fn regex_replacements_expand_groups() {
        let re = Options {
            match_case: false,
            regex: true,
        };
        let text = "John Smith";
        let regex = compile(r"(\w+) (\w+)", re).unwrap();
        let m = 0..10;
        assert_eq!(
            replacement_for(text, &m, &regex, re, "$2, $1 ($&) $$ $x"),
            "Smith, John (John Smith) $ $x"
        );
        // Literal mode leaves dollars alone.
        let lit = Options::default();
        let regex = compile("John", lit).unwrap();
        assert_eq!(replacement_for(text, &(0..4), &regex, lit, "$1"), "$1");
    }

    #[test]
    fn next_and_previous_wrap() {
        let m = vec![2..4, 10..12, 20..22];
        assert_eq!(next_from(&m, 0), Some(0));
        assert_eq!(next_from(&m, 4), Some(1));
        assert_eq!(next_from(&m, 21), Some(0));
        assert_eq!(previous_from(&m, 20), Some(1));
        assert_eq!(previous_from(&m, 2), Some(2));
        assert_eq!(next_from(&[], 0), None);
    }
}
