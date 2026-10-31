//! The evaluator against Obsidian 1.14.5's own values.
//!
//! The sibling of `obsidian_trees.rs`. That one holds the parser to the tree
//! Obsidian built for each statement; this one holds the evaluator to what
//! Obsidian's own evaluation *answered* for each statement, lifted out of the
//! same CDP run (`tools/obsidian-probe/bases-expr.mjs`, raw run in
//! `tools/obsidian-probe/results/2026-10-25-bases-expr.json`, 2026-10-25).
//! Where the specification and the run disagree, the run wins.
//!
//! # The fixture
//!
//! `obsidian_values.tsv` is one statement per line, sorted and deduplicated:
//!
//! ```text
//! <statement> \t <outcome>
//! ```
//!
//! and an outcome is one of two forms, both carrying a quoted string with
//! Rust's own escapes (`\\`, `\"`, `\n`, `\t`, `\r`):
//!
//! ```text
//! lucide-binary "12.3"     a value: its type tag, then what it rendered as
//! error "Cannot find function \"sum\""   a refusal, in Obsidian's own words
//! ```
//!
//! The type tag is Obsidian's `icon` and is recorded for **every** value, not
//! only where it disambiguates, because it often is the only difference: the
//! Number `12.3` and the String `12.3` that `12.3.toFixed(1)` returns render
//! identically and are told apart by `lucide-binary` against `lucide-text`.
//!
//! # What "rendered as" means
//!
//! Not quite `toString()`. The probe read a value as
//! `String(value.data !== undefined ? value.data : value)`, and Obsidian
//! carries a String, Number, Boolean, List, Object, Tags and a rendered
//! icon/image as `{icon, data}` while a Date, Duration, RegExp, Formulas and
//! null have no `data`. So for most types the probe read `toString()`, but
//! for a List it read JavaScript's `Array.prototype.toString` over the
//! elements — `","` between elements, each element its own `toString()` — and
//! for an Object it read the useless `[object Object]`. [`rendered`] below
//! reproduces that rule rather than calling [`Value::to_string`] blindly, so
//! that the fixture asks for what was actually observed.
//!
//! The distinction is observable in the fixture itself:
//! `file.properties.values()` is a List whose elements are Lists, and it
//! rendered as `...,alpha, beta, alpha,3, 1, 2,...` — the outer join is `","`
//! and the inner one is Obsidian's own `List.toString()`, `", "`.
//!
//! # The row
//!
//! Every statement was evaluated against one row of one view: `Silica.md` in
//! the probe's fixture vault, shown by `Main.base`. [`silica_row`] reproduces
//! that note's frontmatter, and `now` is pinned to the instant the run's clock
//! read, so nothing here is a clock race.
//!
//! # What is left out, and why
//!
//! - **A bare Link or File.** The probe's `str` is `String(value.data)` when a
//!   value has `data`, and the run could not serialize a Link or a File
//!   (`link("Pumice")` answered `ERR:Converting circular structure to JSON`)
//!   to say whether it had one. It matters: `linkish` rendered as `Pumice`
//!   while the very same Link inside `file.links` rendered as
//!   `[[Pumice|Pumice]]`. The observation does not pin `toString`, so it is
//!   dropped. A *List of* Links is not ambiguous — `Array.prototype.join`
//!   calls each element's `toString` — so `file.links` and `file.backlinks`
//!   stay, and they are what pins the Link rendering.
//! - **The wall clock and `random()`.** The run read `now()` at 15:28:49 in
//!   one step and 15:28:50 in the next, so no statement that renders the clock
//!   to the second can be pinned to a single answer — `now()` itself,
//!   `now() - "1 week"`, `now().time()`, `now().second` and the fractional
//!   totals of `(now() - when)` are all out. What survives is coarser than the
//!   run's own span: `today()`, `now().date()`, `now() - when` as a humanized
//!   duration. `file.ctime` also reads the run's clock, but it is the fixture
//!   vault's own mtime and [`silica_row`] pins it, so it stays.
//! - **A key that is a label rather than a statement.** This is the trap the
//!   parser's extractor hit. `e11-cells.exprs` is keyed by a column name
//!   (`cStr`, `cNum`) and `e10-invalid`'s formulas by a formula name
//!   (`broken`, `alsoBroken`); the fixture is built from `e11-cells.oracle`,
//!   which is keyed by the statement, and `e10-invalid` is not read at all.
//!   Where a step keys by a *function* name (`e08-methods`, `e14-function-
//!   table`, `e15-globals-table`) the statement is rebuilt from the step's own
//!   seed, exactly as the probe built it: `seed + "." + name + "(" + args +
//!   ")"`.
//! - **`e11-cells.table`.** Whole cells rendered in a real view, for every row
//!   of the vault rather than this one, and a cell is the *view's* rendering:
//!   a null renders as an empty cell there while `toString` is `"null"`. That
//!   is the view's business, not the evaluator's.
//! - **Two statements the run answered with a bare JavaScript `null`**
//!   (`min()` and `max()`), which carry no type tag at all.

use silica_bases::{eval_str, DateValue, FileData, LinkValue, SimpleContext, Value};

/// What the probe read a value as. See the module comment: this is
/// `String(value.data ?? value)`, not `toString()`.
fn rendered(v: &Value) -> String {
    match v {
        // `String(array)` is `Array.prototype.join(",")`, which calls each
        // element's own `toString`.
        Value::List(items) => items
            .iter()
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(","),
        Value::Tags(tags) => tags.join(","),
        // `String(plainObject)`.
        Value::Object(_) => "[object Object]".to_string(),
        other => other.to_string(),
    }
}

/// The instant the run's clock read, as `now()`.
fn pinned_now() -> DateValue {
    DateValue {
        at: "2026-10-25T15:28:50"
            .parse()
            .expect("the pinned clock is a date-time"),
        has_time: true,
    }
}

/// When the probe wrote the fixture vault, which is `file.ctime` and
/// `file.mtime` for every note in it.
fn vault_written() -> DateValue {
    DateValue {
        at: "2026-10-25T15:28:44"
            .parse()
            .expect("the vault's mtime is a date-time"),
        has_time: true,
    }
}

fn civil(s: &str) -> DateValue {
    DateValue {
        at: s.parse().expect("a fixture date"),
        has_time: false,
    }
}

fn stamp(s: &str) -> DateValue {
    DateValue {
        at: s.parse().expect("a fixture date-time"),
        has_time: true,
    }
}

fn link(path: &str) -> LinkValue {
    LinkValue {
        path: path.to_string(),
        display: None,
    }
}

fn displayed(path: &str, display: &str) -> LinkValue {
    LinkValue {
        path: path.to_string(),
        display: Some(display.to_string()),
    }
}

fn strs(items: &[&str]) -> Value {
    Value::List(items.iter().map(|s| Value::Str(s.to_string())).collect())
}

/// A note of the probe's fixture vault. `name` is Obsidian's own: the path
/// without a `.md` extension, which is why the base file below is called
/// `Main.base` and this one `Silica`.
fn note(path: &str, properties: &[(&str, Value)]) -> FileData {
    let (basename, ext) = path.rsplit_once('.').unwrap_or((path, ""));
    let mut file = FileData {
        name: path.strip_suffix(".md").unwrap_or(path).to_string(),
        basename: basename.to_string(),
        fullname: path.to_string(),
        path: path.to_string(),
        folder: "/".to_string(),
        ext: ext.to_string(),
        ctime: Some(vault_written()),
        mtime: Some(vault_written()),
        ..FileData::default()
    };
    for (k, v) in properties {
        file.properties.insert(k.to_string(), v.clone());
    }
    file
}

/// `Silica.md`, the row every statement was evaluated against. The frontmatter
/// is the probe's, in file order; `file.size` and the tag list are what the run
/// read back out of Obsidian.
fn silica_row() -> FileData {
    let mut file = note(
        "Silica.md",
        &[
            ("type", Value::Str("rock".into())),
            ("status", Value::Str("todo".into())),
            ("density", Value::Number(1.21)),
            ("silica", Value::Number(50.0)),
            ("count", Value::Number(3.0)),
            ("ratio", Value::Number(0.5)),
            ("negative", Value::Number(-4.0)),
            ("big", Value::Number(1000000.0)),
            ("zeroish", Value::Number(0.0)),
            ("flag", Value::Bool(true)),
            ("off", Value::Bool(false)),
            ("when", Value::Date(civil("2026-01-02T00:00:00"))),
            ("stamp", Value::Date(stamp("2026-01-02T03:04:05"))),
            ("text", Value::Str("Frothed basalt".into())),
            ("numish", Value::Str("42".into())),
            ("spaced", Value::Str("  padded  ".into())),
            ("empty", Value::Str(String::new())),
            ("listish", strs(&["alpha", "beta", "alpha"])),
            (
                "nums",
                Value::List(vec![
                    Value::Number(3.0),
                    Value::Number(1.0),
                    Value::Number(2.0),
                ]),
            ),
            ("linkish", Value::Link(link("Pumice"))),
            (
                "tags",
                Value::Tags(vec!["#rock".into(), "#igneous/extrusive".into()]),
            ),
            ("aliases", strs(&["Cinder"])),
        ],
    );
    file.size = 483.0;
    file.tags = vec!["#rock".into(), "#igneous/extrusive".into()];
    // Observed through `file.links`: a link carries the text it was written
    // with as its display, and `Silica.md` links to Pumice twice and Basalt
    // once.
    file.links = vec![
        displayed("Pumice", "Pumice"),
        displayed("Pumice", "Pumice"),
        displayed("Basalt", "Basalt"),
    ];
    file.backlinks = vec![displayed("Pumice.md", "Pumice")];
    file
}

/// The view's own base file. **`this` is the base file, not the row.** It has
/// no formulas: `formula.ppu` was refused with `Cannot find "ppu" on type
/// Formulas`.
fn main_base() -> FileData {
    let mut file = note("Main.base", &[]);
    file.size = 0.0;
    file
}

fn context() -> SimpleContext {
    let mut ctx = SimpleContext::new()
        .with_file(silica_row())
        .with_now(pinned_now().at);
    ctx.base_file = Some(std::sync::Arc::new(main_base()));
    for f in [
        note(
            "Pumice.md",
            &[
                ("type", Value::Str("rock".into())),
                ("status", Value::Str("done".into())),
                ("density", Value::Number(0.64)),
                ("silica", Value::Number(72.0)),
                ("count", Value::Number(10.0)),
                ("when", Value::Date(civil("2026-03-04T00:00:00"))),
                ("tags", Value::Tags(vec!["#rock".into()])),
            ],
        ),
        note(
            "Basalt.md",
            &[
                ("type", Value::Str("rock".into())),
                ("status", Value::Str("todo".into())),
                ("density", Value::Number(2.9)),
                ("silica", Value::Number(49.0)),
                ("tags", Value::Tags(vec!["#rock".into()])),
            ],
        ),
        note("Bare.md", &[]),
    ] {
        ctx = ctx.with_vault_file(f);
    }
    ctx
}

/// Which family of the function table owns a statement, so a failing run can
/// be handed back to the five of them separately. A reporting aid, not a
/// specification: the receiver decides, and a bare operator falls to the
/// arithmetic bucket.
fn family(source: &str) -> &'static str {
    const STRING: &str = "String/RegExp";
    const NUMBER: &str = "Number and the globals";
    const LIST: &str = "List/Object";
    const TIME: &str = "Date/Duration";
    const FILE: &str = "File/Link";

    // `file.properties` is an Object, so it is the List/Object family's even
    // though it is reached through `file`.
    if source.starts_with("file.properties") || source.starts_with("this.note") {
        return LIST;
    }
    let receiver = head_operand(receiver_of(source));
    match receiver {
        "file" | "this" | "linkish" => return FILE,
        "listish" | "nums" | "aliases" | "tags" | "note" | "values" => return LIST,
        "now()" | "today()" | "when" | "stamp" | "(now() - when)" => return TIME,
        "text" | "status" | "numish" | "spaced" | "empty" => return STRING,
        "density" | "silica" | "count" | "ratio" | "negative" | "big" | "zeroish" => {
            return NUMBER;
        }
        _ => {}
    }
    if receiver.starts_with('/') {
        return STRING;
    }
    if receiver.starts_with('"') || receiver.starts_with('\'') {
        return STRING;
    }
    if receiver.starts_with('[') {
        return LIST;
    }
    if receiver.starts_with("file.") || receiver.starts_with("this.") {
        return if receiver.starts_with("file.properties") {
            LIST
        } else {
            FILE
        };
    }
    for (head, fam) in [
        ("date(", TIME),
        ("duration(", TIME),
        ("link(", FILE),
        ("file(", FILE),
        ("list(", LIST),
    ] {
        if receiver.starts_with(head) {
            return fam;
        }
    }
    NUMBER
}

/// The left operand of a statement that is an operator expression rather than
/// a call: `when + "1 month"` is the Date family's, not arithmetic's.
fn head_operand(receiver: &str) -> &str {
    if receiver.starts_with(['(', '[', '"', '\'', '/']) {
        return receiver;
    }
    match receiver.split_once(' ') {
        Some((head, _)) => head,
        None => receiver,
    }
}

/// The text before the first `.` that is not inside a string, a bracket or a
/// regexp — what a method call is called *on*.
fn receiver_of(source: &str) -> &str {
    let bytes = source.as_bytes();
    let (mut depth, mut quote, mut i) = (0i32, None::<u8>, 0usize);
    while i < bytes.len() {
        let c = bytes[i];
        match quote {
            Some(q) => {
                if c == b'\\' {
                    i += 1;
                } else if c == q {
                    quote = None;
                }
            }
            None => match c {
                b'"' | b'\'' | b'/' => quote = Some(c),
                b'(' | b'[' => depth += 1,
                b')' | b']' => depth -= 1,
                // A digit either side makes it a decimal point, not access.
                b'.' if depth == 0
                    && !(i > 0 && bytes[i - 1].is_ascii_digit())
                    && !(i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit()) =>
                {
                    return &source[..i]
                }
                _ => {}
            },
        }
        i += 1;
    }
    source
}

/// The fixture's quoting: a Rust-style string literal.
fn unquote(field: &str) -> Option<String> {
    let inner = field.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next()? {
            'n' => out.push('\n'),
            't' => out.push('\t'),
            'r' => out.push('\r'),
            '"' => out.push('"'),
            '\\' => out.push('\\'),
            // The fixture writes no other escape.
            _ => return None,
        }
    }
    Some(out)
}

#[test]
fn every_statement_evaluates_to_what_obsidian_answered() {
    let fixture = include_str!("obsidian_values.tsv");
    let ctx = context();
    let mut checked = 0usize;
    let mut wrong: Vec<(&'static str, String)> = Vec::new();

    for line in fixture.lines() {
        let Some((source, outcome)) = line.split_once('\t') else {
            continue;
        };
        let Some((tag, quoted)) = outcome.split_once(' ') else {
            panic!("fixture line is not `<tag> <quoted>`: {line}");
        };
        let expected = unquote(quoted).unwrap_or_else(|| panic!("unquotable fixture line: {line}"));
        checked += 1;

        let got = eval_str(source, &ctx);
        let mismatch = match (tag, &got) {
            ("error", Err(e)) if e.message == expected => None,
            ("error", Err(e)) => Some(format!(
                "obsidian refused: {expected}\n    silica refused:   {}",
                e.message
            )),
            ("error", Ok(v)) => Some(format!(
                "obsidian refused: {expected}\n    silica answered:  {} {:?}",
                v.icon(),
                rendered(v)
            )),
            (_, Err(e)) => Some(format!(
                "obsidian answered: {tag} {expected:?}\n    silica refused:    {}",
                e.message
            )),
            (_, Ok(v)) => {
                let text = rendered(v);
                if v.icon() == tag && text == expected {
                    None
                } else {
                    Some(format!(
                        "obsidian answered: {tag} {expected:?}\n    silica answered:   {} {text:?}",
                        v.icon()
                    ))
                }
            }
        };
        if let Some(detail) = mismatch {
            wrong.push((family(source), format!("  {source}\n    {detail}")));
        }
    }

    // A floor, so that a fixture that stops being read fails here rather than
    // passing while checking nothing.
    assert!(checked >= 1044, "only {checked} statements were compared");

    let passed = checked - wrong.len();
    println!("obsidian_values: {passed} of {checked} statements match Obsidian 1.14.5");
    let mut families: Vec<&'static str> = wrong.iter().map(|(f, _)| *f).collect();
    families.sort_unstable();
    families.dedup();
    for fam in &families {
        let n = wrong.iter().filter(|(f, _)| f == fam).count();
        println!("  {n:5} failing, owned by {fam}");
    }

    if !wrong.is_empty() {
        let mut report = String::new();
        for fam in &families {
            let lines: Vec<&str> = wrong
                .iter()
                .filter(|(f, _)| f == fam)
                .map(|(_, l)| l.as_str())
                .collect();
            report.push_str(&format!("\n=== {} ({} failing)\n", fam, lines.len()));
            report.push_str(&lines.join("\n"));
            report.push('\n');
        }
        panic!(
            "{} of {checked} statements evaluated differently to Obsidian:\n{report}",
            wrong.len()
        );
    }
}
