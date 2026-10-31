//! The query grammar.
//!
//! ```text
//! query   := or
//! or      := and ( "OR" and )*
//! and     := unary+
//! unary   := "-" unary | primary
//! primary := "(" or ")" | field ":" unary | "\"" phrase "\"" | "/" regex "/" flags
//!          | "[" property "]" | word
//! ```
//!
//! `OR` must be uppercase and binds looser than the implicit AND between
//! adjacent terms. A field prefix applies to the one term, group, phrase or
//! regex after it. Unknown prefixes stay part of the word.
//!
//! Half-typed input is read as far as it goes, and Obsidian is not uniform
//! about it (`docs/silica/Specification.md`, "A half-typed query: tolerated,
//! or nothing at all"): a dangling `OR` or `-`, a stray `)`, a leading `(`
//! and an unclosed quote all match as if they were not there, while a
//! dangling `AND` and an unclosed trailing `(` make the whole query match
//! nothing. Both of those parse to a group with no term — `Query::All(vec![])`
//! — which never matches, and is kept rather than flattened away.

use std::fmt::Write;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    /// `file:` — the file name.
    File,
    /// `path:` — the full vault path.
    Path,
    /// `content:` — the note text; the default scope.
    Content,
    /// `tag:` — exact tag or a parent of a nested tag.
    Tag,
    /// `line:(…)` — every term on one line.
    Line,
    /// `block:(…)` — every term in one markdown block.
    Block,
    /// `section:(…)` — every term under one heading.
    Section,
    /// `task:`, `task-todo:`, `task-done:`.
    Task,
    TaskTodo,
    TaskDone,
}

impl Field {
    fn from_prefix(s: &str) -> Option<Field> {
        Some(match s {
            "file" => Field::File,
            "path" => Field::Path,
            "content" => Field::Content,
            "tag" => Field::Tag,
            "line" => Field::Line,
            "block" => Field::Block,
            "section" => Field::Section,
            "task" => Field::Task,
            "task-todo" => Field::TaskTodo,
            "task-done" => Field::TaskDone,
            _ => return None,
        })
    }

    /// The label Obsidian shows for this operator.
    pub fn label(self) -> &'static str {
        match self {
            Field::File => "Match file name",
            Field::Path => "Match file path",
            Field::Content => "Match file content",
            Field::Tag => "Match tag",
            Field::Line => "Match line",
            Field::Block => "Match block",
            Field::Section => "Match section",
            Field::Task => "Match task",
            Field::TaskTodo => "Match task (todo)",
            Field::TaskDone => "Match task (done)",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compare {
    Greater,
    Less,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Query {
    /// Every child matches ("Match all of").
    All(Vec<Query>),
    /// Any child matches ("Match any of").
    Any(Vec<Query>),
    /// The child does not match ("Excluding").
    Not(Box<Query>),
    /// A bare word ("Matches text").
    Text(String),
    /// A quoted phrase ("Contains exact text").
    Exact(String),
    /// `/pattern/flags` ("Matches regex").
    Regex { pattern: String, flags: String },
    /// `match-case:` ("Case sensitive").
    CaseSensitive(Box<Query>),
    /// `ignore-case:` ("Case insensitive").
    CaseInsensitive(Box<Query>),
    /// `field:term`.
    Field(Field, Box<Query>),
    /// `[name]` ("Contains property").
    HasProperty(String),
    /// `[name:null]` ("Is empty").
    PropertyEmpty(String),
    /// `[name:true]` / `[name:false]` ("Is true" / "Is false").
    PropertyBool(String, bool),
    /// `[name:>5]` / `[name:<5]` ("Greater than" / "Less than").
    PropertyCompare(String, Compare, String),
    /// `[name:value]` — the value is itself a query over the property's text.
    PropertyMatch(String, Box<Query>),
}

impl Query {
    /// The tree in the format of Obsidian's "Explain search terms" panel.
    pub fn explain(&self) -> String {
        let mut out = String::new();
        self.explain_into(&mut out, 0);
        out
    }

    fn explain_into(&self, out: &mut String, depth: usize) {
        let indent = "  ".repeat(depth);
        let line = |out: &mut String, s: &str| {
            let _ = writeln!(out, "{indent}{s}");
        };
        match self {
            Query::All(children) => {
                line(out, "Match all of");
                for c in children {
                    c.explain_into(out, depth + 1);
                }
            }
            Query::Any(children) => {
                line(out, "Match any of");
                for c in children {
                    c.explain_into(out, depth + 1);
                }
            }
            Query::Not(child) => {
                line(out, "Excluding");
                child.explain_into(out, depth + 1);
            }
            Query::Text(t) => line(out, &format!("Matches text: \"{t}\"")),
            Query::Exact(t) => line(out, &format!("Contains exact text: {t}")),
            Query::Regex { pattern, flags } => {
                line(out, &format!("Matches regex: /{pattern}/{flags}"))
            }
            Query::CaseSensitive(child) => {
                line(out, "Case sensitive");
                child.explain_into(out, depth + 1);
            }
            Query::CaseInsensitive(child) => {
                line(out, "Case insensitive");
                child.explain_into(out, depth + 1);
            }
            Query::Field(field, child) => {
                line(out, field.label());
                child.explain_into(out, depth + 1);
            }
            Query::HasProperty(name) => line(out, &format!("Contains property: {name}")),
            Query::PropertyEmpty(name) => line(out, &format!("Is empty: {name}")),
            Query::PropertyBool(name, true) => line(out, &format!("Is true: {name}")),
            Query::PropertyBool(name, false) => line(out, &format!("Is false: {name}")),
            Query::PropertyCompare(name, Compare::Greater, v) => {
                line(out, &format!("Greater than: {name} > {v}"))
            }
            Query::PropertyCompare(name, Compare::Less, v) => {
                line(out, &format!("Less than: {name} < {v}"))
            }
            Query::PropertyMatch(name, child) => {
                line(out, &format!("Contains property: {name}"));
                child.explain_into(out, depth + 1);
            }
        }
    }

    /// True for a query that holds no term able to match anything: empty
    /// input, but also the half-typed `(`, `"`, `OR` or `file:` that the
    /// parser reads as far as it goes. Obsidian's `startSearch` empties the
    /// result list and returns when its compile yields no matcher, so a
    /// caller showing results should do nothing for such a query rather than
    /// list the whole vault.
    ///
    /// `foo (` and `foo AND` are *not* empty: each holds a term beside a
    /// group with no term, so the search runs and finds nothing — Obsidian's
    /// "No matches found.", a real 0 rather than an untouched pane.
    pub fn is_empty(&self) -> bool {
        match self {
            Query::All(c) | Query::Any(c) => c.iter().all(Query::is_empty),
            Query::Not(c)
            | Query::CaseSensitive(c)
            | Query::CaseInsensitive(c)
            | Query::Field(_, c)
            | Query::PropertyMatch(_, c) => c.is_empty(),
            Query::Text(t) | Query::Exact(t) => t.is_empty(),
            Query::Regex { pattern, .. } => pattern.is_empty(),
            Query::HasProperty(n) | Query::PropertyEmpty(n) | Query::PropertyBool(n, _) => {
                n.is_empty()
            }
            Query::PropertyCompare(n, _, v) => n.is_empty() || v.is_empty(),
        }
    }
}

/// Parses a query string. Never fails: unbalanced input is read as far as it
/// goes, as Obsidian does while the user is still typing.
pub fn parse(input: &str) -> Query {
    let mut p = Parser {
        chars: input.chars().collect(),
        pos: 0,
    };
    let q = p.or(None);
    p.skip_ws();
    if p.pos < p.chars.len() {
        // Stray closing parens or brackets: skip and keep parsing.
        p.pos += 1;
        let rest = p.or(None);
        return simplify(Query::All(vec![q, rest]));
    }
    simplify(q)
}

/// Flattens single-child groups and nested `All`/`Any` of the same kind.
/// A group with no term is the one thing not flattened away: it is not
/// nothing, it is a group nothing can match, and `foo (` owes its "No
/// matches found." to it.
fn simplify(q: Query) -> Query {
    match q {
        Query::All(children) => {
            let mut out = Vec::new();
            for c in children.into_iter().map(simplify) {
                match c {
                    Query::All(inner) if !inner.is_empty() => out.extend(inner),
                    other => out.push(other),
                }
            }
            if out.len() == 1 {
                out.pop().unwrap()
            } else {
                Query::All(out)
            }
        }
        Query::Any(children) => {
            let mut out = Vec::new();
            for c in children.into_iter().map(simplify) {
                match c {
                    Query::Any(inner) => out.extend(inner),
                    other => out.push(other),
                }
            }
            if out.len() == 1 {
                out.pop().unwrap()
            } else {
                Query::Any(out)
            }
        }
        Query::Not(c) => Query::Not(Box::new(simplify(*c))),
        Query::CaseSensitive(c) => Query::CaseSensitive(Box::new(simplify(*c))),
        Query::CaseInsensitive(c) => Query::CaseInsensitive(Box::new(simplify(*c))),
        Query::Field(f, c) => Query::Field(f, Box::new(simplify(*c))),
        Query::PropertyMatch(n, c) => Query::PropertyMatch(n, Box::new(simplify(*c))),
        other => other,
    }
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<char> {
        self.chars.get(self.pos + offset).copied()
    }

    fn skip_ws(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.pos += 1;
        }
    }

    fn at_terminator(&self, close: Option<char>) -> bool {
        match (self.peek(), close) {
            (None, _) => true,
            (Some(c), Some(close)) if c == close => true,
            _ => false,
        }
    }

    /// An uppercase keyword at the cursor, ended the way `OR` must be: by
    /// whitespace, a paren or the end of the input.
    fn at_keyword(&self, word: &str) -> bool {
        for (i, c) in word.chars().enumerate() {
            if self.peek_at(i) != Some(c) {
                return false;
            }
        }
        self.peek_at(word.chars().count())
            .is_none_or(|c| c.is_whitespace() || c == '(' || c == ')')
    }

    fn at_or(&self) -> bool {
        self.at_keyword("OR")
    }

    /// `AND` with no term after it. Obsidian shows "No matches found." for
    /// `foo AND` where it ignores a dangling `OR`, so a trailing `AND` is an
    /// operator whose right-hand side is still missing and the query matches
    /// nothing. Whether an `AND` *between* two terms is an operator at all is
    /// not observed — only `OR` is documented as one, and the AND between
    /// adjacent terms is implicit — so such an `AND` stays an ordinary word
    /// until a probe settles it.
    fn at_dangling_and(&self, close: Option<char>) -> bool {
        self.at_keyword("AND") && self.nothing_follows(3, close)
    }

    /// Whether the input `offset` chars ahead holds nothing but whitespace
    /// before the end or the enclosing group's `close`: the test for an
    /// operator typed with its right-hand side still missing.
    fn nothing_follows(&self, offset: usize, close: Option<char>) -> bool {
        let mut i = offset;
        while self.peek_at(i).is_some_and(char::is_whitespace) {
            i += 1;
        }
        match (self.peek_at(i), close) {
            (None, _) => true,
            (Some(c), Some(close)) => c == close,
            _ => false,
        }
    }

    fn or(&mut self, close: Option<char>) -> Query {
        let mut branches = vec![self.and(close)];
        loop {
            self.skip_ws();
            if self.at_or() {
                self.pos += 2;
                branches.push(self.and(close));
            } else {
                break;
            }
        }
        if branches.len() == 1 {
            branches.pop().unwrap()
        } else {
            Query::Any(branches)
        }
    }

    fn and(&mut self, close: Option<char>) -> Query {
        let mut terms = Vec::new();
        loop {
            self.skip_ws();
            if self.at_terminator(close) || self.at_or() {
                break;
            }
            if close.is_none() && self.peek() == Some(')') {
                self.pos += 1;
                continue;
            }
            if self.at_dangling_and(close) {
                // An operator with no operand: a group with no term, which
                // matches nothing, rather than a search for the word "AND".
                self.pos += 3;
                terms.push(Query::All(Vec::new()));
                continue;
            }
            match self.unary(close) {
                Some(t) => terms.push(t),
                None => break,
            }
        }
        Query::All(terms)
    }

    fn unary(&mut self, close: Option<char>) -> Option<Query> {
        if self.peek() == Some('-') {
            if self.nothing_follows(1, close) {
                // A dangling `-`: Obsidian matches `foo -` exactly as `foo`,
                // so the operator with nothing to negate yields no term at
                // all rather than a search for a literal hyphen. Consuming it
                // (and the whitespace after it) leaves the cursor on the
                // terminator, so `parse` does not take it for stray input.
                self.pos += 1;
                self.skip_ws();
                return None;
            }
            self.pos += 1;
            return match self.unary(close) {
                Some(q) => Some(Query::Not(Box::new(q))),
                None => Some(Query::Text("-".into())),
            };
        }
        self.primary(close)
    }

    fn primary(&mut self, close: Option<char>) -> Option<Query> {
        match self.peek()? {
            '(' => {
                self.pos += 1;
                let q = self.or(Some(')'));
                if self.peek() == Some(')') {
                    self.pos += 1;
                }
                Some(q)
            }
            '"' => {
                self.pos += 1;
                Some(Query::Exact(self.quoted()))
            }
            '/' => {
                self.pos += 1;
                let mut pattern = String::new();
                let mut closed = false;
                while let Some(c) = self.peek() {
                    self.pos += 1;
                    if c == '\\' {
                        pattern.push(c);
                        if let Some(n) = self.peek() {
                            pattern.push(n);
                            self.pos += 1;
                        }
                    } else if c == '/' {
                        closed = true;
                        break;
                    } else {
                        pattern.push(c);
                    }
                }
                let mut flags = String::new();
                if closed {
                    while let Some(c) = self.peek() {
                        if c.is_ascii_alphabetic() {
                            flags.push(c);
                            self.pos += 1;
                        } else {
                            break;
                        }
                    }
                }
                Some(Query::Regex { pattern, flags })
            }
            '[' => {
                self.pos += 1;
                Some(self.property())
            }
            _ => self.word(close),
        }
    }

    /// Reads to the closing quote; `\"` is a literal quote.
    fn quoted(&mut self) -> String {
        let mut out = String::new();
        while let Some(c) = self.peek() {
            self.pos += 1;
            match c {
                '\\' if self.peek() == Some('"') => {
                    out.push('"');
                    self.pos += 1;
                }
                '"' => break,
                c => out.push(c),
            }
        }
        out
    }

    fn word(&mut self, close: Option<char>) -> Option<Query> {
        let start = self.pos;
        let mut word = String::new();
        while let Some(c) = self.peek() {
            if c.is_whitespace() || c == '(' || c == ')' || Some(c) == close {
                break;
            }
            if c == ':' {
                let prefix = word.as_str();
                let is_prefix = Field::from_prefix(prefix).is_some()
                    || prefix == "match-case"
                    || prefix == "ignore-case";
                if is_prefix {
                    self.pos += 1;
                    let inner = self
                        .unary(close)
                        .unwrap_or_else(|| Query::Text(String::new()));
                    return Some(match prefix {
                        "match-case" => Query::CaseSensitive(Box::new(inner)),
                        "ignore-case" => Query::CaseInsensitive(Box::new(inner)),
                        p => Query::Field(Field::from_prefix(p).unwrap(), Box::new(inner)),
                    });
                }
            }
            if c == '"' && word.is_empty() {
                break;
            }
            word.push(c);
            self.pos += 1;
        }
        if word.is_empty() {
            if self.pos == start {
                // Nothing consumable here (a lone quote or bracket); step over it.
                self.pos += 1;
            }
            return None;
        }
        Some(Query::Text(word))
    }

    /// `[name]`, `[name:value]`; the value grammar is the full query grammar
    /// up to the matching `]`.
    fn property(&mut self) -> Query {
        let mut name = String::new();
        while let Some(c) = self.peek() {
            if c == ']' || c == ':' {
                break;
            }
            name.push(c);
            self.pos += 1;
        }
        // Obsidian writes `["Status"]` from the All properties pane: the
        // quotes allow a space in the name and are not part of it.
        let name = name.trim().trim_matches('"').trim().to_string();
        match self.peek() {
            Some(']') => {
                self.pos += 1;
                Query::HasProperty(name)
            }
            Some(':') => {
                self.pos += 1;
                self.skip_ws();
                let value_start = self.pos;
                let value = self.or(Some(']'));
                let raw: String = self.chars[value_start..self.pos].iter().collect();
                if self.peek() == Some(']') {
                    self.pos += 1;
                }
                let raw = raw.trim();
                if raw == "null" {
                    Query::PropertyEmpty(name)
                } else if raw == "true" {
                    Query::PropertyBool(name, true)
                } else if raw == "false" {
                    Query::PropertyBool(name, false)
                } else if let Some(v) = raw.strip_prefix('>') {
                    Query::PropertyCompare(name, Compare::Greater, v.trim().to_string())
                } else if let Some(v) = raw.strip_prefix('<') {
                    Query::PropertyCompare(name, Compare::Less, v.trim().to_string())
                } else {
                    Query::PropertyMatch(name, Box::new(value))
                }
            }
            _ => Query::HasProperty(name),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Query {
        Query::Text(s.into())
    }

    #[test]
    fn words_are_an_implicit_and_and_or_binds_looser() {
        assert_eq!(parse("a b"), Query::All(vec![text("a"), text("b")]));
        assert_eq!(
            parse("a b OR c"),
            Query::Any(vec![Query::All(vec![text("a"), text("b")]), text("c")])
        );
        assert_eq!(
            parse("(a OR b) c"),
            Query::All(vec![Query::Any(vec![text("a"), text("b")]), text("c")])
        );
        // Lowercase "or" is a word.
        assert_eq!(
            parse("a or b"),
            Query::All(vec![text("a"), text("or"), text("b")])
        );
        assert_eq!(parse("ORDER"), text("ORDER"));
    }

    #[test]
    fn phrases_regexes_and_negation() {
        assert_eq!(
            parse(r#""say \"hi\" now""#),
            Query::Exact(r#"say "hi" now"#.into())
        );
        assert_eq!(
            parse(r"/\d{4}-\d{2}/i rest"),
            Query::All(vec![
                Query::Regex {
                    pattern: r"\d{4}-\d{2}".into(),
                    flags: "i".into()
                },
                text("rest")
            ])
        );
        assert_eq!(parse("-term"), Query::Not(Box::new(text("term"))));
        assert_eq!(
            parse("-(a b)"),
            Query::Not(Box::new(Query::All(vec![text("a"), text("b")])))
        );
        assert_eq!(
            parse("-line:(x)"),
            Query::Not(Box::new(Query::Field(Field::Line, Box::new(text("x")))))
        );
    }

    #[test]
    fn fields_apply_to_one_term_or_group() {
        assert_eq!(
            parse("file:x y"),
            Query::All(vec![
                Query::Field(Field::File, Box::new(text("x"))),
                text("y")
            ])
        );
        assert_eq!(
            parse("path:/\\d{4}/"),
            Query::Field(
                Field::Path,
                Box::new(Query::Regex {
                    pattern: r"\d{4}".into(),
                    flags: String::new()
                })
            )
        );
        assert_eq!(
            parse("line:(a b)"),
            Query::Field(
                Field::Line,
                Box::new(Query::All(vec![text("a"), text("b")]))
            )
        );
        assert_eq!(
            parse("tag:#x task-done:y"),
            Query::All(vec![
                Query::Field(Field::Tag, Box::new(text("#x"))),
                Query::Field(Field::TaskDone, Box::new(text("y")))
            ])
        );
        assert_eq!(
            parse("match-case:Foo ignore-case:\"Bar baz\""),
            Query::All(vec![
                Query::CaseSensitive(Box::new(text("Foo"))),
                Query::CaseInsensitive(Box::new(Query::Exact("Bar baz".into())))
            ])
        );
        // Unknown prefixes are words, as are URLs.
        assert_eq!(parse("http://x.y"), text("http://x.y"));
    }

    #[test]
    fn properties() {
        assert_eq!(parse("[status]"), Query::HasProperty("status".into()));
        assert_eq!(
            parse("[\"Status\"]"),
            Query::HasProperty("Status".into()),
            "the quotes the All properties pane writes are not part of the name"
        );
        assert_eq!(
            parse("[status:null]"),
            Query::PropertyEmpty("status".into())
        );
        assert_eq!(
            parse("[done:true]"),
            Query::PropertyBool("done".into(), true)
        );
        assert_eq!(
            parse("[rating:>5]"),
            Query::PropertyCompare("rating".into(), Compare::Greater, "5".into())
        );
        assert_eq!(
            parse("[status:done OR \"in progress\"] x"),
            Query::All(vec![
                Query::PropertyMatch(
                    "status".into(),
                    Box::new(Query::Any(vec![
                        text("done"),
                        Query::Exact("in progress".into())
                    ]))
                ),
                text("x")
            ])
        );
    }

    #[test]
    fn explain_uses_obsidians_labels() {
        let q = parse(r#"-file:draft (tag:#work OR "exact one") match-case:Case /re/ [p:>3]"#);
        assert_eq!(
            q.explain(),
            "Match all of\n  Excluding\n    Match file name\n      Matches text: \"draft\"\n  Match any of\n    Match tag\n      Matches text: \"#work\"\n    Contains exact text: exact one\n  Case sensitive\n    Matches text: \"Case\"\n  Matches regex: /re/\n  Greater than: p > 3\n"
        );
    }

    #[test]
    fn unbalanced_input_still_parses() {
        assert_eq!(parse("(a b"), Query::All(vec![text("a"), text("b")]));
        assert_eq!(parse("a) b"), Query::All(vec![text("a"), text("b")]));
        assert_eq!(parse("\"open phrase"), Query::Exact("open phrase".into()));
        assert_eq!(
            parse("/open"),
            Query::Regex {
                pattern: "open".into(),
                flags: String::new()
            }
        );
        assert_eq!(parse("[prop"), Query::HasProperty("prop".into()));
        assert!(parse("   ").is_empty());
        // A dangling `-` is the operator with nothing to negate, not a
        // search for a hyphen: `foo -` matches as `foo` does, so a lone one
        // leaves no term behind at all.
        assert!(parse("-").is_empty());
        assert_eq!(parse("foo -"), text("foo"));
        assert_eq!(parse("foo -  "), text("foo"));
        // One still beside a term keeps negating it.
        assert_eq!(parse("-a"), Query::Not(Box::new(text("a"))));
        assert_eq!(
            parse("- a"),
            Query::All(vec![text("-"), text("a")]),
            "a `-` with a term later in the query is unprobed and stays a word"
        );
        // An unclosed trailing `(` and a dangling `AND` keep a group with no
        // term, which never matches; an `AND` with a term after it is a word.
        let nothing = Query::All(Vec::new());
        assert_eq!(
            parse("foo ("),
            Query::All(vec![text("foo"), nothing.clone()])
        );
        assert_eq!(
            parse("foo AND"),
            Query::All(vec![text("foo"), nothing.clone()])
        );
        assert_eq!(parse("(foo AND)"), Query::All(vec![text("foo"), nothing]));
        assert_eq!(
            parse("a AND b"),
            Query::All(vec![text("a"), text("AND"), text("b")])
        );
        assert_eq!(parse("ANDY"), text("ANDY"));
        // A dangling `OR` keeps its empty branch, which the matcher fails.
        assert_eq!(
            parse("foo OR"),
            Query::Any(vec![text("foo"), Query::All(Vec::new())])
        );
    }

    #[test]
    fn half_typed_input_holds_no_matcher() {
        for q in [
            "",
            "   ",
            "(",
            ")",
            "()",
            "\"",
            "OR",
            "AND",
            "-",
            "file:",
            "path:",
            "section:()",
            "[]",
        ] {
            assert!(parse(q).is_empty(), "{q:?} should hold no matcher");
        }
        // `foo (` and `foo AND` hold a matcher that fails, not none at all:
        // Obsidian runs the search and reports "No matches found.".
        for q in [
            "a", "(a", "file:x", "OR a", "-a", "[p]", "/x/", "a (", "a AND",
        ] {
            assert!(!parse(q).is_empty(), "{q:?} holds a matcher");
        }
    }
}
