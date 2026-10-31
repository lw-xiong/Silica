//! Matching a compiled query against one file.
//!
//! Every text term becomes a regex (`(?i)` unless the term is under
//! `match-case:`) so case-insensitive matching is Unicode-correct and the
//! match ranges come straight from the regex engine. Scoped operators
//! (`line:`, `section:`, `block:`, `task:`) evaluate their sub-query against
//! each candidate range of the content and succeed if any range matches.

use crate::query::{Compare, Field, Query};
use regex::{Regex, RegexBuilder};
use std::collections::HashMap;
use std::ops::Range;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchError {
    Regex(String),
}

impl std::fmt::Display for SearchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SearchError::Regex(e) => write!(f, "invalid regex: {e}"),
        }
    }
}

impl std::error::Error for SearchError {}

/// A task list item line inside a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskLine {
    pub range: Range<usize>,
    pub done: bool,
}

/// One file as the matcher sees it. Ranges are byte ranges into `content`.
#[derive(Debug, Clone, Default)]
pub struct SearchFile<'a> {
    /// Vault-relative path.
    pub path: &'a str,
    pub content: &'a str,
    /// Tags without `#`, lowercased, inline and frontmatter merged.
    pub tags: &'a [String],
    pub frontmatter: Option<&'a serde_yaml::Value>,
    /// Top-level block ranges, for `block:`.
    pub blocks: &'a [Range<usize>],
    /// Ranges between headings (the preamble included), for `section:`.
    pub sections: &'a [Range<usize>],
    pub tasks: &'a [TaskLine],
}

impl SearchFile<'_> {
    fn name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(self.path)
    }
}

/// What a query matched in one file. Content ranges are byte ranges into
/// `SearchFile::content`; name ranges are byte ranges into its base name,
/// which Obsidian highlights in the result's title rather than in a match
/// line, so the two kinds never mix.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MatchSet {
    pub content: Vec<Range<usize>>,
    pub name: Vec<Range<usize>>,
}

impl MatchSet {
    fn extend(&mut self, other: MatchSet) {
        self.content.extend(other.content);
        self.name.extend(other.name);
    }
}

/// A query compiled for matching.
#[derive(Debug, Clone)]
pub struct Search {
    query: Query,
    node: Node,
}

#[derive(Debug, Clone)]
enum Node {
    All(Vec<Node>),
    Any(Vec<Node>),
    Not(Box<Node>),
    /// Compiled once per case mode; `None` until the mode is known.
    Text {
        insensitive: Regex,
        sensitive: Regex,
    },
    Tag(String),
    Field(Field, Box<Node>),
    Case(bool, Box<Node>),
    HasProperty(String),
    PropertyEmpty(String),
    PropertyBool(String, bool),
    PropertyCompare(String, Compare, String),
    PropertyMatch(String, Box<Node>),
}

impl Search {
    /// Parses and compiles `input`.
    pub fn new(input: &str) -> Result<Search, SearchError> {
        Search::from_query(crate::query::parse(input))
    }

    pub fn from_query(query: Query) -> Result<Search, SearchError> {
        let node = compile(&query)?;
        Ok(Search { query, node })
    }

    pub fn query(&self) -> &Query {
        &self.query
    }

    pub fn is_empty(&self) -> bool {
        self.query.is_empty()
    }

    /// Whether `file` matches; on success the ranges to highlight, sorted and
    /// merged (both empty when only the path, tags or properties matched).
    pub fn matches(&self, file: &SearchFile<'_>) -> Option<MatchSet> {
        let scope = Scope {
            text: file.content,
            base: 0,
            names_file: true,
        };
        let mut hits = MatchSet::default();
        if !eval(&self.node, file, scope, false, &mut hits) {
            return None;
        }
        Some(MatchSet {
            content: merge(hits.content),
            name: merge(hits.name),
        })
    }
}

fn merge(mut ranges: Vec<Range<usize>>) -> Vec<Range<usize>> {
    ranges.sort_by_key(|r| (r.start, r.end));
    let mut merged: Vec<Range<usize>> = Vec::new();
    for r in ranges {
        match merged.last_mut() {
            Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
            _ => merged.push(r),
        }
    }
    merged
}

fn compile(q: &Query) -> Result<Node, SearchError> {
    Ok(match q {
        Query::All(c) => Node::All(c.iter().map(compile).collect::<Result<_, _>>()?),
        Query::Any(c) => Node::Any(c.iter().map(compile).collect::<Result<_, _>>()?),
        Query::Not(c) => Node::Not(Box::new(compile(c)?)),
        Query::Text(t) | Query::Exact(t) => literal(t)?,
        Query::Regex { pattern, flags } => {
            let build = |ci: bool| {
                RegexBuilder::new(pattern)
                    .case_insensitive(ci || flags.contains('i'))
                    .multi_line(flags.contains('m'))
                    .dot_matches_new_line(flags.contains('s'))
                    .build()
                    .map_err(|e| SearchError::Regex(e.to_string()))
            };
            Node::Text {
                insensitive: build(true)?,
                sensitive: build(false)?,
            }
        }
        Query::CaseSensitive(c) => Node::Case(true, Box::new(compile(c)?)),
        Query::CaseInsensitive(c) => Node::Case(false, Box::new(compile(c)?)),
        Query::Field(Field::Tag, c) => match c.as_ref() {
            Query::Text(t) | Query::Exact(t) => Node::Tag(
                t.trim_start_matches('#')
                    .trim_end_matches('/')
                    .to_lowercase(),
            ),
            other => Node::Field(Field::Tag, Box::new(compile(other)?)),
        },
        Query::Field(f, c) => Node::Field(*f, Box::new(compile(c)?)),
        Query::HasProperty(n) => Node::HasProperty(n.to_lowercase()),
        Query::PropertyEmpty(n) => Node::PropertyEmpty(n.to_lowercase()),
        Query::PropertyBool(n, b) => Node::PropertyBool(n.to_lowercase(), *b),
        Query::PropertyCompare(n, op, v) => Node::PropertyCompare(n.to_lowercase(), *op, v.clone()),
        Query::PropertyMatch(n, c) => Node::PropertyMatch(n.to_lowercase(), Box::new(compile(c)?)),
    })
}

fn literal(t: &str) -> Result<Node, SearchError> {
    let escaped = regex::escape(t);
    let build = |ci: bool| {
        RegexBuilder::new(&escaped)
            .case_insensitive(ci)
            .build()
            .map_err(|e| SearchError::Regex(e.to_string()))
    };
    Ok(Node::Text {
        insensitive: build(true)?,
        sensitive: build(false)?,
    })
}

#[derive(Clone, Copy)]
struct Scope<'a> {
    text: &'a str,
    /// Offset of `text` inside the file content, for reported ranges.
    base: usize,
    /// Whether a bare term here also matches the file name. True only for the
    /// unscoped top level: inside `content:`, `line:`, a property value or any
    /// other operator the user named a scope and the name is not it.
    names_file: bool,
}

/// Evaluates `node` in `scope`, appending the ranges it matched to `hits`.
/// Evaluates `node`, and marks where a property hit is written: the key's
/// line in the frontmatter, so a `[key]` result has a line to show and
/// counts as a result, as a tag hit does.
fn eval(
    node: &Node,
    file: &SearchFile<'_>,
    scope: Scope<'_>,
    case_sensitive: bool,
    hits: &mut MatchSet,
) -> bool {
    let found = eval_inner(node, file, scope, case_sensitive, hits);
    if found {
        if let Some(name) = property_name(node) {
            property_key_hit(name, scope.text, scope.base, &mut hits.content);
        }
    }
    found
}

/// The property a node is about, for the five property node kinds.
fn property_name(node: &Node) -> Option<&str> {
    match node {
        Node::HasProperty(n)
        | Node::PropertyEmpty(n)
        | Node::PropertyBool(n, _)
        | Node::PropertyCompare(n, _, _)
        | Node::PropertyMatch(n, _) => Some(n),
        _ => None,
    }
}

/// The key as written on its own frontmatter line (`status:`), or for a
/// dotted name its last segment indented under its parent.
fn property_key_hit(name: &str, text: &str, base: usize, out: &mut Vec<Range<usize>>) {
    let Some(rest) = text.strip_prefix("---") else {
        return;
    };
    let Some(end) = rest.find("\n---") else {
        return;
    };
    let frontmatter = &text[..3 + end];
    let whole = format!(r"(?im)^[ \t]*({})[ \t]*:", regex::escape(name));
    let hit = Regex::new(&whole)
        .ok()
        .and_then(|re| {
            re.captures(frontmatter)
                .and_then(|c| c.get(1).map(|m| m.range()))
        })
        .or_else(|| {
            let last = name.rsplit('.').next()?;
            if last == name {
                return None;
            }
            let nested = format!(r"(?im)^[ \t]+({})[ \t]*:", regex::escape(last));
            Regex::new(&nested).ok().and_then(|re| {
                re.captures(frontmatter)
                    .and_then(|c| c.get(1).map(|m| m.range()))
            })
        });
    if let Some(r) = hit {
        out.push(base + r.start..base + r.end);
    }
}

fn eval_inner(
    node: &Node,
    file: &SearchFile<'_>,
    scope: Scope<'_>,
    case_sensitive: bool,
    hits: &mut MatchSet,
) -> bool {
    match node {
        Node::All(children) => {
            // A group with no term holds no matcher, so nothing matches it.
            // Vacuous truth here is the bug behind `foo OR`, which parses as
            // `foo` OR an empty right-hand side: an empty `All` answering
            // true listed the whole vault. `Node::Any(vec![])` already
            // answers false, and this is the same rule for the other side.
            if children.is_empty() {
                return false;
            }
            let mut local = MatchSet::default();
            for c in children {
                if !eval(c, file, scope, case_sensitive, &mut local) {
                    return false;
                }
            }
            hits.extend(local);
            true
        }
        Node::Any(children) => {
            let mut any = false;
            for c in children {
                let mut local = MatchSet::default();
                if eval(c, file, scope, case_sensitive, &mut local) {
                    any = true;
                    hits.extend(local);
                }
            }
            any
        }
        Node::Not(child) => {
            let mut local = MatchSet::default();
            !eval(child, file, scope, case_sensitive, &mut local)
        }
        Node::Text {
            insensitive,
            sensitive,
        } => {
            let re = if case_sensitive {
                sensitive
            } else {
                insensitive
            };
            let mut found = find_all(re, scope.text, scope.base, &mut hits.content);
            if scope.names_file {
                // Obsidian matches a bare term against the file name as well
                // as the content, and highlights the hit in the result title.
                found |= find_all(re, file.name(), 0, &mut hits.name);
            }
            found || (re.as_str().is_empty())
        }
        Node::Case(sensitive, child) => eval(child, file, scope, *sensitive, hits),
        Node::Tag(tag) => {
            let found = file
                .tags
                .iter()
                .any(|t| t == tag || t.starts_with(&format!("{tag}/")));
            if found {
                tag_occurrences(tag, scope.text, scope.base, &mut hits.content);
            }
            found
        }
        Node::Field(field, child) => match field {
            Field::Content => eval(
                child,
                file,
                Scope {
                    names_file: false,
                    ..scope
                },
                case_sensitive,
                hits,
            ),
            Field::File => {
                // The child ran over the name, so its ranges are name ranges:
                // an explicit `file:` hit highlights the title the same way a
                // bare term's does.
                let mut local = MatchSet::default();
                let name = file.name();
                let found = eval(
                    child,
                    file,
                    Scope {
                        text: name,
                        base: 0,
                        names_file: false,
                    },
                    case_sensitive,
                    &mut local,
                );
                if found {
                    hits.name.extend(local.content);
                }
                found
            }
            Field::Path => {
                let mut local = MatchSet::default();
                let path = file.path;
                eval(
                    child,
                    file,
                    Scope {
                        text: path,
                        base: 0,
                        names_file: false,
                    },
                    case_sensitive,
                    &mut local,
                )
            }
            Field::Tag => {
                // A non-literal tag query (regex, group): match against each tag.
                file.tags.iter().any(|t| {
                    let mut local = MatchSet::default();
                    eval(
                        child,
                        file,
                        Scope {
                            text: t,
                            base: 0,
                            names_file: false,
                        },
                        case_sensitive,
                        &mut local,
                    )
                })
            }
            Field::Line => {
                let mut any = false;
                let mut at = 0;
                for line in scope.text.split_inclusive('\n') {
                    let trimmed = line.trim_end_matches(['\n', '\r']);
                    let mut local = MatchSet::default();
                    let s = Scope {
                        text: trimmed,
                        base: scope.base + at,
                        names_file: false,
                    };
                    if eval(child, file, s, case_sensitive, &mut local) {
                        any = true;
                        hits.extend(local);
                    }
                    at += line.len();
                }
                any
            }
            Field::Block | Field::Section => {
                let ranges = if *field == Field::Block {
                    file.blocks
                } else {
                    file.sections
                };
                eval_ranges(
                    child,
                    file,
                    scope,
                    ranges.iter().cloned(),
                    case_sensitive,
                    hits,
                )
            }
            Field::Task | Field::TaskTodo | Field::TaskDone => {
                let want = match field {
                    Field::TaskTodo => Some(false),
                    Field::TaskDone => Some(true),
                    _ => None,
                };
                let ranges = file
                    .tasks
                    .iter()
                    .filter(|t| want.is_none_or(|w| t.done == w))
                    .map(|t| t.range.clone());
                eval_ranges(child, file, scope, ranges, case_sensitive, hits)
            }
        },
        Node::HasProperty(name) => property(file, name).is_some(),
        Node::PropertyEmpty(name) => property(file, name).is_some_and(is_empty_value),
        Node::PropertyBool(name, want) => property(file, name).is_some_and(|v| match v {
            serde_yaml::Value::Bool(b) => b == want,
            serde_yaml::Value::String(s) => s.eq_ignore_ascii_case(&want.to_string()),
            _ => false,
        }),
        Node::PropertyCompare(name, op, rhs) => property(file, name).is_some_and(|v| {
            let lhs = value_strings(v);
            lhs.iter().any(|l| compare(l, rhs) == Some(*op))
        }),
        Node::PropertyMatch(name, child) => property(file, name).is_some_and(|v| {
            value_strings(v).iter().any(|s| {
                let mut local = MatchSet::default();
                eval(
                    child,
                    file,
                    Scope {
                        text: s,
                        base: 0,
                        names_file: false,
                    },
                    case_sensitive,
                    &mut local,
                )
            })
        }),
    }
}

/// Appends every non-empty match of `re` in `text`, shifted by `base`, to
/// `out`; true when the regex matched at all, empty match included.
/// Where a matched tag is written, which is what Obsidian highlights for a
/// `tag:` hit: `#tag` and `#tag/sub` in the text, and the bare name on the
/// frontmatter's `tags:` line or the list items under it.
fn tag_occurrences(tag: &str, text: &str, base: usize, out: &mut Vec<Range<usize>>) {
    let escaped = regex::escape(tag);
    let inline = format!(r"(?i)(?:^|[^\p{{L}}\p{{N}}_/#])(#{escaped}(?:/[\p{{L}}\p{{N}}_/-]*)?)");
    if let Ok(re) = Regex::new(&inline) {
        for c in re.captures_iter(text) {
            if let Some(m) = c.get(1) {
                out.push(base + m.start()..base + m.end());
            }
        }
    }
    let Some(rest) = text.strip_prefix("---") else {
        return;
    };
    let Some(end) = rest.find("\n---") else {
        return;
    };
    let frontmatter = &text[..3 + end];
    let bare = format!(
        r"(?i)(?:^|[^\p{{L}}\p{{N}}_/#-])(#?{escaped}(?:/[\p{{L}}\p{{N}}_/-]*)?)(?:[^\p{{L}}\p{{N}}_/-]|$)"
    );
    let Ok(re) = Regex::new(&bare) else {
        return;
    };
    let mut at = 0;
    for line in frontmatter.split_inclusive('\n') {
        let body = line.trim_start();
        let lead = line.len() - body.len();
        if body.starts_with("tags:") || body.starts_with("tag:") || body.starts_with("- ") {
            let mut from = 0;
            while let Some(c) = re.captures_at(body, from) {
                let Some(m) = c.get(1) else { break };
                out.push(base + at + lead + m.start()..base + at + lead + m.end());
                from = m.end();
            }
        }
        at += line.len();
    }
}

fn find_all(re: &Regex, text: &str, base: usize, out: &mut Vec<Range<usize>>) -> bool {
    let mut found = false;
    for m in re.find_iter(text) {
        found = true;
        if m.end() > m.start() {
            out.push(base + m.start()..base + m.end());
        } else {
            break;
        }
    }
    found
}

fn eval_ranges(
    child: &Node,
    file: &SearchFile<'_>,
    scope: Scope<'_>,
    ranges: impl Iterator<Item = Range<usize>>,
    case_sensitive: bool,
    hits: &mut MatchSet,
) -> bool {
    // Ranges are in content coordinates; only those inside the current scope apply.
    let scope_end = scope.base + scope.text.len();
    let mut any = false;
    for r in ranges {
        if r.start < scope.base || r.end > scope_end || r.start > r.end {
            continue;
        }
        let Some(text) = scope.text.get(r.start - scope.base..r.end - scope.base) else {
            continue;
        };
        let mut local = MatchSet::default();
        let s = Scope {
            text,
            base: r.start,
            names_file: false,
        };
        if eval(child, file, s, case_sensitive, &mut local) {
            any = true;
            hits.extend(local);
        }
    }
    any
}

/// Looks a property up case-insensitively.
fn property<'a>(file: &SearchFile<'a>, name: &str) -> Option<&'a serde_yaml::Value> {
    let serde_yaml::Value::Mapping(map) = file.frontmatter? else {
        return None;
    };
    map.iter().find_map(|(k, v)| match k {
        serde_yaml::Value::String(k) if k.eq_ignore_ascii_case(name) => Some(v),
        _ => None,
    })
}

fn is_empty_value(v: &serde_yaml::Value) -> bool {
    match v {
        serde_yaml::Value::Null => true,
        serde_yaml::Value::String(s) => s.trim().is_empty(),
        serde_yaml::Value::Sequence(s) => s.is_empty(),
        serde_yaml::Value::Mapping(m) => m.is_empty(),
        _ => false,
    }
}

/// A property's comparable text forms: a scalar as one string, a list as
/// one string per item.
fn value_strings(v: &serde_yaml::Value) -> Vec<String> {
    match v {
        serde_yaml::Value::Null => vec![String::new()],
        serde_yaml::Value::Bool(b) => vec![b.to_string()],
        serde_yaml::Value::Number(n) => vec![n.to_string()],
        serde_yaml::Value::String(s) => vec![s.clone()],
        serde_yaml::Value::Sequence(items) => items.iter().flat_map(value_strings).collect(),
        other => vec![serde_yaml::to_string(other).unwrap_or_default()],
    }
}

/// Numbers compare numerically, ISO dates and everything else as strings.
fn compare(lhs: &str, rhs: &str) -> Option<Compare> {
    let ord = match (lhs.trim().parse::<f64>(), rhs.trim().parse::<f64>()) {
        (Ok(a), Ok(b)) => a.partial_cmp(&b)?,
        _ => lhs.trim().cmp(rhs.trim()),
    };
    match ord {
        std::cmp::Ordering::Greater => Some(Compare::Greater),
        std::cmp::Ordering::Less => Some(Compare::Less),
        std::cmp::Ordering::Equal => None,
    }
}

/// Convenience for callers that keep per-file inputs in a map.
pub type Tags = HashMap<String, Vec<String>>;

#[cfg(test)]
mod tests {
    use super::*;

    const NOTE: &str = "---\nstatus: Done\nrating: 7\nwhen: 2026-10-16\nlist: [a, b]\nempty:\n---\n# Heading One\nThe Quick brown fox.\n- [ ] buy milk\n- [x] call Bob\n\n## Heading Two\nquick again, fox apart\nbrown\n";

    fn file<'a>(
        tags: &'a [String],
        fm: &'a serde_yaml::Value,
        blocks: &'a [Range<usize>],
        sections: &'a [Range<usize>],
        tasks: &'a [TaskLine],
    ) -> SearchFile<'a> {
        SearchFile {
            path: "Projects/Fox Note.md",
            content: NOTE,
            tags,
            frontmatter: Some(fm),
            blocks,
            sections,
            tasks,
        }
    }

    struct Fixture {
        tags: Vec<String>,
        fm: serde_yaml::Value,
        blocks: Vec<Range<usize>>,
        sections: Vec<Range<usize>>,
        tasks: Vec<TaskLine>,
    }

    fn fixture() -> Fixture {
        let fm_end = NOTE.find("---\n#").unwrap() + 4;
        let fm: serde_yaml::Value = serde_yaml::from_str(&NOTE[4..fm_end - 4]).unwrap();
        let h2 = NOTE.find("## Heading Two").unwrap();
        let line = |s: &str| {
            let start = NOTE.find(s).unwrap();
            start..start + s.len()
        };
        Fixture {
            tags: vec!["work".into(), "work/weekly".into()],
            fm,
            blocks: vec![
                line("# Heading One"),
                line("The Quick brown fox."),
                line("- [ ] buy milk\n- [x] call Bob"),
                line("## Heading Two"),
                line("quick again, fox apart\nbrown"),
            ],
            sections: vec![0..fm_end, fm_end..h2, h2..NOTE.len()],
            tasks: vec![
                TaskLine {
                    range: line("- [ ] buy milk"),
                    done: false,
                },
                TaskLine {
                    range: line("- [x] call Bob"),
                    done: true,
                },
            ],
        }
    }

    fn hits(q: &str) -> Option<MatchSet> {
        let f = fixture();
        let file = file(&f.tags, &f.fm, &f.blocks, &f.sections, &f.tasks);
        Search::new(q).unwrap().matches(&file)
    }

    fn run(q: &str) -> Option<Vec<String>> {
        hits(q).map(|m| {
            m.content
                .iter()
                .map(|r| NOTE[r.clone()].to_string())
                .collect()
        })
    }

    /// The matched substrings of the file name, "Fox Note.md".
    fn names(q: &str) -> Option<Vec<String>> {
        let name = "Fox Note.md";
        hits(q).map(|m| m.name.iter().map(|r| name[r.clone()].to_string()).collect())
    }

    #[test]
    fn words_match_case_insensitively_with_ranges() {
        assert_eq!(run("quick"), Some(vec!["Quick".into(), "quick".into()]));
        assert_eq!(run("quick zebra"), None);
        assert_eq!(run("zebra OR fox"), Some(vec!["fox".into(), "fox".into()]));
        assert_eq!(run("-zebra"), Some(vec![]));
        assert_eq!(run("-fox"), None);
        assert_eq!(run("match-case:quick"), Some(vec!["quick".into()]));
        assert_eq!(run("\"brown fox\""), Some(vec!["brown fox".into()]));
        assert_eq!(run("\"fox brown\""), None);
        assert_eq!(run("/f.x/"), Some(vec!["fox".into(), "fox".into()]));
        assert_eq!(run("/F.X/"), Some(vec!["fox".into(), "fox".into()]));
        assert!(Search::new("/(/").is_err());
    }

    /// A `tag:` hit highlights where the tag is written: inline `#tag` and
    /// `#tag/sub`, and the name on the frontmatter's tags line, but not the
    /// word in prose, as Obsidian's result rows show.
    #[test]
    fn a_tag_hit_highlights_the_tags_occurrences() {
        let text = "---\ntags: [work, other]\n---\nSee #work and #work/weekly; work itself and #network stay.\n";
        let tags = vec!["work".to_string(), "work/weekly".into(), "other".into()];
        let fm: serde_yaml::Value = serde_yaml::from_str("tags: [work, other]").unwrap();
        let file = SearchFile {
            path: "A.md",
            content: text,
            tags: &tags,
            frontmatter: Some(&fm),
            blocks: &[],
            sections: &[],
            tasks: &[],
        };
        let hits = Search::new("tag:#work").unwrap().matches(&file).unwrap();
        let shown: Vec<&str> = hits.content.iter().map(|r| &text[r.clone()]).collect();
        assert_eq!(shown, vec!["work", "#work", "#work/weekly"]);
        assert!(Search::new("tag:#net").unwrap().matches(&file).is_none());
    }

    /// A property hit shows its key's line: the key is the highlight, so
    /// the row counts as a result; the name is matched whatever its case,
    /// and the quotes the All properties pane writes are not part of it.
    #[test]
    fn a_property_hit_highlights_the_key_in_the_frontmatter() {
        let f = fixture();
        let file = file(&f.tags, &f.fm, &f.blocks, &f.sections, &f.tasks);
        let hits = Search::new("[\"Status\"]").unwrap().matches(&file).unwrap();
        let shown: Vec<&str> = hits.content.iter().map(|r| &NOTE[r.clone()]).collect();
        assert_eq!(shown, vec!["status"]);
        let hits = Search::new("[rating:7]").unwrap().matches(&file).unwrap();
        let shown: Vec<&str> = hits.content.iter().map(|r| &NOTE[r.clone()]).collect();
        assert_eq!(shown, vec!["rating"]);
        assert!(Search::new("[missing]").unwrap().matches(&file).is_none());
    }

    #[test]
    fn file_path_and_tag_fields() {
        assert_eq!(run("file:fox"), Some(vec![]));
        assert_eq!(run("file:projects"), None);
        assert_eq!(run("path:projects"), Some(vec![]));
        assert_eq!(run("tag:#work"), Some(vec![]));
        assert_eq!(run("tag:work/weekly"), Some(vec![]));
        assert_eq!(
            run("tag:wor"),
            None,
            "tags match exactly, not as substrings"
        );
        assert_eq!(run("tag:#weekly"), None);
        assert_eq!(run("tag:/wee/"), Some(vec![]));
    }

    #[test]
    fn scoped_operators() {
        assert_eq!(
            run("line:(quick fox)"),
            Some(vec![
                "Quick".into(),
                "fox".into(),
                "quick".into(),
                "fox".into()
            ])
        );
        assert_eq!(
            run("line:(quick brown)"),
            Some(vec!["Quick".into(), "brown".into()])
        );
        assert_eq!(
            run("line:(fox brown)"),
            Some(vec!["brown".into(), "fox".into()])
        );
        assert_eq!(
            run("block:(fox brown)"),
            Some(vec![
                "brown".into(),
                "fox".into(),
                "fox".into(),
                "brown".into()
            ])
        );
        assert_eq!(
            run("block:(again brown)"),
            Some(vec!["again".into(), "brown".into()])
        );
        assert_eq!(run("section:(milk again)"), None);
        assert_eq!(
            run("section:(milk fox)"),
            Some(vec!["fox".into(), "milk".into()])
        );
        assert_eq!(run("task:milk"), Some(vec!["milk".into()]));
        assert_eq!(run("task-done:milk"), None);
        assert_eq!(run("task-done:bob"), Some(vec!["Bob".into()]));
        assert_eq!(run("task-todo:bob"), None);
        assert_eq!(run("-line:(quick fox)"), None);
    }

    #[test]
    fn property_queries() {
        // A property hit highlights its key's line in the frontmatter.
        assert_eq!(run("[status]"), Some(vec!["status".into()]));
        assert_eq!(run("[Status]"), Some(vec!["status".into()]));
        assert_eq!(run("[missing]"), None);
        assert_eq!(run("[status:done]"), Some(vec!["status".into()]));
        assert_eq!(run("[status:open OR done]"), Some(vec!["status".into()]));
        assert_eq!(
            run("[status:don]"),
            Some(vec!["status".into()]),
            "substring like text terms"
        );
        assert_eq!(run("[list:b]"), Some(vec!["list".into()]));
        assert_eq!(run("[empty:null]"), Some(vec!["empty".into()]));
        assert_eq!(run("[status:null]"), None);
        assert_eq!(run("[rating:>5]"), Some(vec!["rating".into()]));
        assert_eq!(run("[rating:<5]"), None);
        assert_eq!(run("[when:>2026-01-01]"), Some(vec!["when".into()]));
        assert_eq!(run("[when:<2026-01-01]"), None);
        assert_eq!(run("[rating:true]"), None);
    }

    #[test]
    fn bare_terms_match_the_file_name_too() {
        // "note" is in the name only, "fox" in both, "quick" in the content only.
        assert_eq!(run("note"), Some(vec![]));
        assert_eq!(names("note"), Some(vec!["Note".into()]));
        assert_eq!(names("fox"), Some(vec!["Fox".into()]));
        assert_eq!(names("quick"), Some(vec![]));
        assert_eq!(names("\"fox note\""), Some(vec!["Fox Note".into()]));
        assert_eq!(names("/n.te/"), Some(vec!["Note".into()]));
        assert_eq!(names("note.md"), Some(vec!["Note.md".into()]));
        // A name hit satisfies its half of an AND and of an OR.
        assert_eq!(
            run("note quick"),
            Some(vec!["Quick".into(), "quick".into()])
        );
        assert_eq!(names("note quick"), Some(vec!["Note".into()]));
        assert_eq!(run("note OR zebra"), Some(vec![]));
        assert_eq!(run("-note"), None, "a name hit is a hit to exclude");
    }

    #[test]
    fn a_named_scope_excludes_the_file_name() {
        assert_eq!(run("content:note"), None);
        assert_eq!(run("line:(note)"), None);
        assert_eq!(run("block:(note)"), None);
        assert_eq!(run("section:(note)"), None);
        assert_eq!(run("task:note"), None);
        assert_eq!(run("[status:note]"), None);
        assert_eq!(run("path:note"), Some(vec![]), "the path still matches");
        // An explicit file: hit highlights the title, like a bare term's.
        assert_eq!(names("file:note"), Some(vec!["Note".into()]));
        assert_eq!(names("path:projects"), Some(vec![]));
    }

    /// A query with no term holds no matcher, so it matches no file. The
    /// pane never asks — `Query::is_empty` gates it, the way Obsidian's
    /// `startSearch` returns before adding a result — but the answer must
    /// still be "nothing": an empty `All` answering true by vacuous truth is
    /// what made `foo OR` list the whole vault.
    #[test]
    fn an_empty_query_holds_no_matcher() {
        assert_eq!(run(""), None);
        assert_eq!(names(""), None);
        assert_eq!(run("()"), None);
    }

    /// The ten half-typed shapes of `docs/silica/Specification.md`, "A
    /// half-typed query: tolerated, or nothing at all", over the vault that
    /// section was observed on: two notes holding `foo`, two without it.
    /// Obsidian is not uniform — a dangling `OR` or `-`, a stray `)`, a
    /// leading `(` and an unclosed quote all match as if they were not
    /// there, while a dangling `AND` and an unclosed trailing `(` match
    /// nothing. Each row asserts the files returned rather than the parse
    /// tree: two of these shapes parse cleanly and went wrong only here.
    #[test]
    fn half_typed_queries_match_as_obsidian_does() {
        // `and` sits in a note that also holds `foo`, so a query that read
        // the dangling `AND` as an ordinary word would match it.
        let vault = [
            ("apple.md", "apple and foo at the root\n"),
            ("notes/apple.md", "apple in a folder, with foo in it\n"),
            ("kiwi.md", "kiwi, and nothing else\n"),
            ("mango.md", "mango, a plain note\n"),
        ];
        let results = |q: &str| -> Vec<&'static str> {
            let search = Search::new(q).unwrap();
            if search.is_empty() {
                // No matcher: the pane returns before listing anything.
                return Vec::new();
            }
            let mut out = Vec::new();
            for (path, content) in vault.iter().copied() {
                let file = SearchFile {
                    path,
                    content,
                    ..Default::default()
                };
                if search.matches(&file).is_some() {
                    out.push(path);
                }
            }
            out
        };
        let as_foo = vec!["apple.md", "notes/apple.md"];
        for q in ["foo", "foo OR", "foo -", "foo )", "(foo", "\"foo", "foo \""] {
            assert_eq!(results(q), as_foo, "{q:?} should match as foo does");
        }
        for q in ["foo AND", "foo (", "OR", "("] {
            assert_eq!(results(q), Vec::<&str>::new(), "{q:?} should match nothing");
        }
    }
}
