//! The List and Object function family, against Obsidian 1.14.5.
//!
//! Every case below is lifted from the raw probe run
//! `tools/obsidian-probe/results/2026-10-25-bases-expr.json` — steps
//! `e14-function-table` (`byType.List`, `byType.Object`), `e04-lambda`,
//! `e16-reduce-and-duration`, `e06-values`, `e11-cells` and
//! `e17-case-and-namespaces` — with the expression and Obsidian's own rendered
//! `str` (or its refusal) copied across unchanged.
//!
//! **The expected text is the probe's `str`, not `toString()`.** They differ
//! for a List: the cell renders `[3, 1, 2]` as `3,1,2` while
//! `[3, 1, 2].toString()` is the String `3, 1, 2`. [`render`] reproduces the
//! cell's rendering so a case can be copied verbatim; `toString()` is checked
//! as its own case.
//!
//! The fixtures are the probe's own: the frontmatter of its `Silica.md`, in
//! file order, as `e08-methods.byType.object.value.data` records it.

use chrono::NaiveDate;
use indexmap::IndexMap;
use silica_bases::value::{DateValue, LinkValue};
use silica_bases::{eval_str, FileData, SimpleContext, Value};

/// What the probe recorded for a case: a rendered value, or a refusal.
enum Want {
    Str(&'static str),
    Err(&'static str),
}

use Want::{Err as E, Str as S};

/// The probe's `str`: what a cell shows. A List renders its elements' own
/// `toString()` joined with `,` — note the missing space, which is what
/// separates this from `List.toString()` — and an Object renders as
/// JavaScript's `[object Object]`.
fn render(v: &Value) -> String {
    match v {
        Value::List(items) => items
            .iter()
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(","),
        Value::Tags(tags) => tags.join(","),
        Value::Object(_) => "[object Object]".to_string(),
        other => other.to_string(),
    }
}

fn date(y: i32, m: u32, d: u32) -> Value {
    Value::Date(DateValue {
        at: NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap(),
        has_time: false,
    })
}

fn stamp(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> Value {
    Value::Date(DateValue {
        at: NaiveDate::from_ymd_opt(y, mo, d)
            .unwrap()
            .and_hms_opt(h, mi, s)
            .unwrap(),
        has_time: true,
    })
}

fn strings(items: &[&str]) -> Value {
    Value::List(items.iter().map(|s| Value::str(*s)).collect())
}

/// `Silica.md`, the note every value case was evaluated against.
fn ctx() -> SimpleContext {
    let mut props: IndexMap<String, Value> = IndexMap::new();
    props.insert("type".into(), Value::str("rock"));
    props.insert("status".into(), Value::str("todo"));
    props.insert("density".into(), Value::Number(1.21));
    props.insert("silica".into(), Value::Number(50.0));
    props.insert("count".into(), Value::Number(3.0));
    props.insert("ratio".into(), Value::Number(0.5));
    props.insert("negative".into(), Value::Number(-4.0));
    props.insert("big".into(), Value::Number(1_000_000.0));
    props.insert("zeroish".into(), Value::Number(0.0));
    props.insert("flag".into(), Value::Bool(true));
    props.insert("off".into(), Value::Bool(false));
    props.insert("when".into(), date(2026, 1, 2));
    props.insert("stamp".into(), stamp(2026, 1, 2, 3, 4, 5));
    props.insert("text".into(), Value::str("Frothed basalt"));
    props.insert("numish".into(), Value::str("42"));
    props.insert("spaced".into(), Value::str("  padded  "));
    props.insert("empty".into(), Value::str(""));
    props.insert("listish".into(), strings(&["alpha", "beta", "alpha"]));
    props.insert(
        "nums".into(),
        Value::List(vec![
            Value::Number(3.0),
            Value::Number(1.0),
            Value::Number(2.0),
        ]),
    );
    props.insert(
        "linkish".into(),
        Value::Link(LinkValue {
            path: "Pumice".into(),
            display: None,
        }),
    );
    props.insert(
        "tags".into(),
        Value::Tags(vec!["#rock".into(), "#igneous/extrusive".into()]),
    );
    props.insert("aliases".into(), strings(&["Cinder"]));

    SimpleContext::new().with_file(FileData {
        name: "Silica".into(),
        basename: "Silica".into(),
        fullname: "Silica.md".into(),
        path: "Silica.md".into(),
        folder: "/".into(),
        ext: "md".into(),
        size: 483.0,
        tags: vec!["#rock".into(), "#igneous/extrusive".into()],
        properties: props,
        ..FileData::default()
    })
}

#[track_caller]
fn check(cases: &[(&str, Want)]) {
    let ctx = ctx();
    let mut bad = Vec::new();
    for (expr, want) in cases {
        let got = eval_str(expr, &ctx);
        let ok = match (want, &got) {
            (S(s), Ok(v)) => render(v) == *s,
            (E(m), Err(e)) => e.message == *m,
            _ => false,
        };
        if !ok {
            let want = match want {
                S(s) => format!("value {s:?}"),
                E(m) => format!("error {m:?}"),
            };
            let got = match got {
                Ok(v) => format!("value {:?}", render(&v)),
                Err(e) => format!("error {:?}", e.message),
            };
            bad.push(format!("{expr}\n    want {want}\n    got  {got}"));
        }
    }
    assert!(
        bad.is_empty(),
        "{} case(s):\n  {}",
        bad.len(),
        bad.join("\n  ")
    );
}

/// `e14-function-table`, `byType.List`: the brute-force pass over every name,
/// on the seed `[3, 1, 2]`.
#[test]
fn function_table_list() {
    check(&[
        (
            "[3, 1, 2].contains()",
            E("Cannot call function \"contains\", not enough arguments."),
        ),
        ("[3, 1, 2].contains(/zz/)", S("false")),
        ("[3, 1, 2].containsAll()", S("true")),
        ("[3, 1, 2].containsAny()", S("false")),
        ("[3, 1, 2].earliest()", S("null")),
        (
            "[3, 1, 2].filter()",
            E("Cannot call function \"filter\", not enough arguments."),
        ),
        ("[3, 1, 2].filter(/zz/)", S("3,1,2")),
        ("[3, 1, 2].flat()", S("3,1,2")),
        ("[3, 1, 2].isEmpty()", S("false")),
        ("[3, 1, 2].isTruthy()", S("true")),
        (
            "[3, 1, 2].join()",
            E("Cannot call function \"join\", not enough arguments."),
        ),
        (
            "[3, 1, 2].join(/zz/)",
            E("Type error in \"join\", parameter \"separator\". Expected String, given RegExp."),
        ),
        (
            "[3, 1, 2].join(/zz/, /zz/)",
            E("Type error in \"join\", parameter \"separator\". Expected String, given RegExp."),
        ),
        (
            "[3, 1, 2].join(/zz/, /zz/, /zz/)",
            E("Type error in \"join\", parameter \"separator\". Expected String, given RegExp."),
        ),
        ("[3, 1, 2].latest()", S("null")),
        (
            "[3, 1, 2].map()",
            E("Cannot call function \"map\", not enough arguments."),
        ),
        ("[3, 1, 2].map(/zz/)", S("/zz/,/zz/,/zz/")),
        ("[3, 1, 2].max()", S("3")),
        ("[3, 1, 2].mean()", S("2")),
        ("[3, 1, 2].median()", S("2")),
        ("[3, 1, 2].min()", S("1")),
        (
            "[3, 1, 2].reduce()",
            E("Cannot call function \"reduce\", not enough arguments."),
        ),
        (
            "[3, 1, 2].reduce(/zz/)",
            E("Cannot call function \"reduce\", not enough arguments."),
        ),
        ("[3, 1, 2].reduce(/zz/, /zz/)", S("/zz/")),
        ("[3, 1, 2].reverse()", S("2,1,3")),
        (
            "[3, 1, 2].slice()",
            E("Cannot call function \"slice\", not enough arguments."),
        ),
        (
            "[3, 1, 2].slice(/zz/)",
            E("Type error in \"slice\", parameter \"start\". Expected Number, given RegExp."),
        ),
        (
            "[3, 1, 2].slice(/zz/, /zz/)",
            E("Type error in \"slice\", parameter \"start\". Expected Number, given RegExp."),
        ),
        (
            "[3, 1, 2].slice(/zz/, /zz/, /zz/)",
            E("Type error in \"slice\", parameter \"start\". Expected Number, given RegExp."),
        ),
        ("[3, 1, 2].sort()", S("1,2,3")),
        ("[3, 1, 2].stddev()", S("0.816496580927726")),
        ("[3, 1, 2].sum()", S("6")),
        // The one case where the cell's text and `toString()` differ.
        ("[3, 1, 2].toString()", S("3, 1, 2")),
        ("[3, 1, 2].unique()", S("1,2,3")),
        ("[3, 1, 2].length", S("3")),
    ]);
}

/// `e04-lambda`: there is no lambda, and the argument is an ordinary
/// expression run per element.
#[test]
fn no_lambda() {
    check(&[
        ("listish.filter(value != \"beta\")", S("alpha,alpha")),
        ("listish.filter(value)", S("alpha,beta,alpha")),
        (
            "listish.filter(v, v != \"beta\")",
            E("Cannot call function \"filter\", too many arguments."),
        ),
        // A non-empty String is truthy, so a "quoted lambda" keeps everything.
        ("listish.filter(\"value != 'beta'\")", S("alpha,beta,alpha")),
        ("nums.map(value * 2)", S("6,2,4")),
        ("nums.map(value)", S("3,1,2")),
        // An unknown ident is null, not an error.
        ("nums.map(item * 2)", S("null,null,null")),
        (
            "nums.reduce(value + 1)",
            E("Cannot call function \"reduce\", not enough arguments."),
        ),
        (
            "nums.reduce(acc + value)",
            E("Cannot call function \"reduce\", not enough arguments."),
        ),
        ("nums.reduce(value, 0)", S("2")),
        (
            "nums.sort(value)",
            E("Cannot call function \"sort\", too many arguments."),
        ),
        ("nums.filter(index > 0)", S("1,2")),
        ("nums.map(index)", S("0,1,2")),
        (
            "nums.map(value, index)",
            E("Cannot call function \"map\", too many arguments."),
        ),
    ]);
}

/// `e04-lambda`: the names the List table does *not* know. Each was tried with
/// the argument a JavaScript programmer would reach for, and each is refused.
#[test]
fn names_the_table_does_not_know() {
    check(&[
        (
            "nums.find(value > 1)",
            E("Cannot find function \"find\" on type List"),
        ),
        (
            "nums.every(value > 0)",
            E("Cannot find function \"every\" on type List"),
        ),
        (
            "nums.some(value > 2)",
            E("Cannot find function \"some\" on type List"),
        ),
        (
            "nums.flatMap(value)",
            E("Cannot find function \"flatMap\" on type List"),
        ),
        (
            "nums.indexOf(1)",
            E("Cannot find function \"indexOf\" on type List"),
        ),
        // A Tags receiver names itself List in a refusal.
        (
            "tags.includes()",
            E("Cannot find function \"includes\" on type List"),
        ),
    ]);
}

/// `e16-reduce-and-duration`: which idents bind inside `reduce`.
#[test]
fn reduce_binds_only_acc() {
    check(&[
        ("nums.reduce(value, 0)", S("2")),
        ("nums.reduce(value, 100)", S("2")),
        ("nums.reduce(acc + value, 0)", S("6")),
        ("nums.reduce(accumulator + value, 0)", S("null")),
        ("nums.reduce(total + value, 0)", S("null")),
        ("nums.reduce(result + value, 0)", S("null")),
        ("nums.reduce(previous + value, 0)", S("null")),
        ("nums.reduce(carry + value, 0)", S("null")),
        ("nums.reduce(sum + value, 0)", S("null")),
        ("nums.reduce(current + value, 0)", S("null")),
        ("nums.reduce(prev + value, 0)", S("null")),
        ("nums.reduce(a + value, 0)", S("null")),
        ("nums.reduce(acc, 0)", S("0")),
        // `index` binds inside `reduce` as well, which the signature does not
        // say.
        ("nums.reduce(index, 0)", S("2")),
        ("nums.reduce(value + index, 0)", S("4")),
    ]);
}

/// `e06-values`, `e11-cells`, `e16` and `e17`: the chained cases, on the
/// fixture properties.
#[test]
fn chained_on_fixtures() {
    check(&[
        ("listish", S("alpha,beta,alpha")),
        ("listish.length", S("3")),
        ("listish.unique()", S("alpha,beta")),
        ("listish.join(\"|\")", S("alpha|beta|alpha")),
        ("listish[0]", S("alpha")),
        ("listish.contains(\"alpha\")", S("true")),
        ("listish.contains(\"ALPHA\")", S("false")),
        ("listish.filter(value != \"beta\").length", S("2")),
        (
            "listish.filter(value != \"beta\").join(\"|\")",
            S("alpha|alpha"),
        ),
        ("nums.map(value * 2).sum()", S("12")),
        ("nums.map(value * 2).join(\"|\")", S("6|2|4")),
        ("nums.filter(value > 1).join(\"-\")", S("3-2")),
        ("aliases", S("Cinder")),
        ("tags", S("#rock,#igneous/extrusive")),
        // Case-insensitive, like every other name in the language.
        ("[1,2].sum()", S("3")),
        ("[1,2].SUM()", S("3")),
    ]);
}

/// `e14-function-table`, `byType.Object`, on the seed `file.properties` —
/// written here as `note`, which the probe shows is the same Object
/// (`note.density` and `density` and `file.properties.density` all answer
/// 1.21), so the Object cases do not depend on the File family.
#[test]
fn function_table_object() {
    check(&[
        (
            "note.filter()",
            E("Cannot call function \"filter\", not enough arguments."),
        ),
        ("note.filter(/zz/)", S("[object Object]")),
        ("note.isEmpty()", S("false")),
        ("note.isTruthy()", S("true")),
        (
            "note.keys()",
            S(
                "type,status,density,silica,count,ratio,negative,big,zeroish,flag,off,when,stamp,\
               text,numish,spaced,empty,listish,nums,linkish,tags,aliases",
            ),
        ),
        (
            "note.map()",
            E("Cannot call function \"map\", not enough arguments."),
        ),
        (
            "note.map(/zz/)",
            S(
                "/zz/,/zz/,/zz/,/zz/,/zz/,/zz/,/zz/,/zz/,/zz/,/zz/,/zz/,/zz/,/zz/,/zz/,/zz/,/zz/,\
               /zz/,/zz/,/zz/,/zz/,/zz/,/zz/",
            ),
        ),
        (
            "note.values()",
            S(
                "rock,todo,1.21,50,3,0.5,-4,1000000,0,true,false,2026-01-02,2026-01-02T03:04:05,\
               Frothed basalt,42,  padded  ,,alpha, beta, alpha,3, 1, 2,[[Pumice]],\
               #rock, #igneous/extrusive,Cinder",
            ),
        ),
        // `e08-methods.byType.object.calls`: a name off the prototype is not a
        // name in the table.
        (
            "note.equals()",
            E("Cannot find function \"equals\" on type Object"),
        ),
        (
            "note.valuesRaw()",
            E("Cannot find function \"valuesRaw\" on type Object"),
        ),
        ("note.nosuch", S("null")),
        ("note.density", S("1.21")),
    ]);
}

/// Behaviour the probe did not reach. Each of these is a *choice*, recorded so
/// a later probe can knock it over cheaply; see the notes in
/// `funcs/list.rs`.
#[test]
fn unprobed_choices() {
    check(&[
        // No number in the list: null, as `earliest()` answers null for a list
        // with no date.
        ("[].sum()", S("null")),
        ("[].mean()", S("null")),
        ("[].median()", S("null")),
        ("[].min()", S("null")),
        ("[].max()", S("null")),
        ("[].stddev()", S("null")),
        ("[].isEmpty()", S("true")),
        ("[].unique()", S("")),
        // A non-number is skipped, not coerced: "the number values in the
        // list".
        ("[1, \"nope\", 3].sum()", S("4")),
        ("[1, \"nope\", 3].mean()", S("2")),
        ("[1, null, 3].min()", S("1")),
        // `"42"` is a String, so it is not one of the number values.
        ("[numish].sum()", S("null")),
        // An even count takes the mean of the two middle numbers.
        ("[4, 1, 3, 2].median()", S("2.5")),
        // `flat()` goes all the way down.
        ("[[1, [2, 3]], 4].flat()", S("1,2,3,4")),
        // `slice` is start-inclusive, end-exclusive, and clamps.
        ("nums.slice(1)", S("1,2")),
        ("nums.slice(0, 2)", S("3,1")),
        ("nums.slice(2, 1)", S("")),
        ("nums.slice(-1)", S("3,1,2")),
        ("nums.slice(0, 99)", S("3,1,2")),
        (
            "nums.slice(0, 1, 2)",
            E("Cannot call function \"slice\", too many arguments."),
        ),
        // `containsAll` / `containsAny` are variadic, each argument a value.
        ("listish.containsAll(\"alpha\", \"beta\")", S("true")),
        ("listish.containsAll(\"alpha\", \"gamma\")", S("false")),
        ("listish.containsAny(\"gamma\", \"beta\")", S("true")),
        // `contains` compares the way `==` compares.
        ("nums.contains(\"3\")", S("true")),
        // `earliest` / `latest` pick over the date values and ignore the
        // rest. They also read a String that parses as a date, which this
        // table cannot show yet: `funcs::time::parse_date` is still a stub, so
        // `["2026-03-01", when].latest()` would answer `when`.
        ("[when, stamp].earliest()", S("2026-01-02")),
        ("[when, stamp].latest()", S("2026-01-02T03:04:05")),
        ("[when, \"nope\"].latest()", S("2026-01-02")),
        // `sort` puts two numbers in numeric order and everything else in the
        // order of its rendered text; the sort is stable.
        ("[\"b\", \"a\", \"c\"].sort()", S("a,b,c")),
        ("[10, 9].sort()", S("9,10")),
        ("[\"10\", \"9\"].sort()", S("10,9")),
        // `unique` is keyed by rendered text, and the distinct elements come
        // back in a plain object's key order: index-like keys ascending, then
        // the rest first-seen.
        ("[\"beta\", \"alpha\", \"beta\"].unique()", S("beta,alpha")),
        ("[2, \"2\", 1].unique()", S("1,2")),
        // `Object.filter` binds `value`; `key` is Obsidian's own documented
        // name for the other one and is not bound here — it needs a `key`
        // field on `Scope`.
        ("note.filter(value == \"rock\").keys()", S("type")),
        ("note.map(value).length", S("22")),
    ]);
}
