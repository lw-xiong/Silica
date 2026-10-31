//! The String and RegExp function family, against Obsidian 1.14.5.
//!
//! Every case in [`observed`] is lifted verbatim from the raw probe run
//! `tools/obsidian-probe/results/2026-10-25-bases-expr.json` — the `str` a
//! try rendered, or the `err` Obsidian refused it with — under the run's own
//! seeds: `String` is `"Frothed basalt"`, `Link` is `linkish` (a link to
//! `Pumice`, so its text is 6 characters) and `RegExp` is `/ba(s)alt/`.
//!
//! The cases in [`chosen`] are the ones the probe settled nothing about: they
//! pin the reading `funcs::text` documents rather than an observation, and a
//! later probe may move them.

use silica_bases::{eval_str, FileData, LinkValue, SimpleContext, Value};

/// The run's seeds, as idents a statement can reach.
fn ctx() -> SimpleContext {
    let mut file = FileData {
        name: "Silica.md".into(),
        basename: "Silica".into(),
        path: "Silica.md".into(),
        ..FileData::default()
    };
    file.properties.insert(
        "linkish".to_string(),
        Value::Link(LinkValue {
            path: "Pumice".into(),
            display: None,
        }),
    );
    file.properties
        .insert("spaced".to_string(), Value::Str("  padded  ".into()));
    SimpleContext::new().with_file(file)
}

/// What a statement answers: the cell's text, or the refusal's text.
fn render(source: &str) -> String {
    match eval_str(source, &ctx()) {
        Ok(v) => v.to_string(),
        Err(e) => e.message,
    }
}

fn check(cases: &[(&str, &str)]) {
    let mut bad = Vec::new();
    for (source, want) in cases {
        let got = render(source);
        if got != *want {
            bad.push(format!("  {source}\n    want {want:?}\n    got  {got:?}"));
        }
    }
    assert!(
        bad.is_empty(),
        "{} case(s) differ:\n{}",
        bad.len(),
        bad.join("\n")
    );
}

#[test]
fn observed() {
    check(&[
        // --- String: contains -------------------------------------------
        (
            r#""Frothed basalt".contains()"#,
            "Cannot call function \"contains\", not enough arguments.",
        ),
        (
            r#""Frothed basalt".contains(/zz/)"#,
            "Type error in \"contains\", parameter \"value\". Expected String, given RegExp.",
        ),
        // Two and three arguments answer the *type* error, not "too many
        // arguments." — the parameters are checked before the count.
        (
            r#""Frothed basalt".contains(/zz/, /zz/)"#,
            "Type error in \"contains\", parameter \"value\". Expected String, given RegExp.",
        ),
        (
            r#""Frothed basalt".contains(/zz/, /zz/, /zz/)"#,
            "Type error in \"contains\", parameter \"value\". Expected String, given RegExp.",
        ),
        // --- String: containsAll / containsAny are variadic -------------
        // With no arguments at all they answer, rather than refusing: the
        // vacuous true and false.
        (r#""Frothed basalt".containsAll()"#, "true"),
        (r#""Frothed basalt".containsAny()"#, "false"),
        // --- String: endsWith / startsWith ------------------------------
        (
            r#""Frothed basalt".endsWith()"#,
            "Cannot call function \"endsWith\", not enough arguments.",
        ),
        (
            r#""Frothed basalt".endsWith(/zz/)"#,
            "Type error in \"endsWith\", parameter \"query\". Expected String, given RegExp.",
        ),
        (
            r#""Frothed basalt".endsWith(/zz/, /zz/)"#,
            "Type error in \"endsWith\", parameter \"query\". Expected String, given RegExp.",
        ),
        (
            r#""Frothed basalt".endsWith(/zz/, /zz/, /zz/)"#,
            "Type error in \"endsWith\", parameter \"query\". Expected String, given RegExp.",
        ),
        (
            r#""Frothed basalt".startsWith()"#,
            "Cannot call function \"startsWith\", not enough arguments.",
        ),
        (
            r#""Frothed basalt".startsWith(/zz/)"#,
            "Type error in \"startsWith\", parameter \"query\". Expected String, given RegExp.",
        ),
        (
            r#""Frothed basalt".startsWith(/zz/, /zz/)"#,
            "Type error in \"startsWith\", parameter \"query\". Expected String, given RegExp.",
        ),
        (
            r#""Frothed basalt".startsWith(/zz/, /zz/, /zz/)"#,
            "Type error in \"startsWith\", parameter \"query\". Expected String, given RegExp.",
        ),
        // --- String: the no-argument set --------------------------------
        (r#""Frothed basalt".isEmpty()"#, "false"),
        (r#""Frothed basalt".isTruthy()"#, "true"),
        (r#""Frothed basalt".lower()"#, "frothed basalt"),
        (r#""Frothed basalt".reverse()"#, "tlasab dehtorF"),
        (r#""Frothed basalt".title()"#, "Frothed Basalt"),
        (r#""Frothed basalt".toString()"#, "Frothed basalt"),
        (r#""Frothed basalt".trim()"#, "Frothed basalt"),
        (r#""Frothed basalt".length"#, "14"),
        // --- String: repeat ---------------------------------------------
        (
            r#""Frothed basalt".repeat()"#,
            "Cannot call function \"repeat\", not enough arguments.",
        ),
        (
            r#""Frothed basalt".repeat(/zz/)"#,
            "Type error in \"repeat\", parameter \"count\". Expected Number, given RegExp.",
        ),
        (
            r#""Frothed basalt".repeat(/zz/, /zz/)"#,
            "Type error in \"repeat\", parameter \"count\". Expected Number, given RegExp.",
        ),
        (
            r#""Frothed basalt".repeat(/zz/, /zz/, /zz/)"#,
            "Type error in \"repeat\", parameter \"count\". Expected Number, given RegExp.",
        ),
        // --- String: replace --------------------------------------------
        // One argument is still "not enough": the minimum is two. A RegExp
        // *pattern* is accepted — the refusal names `replacement`.
        (
            r#""Frothed basalt".replace()"#,
            "Cannot call function \"replace\", not enough arguments.",
        ),
        (
            r#""Frothed basalt".replace(/zz/)"#,
            "Cannot call function \"replace\", not enough arguments.",
        ),
        (
            r#""Frothed basalt".replace(/zz/, /zz/)"#,
            "Type error in \"replace\", parameter \"replacement\". Expected String, given RegExp.",
        ),
        (
            r#""Frothed basalt".replace(/zz/, /zz/, /zz/)"#,
            "Type error in \"replace\", parameter \"replacement\". Expected String, given RegExp.",
        ),
        // --- String: slice ----------------------------------------------
        (
            r#""Frothed basalt".slice()"#,
            "Cannot call function \"slice\", not enough arguments.",
        ),
        (
            r#""Frothed basalt".slice(/zz/)"#,
            "Type error in \"slice\", parameter \"start\". Expected Number, given RegExp.",
        ),
        (
            r#""Frothed basalt".slice(/zz/, /zz/)"#,
            "Type error in \"slice\", parameter \"start\". Expected Number, given RegExp.",
        ),
        (
            r#""Frothed basalt".slice(/zz/, /zz/, /zz/)"#,
            "Type error in \"slice\", parameter \"start\". Expected Number, given RegExp.",
        ),
        // --- String: split ----------------------------------------------
        // A RegExp separator is accepted, and one that never matches gives
        // back the whole string as a single-element List, which renders as
        // the string itself.
        (
            r#""Frothed basalt".split()"#,
            "Cannot call function \"split\", not enough arguments.",
        ),
        (r#""Frothed basalt".split(/zz/)"#, "Frothed basalt"),
        // --- Names are case-insensitive, values are not ------------------
        (r#""abc".lower()"#, "abc"),
        (r#""abc".LOWER()"#, "abc"),
        (r#""abc".Lower()"#, "abc"),
        // --- A Link is a String receiver ---------------------------------
        ("linkish.lower()", "pumice"),
        ("linkish.containsAll()", "true"),
        ("linkish.containsAny()", "false"),
        ("linkish.isEmpty()", "false"),
        ("linkish.isTruthy()", "true"),
        ("linkish.length", "6"),
        (
            "linkish.contains()",
            "Cannot call function \"contains\", not enough arguments.",
        ),
        (
            "linkish.contains(/zz/)",
            "Type error in \"contains\", parameter \"value\". Expected String, given RegExp.",
        ),
        (
            "linkish.repeat(/zz/)",
            "Type error in \"repeat\", parameter \"count\". Expected Number, given RegExp.",
        ),
        (
            "linkish.replace(/zz/, /zz/)",
            "Type error in \"replace\", parameter \"replacement\". Expected String, given RegExp.",
        ),
        // --- RegExp: matches, and nothing else ---------------------------
        (
            "/ba(s)alt/.matches()",
            "Cannot call function \"matches\", not enough arguments.",
        ),
        (
            "/ba(s)alt/.matches(/zz/)",
            "Type error in \"matches\", parameter \"value\". Expected String, given RegExp.",
        ),
        (
            "/ba(s)alt/.matches(/zz/, /zz/)",
            "Type error in \"matches\", parameter \"value\". Expected String, given RegExp.",
        ),
        (
            "/ba(s)alt/.matches(/zz/, /zz/, /zz/)",
            "Type error in \"matches\", parameter \"value\". Expected String, given RegExp.",
        ),
        ("/ba(s)alt/.isTruthy()", "true"),
        ("/ba(s)alt/.toString()", "/ba(s)alt/"),
        // The RegExp completion list is `isTruthy`, `isType`, `matches` and
        // `toString`: a RegExp is not a String receiver.
        (
            "/ba(s)alt/.lower()",
            "Cannot find function \"lower\" on type RegExp",
        ),
        (
            "/ba(s)alt/.isEmpty()",
            "Cannot find function \"isEmpty\" on type RegExp",
        ),
        // --- There is no upper() ------------------------------------------
        (
            r#""Frothed basalt".upper()"#,
            "Cannot find function \"upper\" on type String",
        ),
        // --- null absorbs, and isEmpty() is the exception -----------------
        ("nosuchprop.lower()", "null"),
        ("nosuchprop.isEmpty()", "true"),
    ]);
}

#[test]
fn chosen() {
    check(&[
        // contains / containsAll / containsAny do the obvious thing with
        // real arguments; values are case-sensitive.
        (r#""Frothed basalt".contains("basalt")"#, "true"),
        (r#""Frothed basalt".contains("BASALT")"#, "false"),
        (r#""Frothed basalt".containsAll("Frothed", "basalt")"#, "true"),
        (r#""Frothed basalt".containsAll("Frothed", "pumice")"#, "false"),
        (r#""Frothed basalt".containsAny("pumice", "basalt")"#, "true"),
        (r#""Frothed basalt".containsAny("pumice")"#, "false"),
        (
            r#""Frothed basalt".containsAll("a", 1)"#,
            "Type error in \"containsAll\", parameter \"values\". Expected String, given Number.",
        ),
        (r#""Frothed basalt".endsWith("basalt")"#, "true"),
        (r#""Frothed basalt".startsWith("Frothed")"#, "true"),
        (r#""".isEmpty()"#, "true"),
        (r#""  padded  ".trim()"#, "padded"),
        ("spaced.trim()", "padded"),
        // A zero-argument name refuses an argument it has no parameter for.
        (
            r#""Frothed basalt".trim(1)"#,
            "Cannot call function \"trim\", too many arguments.",
        ),
        // repeat: a fractional count truncates, zero and a negative count
        // give nothing.
        (r#""ab".repeat(3)"#, "ababab"),
        (r#""ab".repeat(2.9)"#, "abab"),
        (r#""ab".repeat(0)"#, ""),
        (r#""ab".repeat(-1)"#, ""),
        (
            r#""ab".repeat(1, 2)"#,
            "Cannot call function \"repeat\", too many arguments.",
        ),
        // replace: every occurrence, for a String pattern and a RegExp one
        // alike, and the replacement is literal text.
        (r#""a-b-c".replace("-", "+")"#, "a+b+c"),
        (r#""a-b-c".replace(/-/, "+")"#, "a+b+c"),
        (r#""Frothed basalt".replace(/BA/i, "x")"#, "Frothed xsalt"),
        (r#""Frothed basalt".replace(/ba(s)alt/, "$1")"#, "Frothed $1"),
        (r#""a-b".replace("z", "+")"#, "a-b"),
        (
            r#""a-b".replace(1, "+")"#,
            "Type error in \"replace\", parameter \"pattern\". Expected String,RegExp, given Number.",
        ),
        // A pattern Rust's regex cannot compile matches nothing: a
        // divergence, documented in funcs::text.
        (r#""Frothed basalt".replace(/(?<=x)y/, "!")"#, "Frothed basalt"),
        ("/(?<=x)y/.matches(\"xy\")", "false"),
        // slice: inclusive start, exclusive end, negatives from the end, and
        // out of range clamps.
        (r#""Frothed basalt".slice(8)"#, "basalt"),
        (r#""Frothed basalt".slice(0, 7)"#, "Frothed"),
        (r#""Frothed basalt".slice(-6)"#, "basalt"),
        (r#""Frothed basalt".slice(1, -1)"#, "rothed basal"),
        (r#""Frothed basalt".slice(100)"#, ""),
        (r#""Frothed basalt".slice(5, 2)"#, ""),
        (r#""Frothed basalt".slice(-100, 7)"#, "Frothed"),
        (
            r#""abc".slice(0, 1, 2)"#,
            "Cannot call function \"slice\", too many arguments.",
        ),
        (
            r#""abc".slice(0, /zz/)"#,
            "Type error in \"slice\", parameter \"end\". Expected Number, given RegExp.",
        ),
        // split: a List, rendered by joining with ", "; `n` caps how many
        // substrings come back.
        (r#""a,b,c".split(",")"#, "a, b, c"),
        (r#""a,b,c".split(",", 2)"#, "a, b"),
        (r#""a,b,c".split(",", 0)"#, ""),
        (r#""a1b2c".split(/[0-9]/)"#, "a, b, c"),
        (r#""abc".split("")"#, "a, b, c"),
        (r#""a,b,c".split(",").length"#, "3"),
        (
            r#""a,b".split(",", /zz/)"#,
            "Type error in \"split\", parameter \"n\". Expected Number, given RegExp.",
        ),
        (
            r#""a,b".split(1)"#,
            "Type error in \"split\", parameter \"separator\". Expected String,RegExp, given Number.",
        ),
        // title: the first letter of each whitespace-delimited word, and the
        // rest of a word is left as written.
        (r#""frothed basalt".title()"#, "Frothed Basalt"),
        (r#""FROTHED basalt".title()"#, "FROTHED Basalt"),
        (r#""  two  words ".title()"#, "  Two  Words "),
        (r#""ignimbrite".title()"#, "Ignimbrite"),
        // matches searches, it does not anchor.
        ("/basalt/.matches(\"Frothed basalt\")", "true"),
        ("/BASALT/.matches(\"Frothed basalt\")", "false"),
        ("/BASALT/i.matches(\"Frothed basalt\")", "true"),
        ("/pumice/.matches(\"Frothed basalt\")", "false"),
        // A Link argument is a String argument.
        ("\"Pumice and basalt\".contains(linkish)", "true"),
        // A Link receiver takes the whole String set.
        ("linkish.slice(0, 3)", "Pum"),
        ("linkish.split(\"m\")", "Pu, ice"),
        ("linkish.title()", "Pumice"),
        ("linkish.reverse()", "ecimuP"),
    ]);
}
