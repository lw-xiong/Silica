//! The evaluator's own rules, as against the function families: operators,
//! coercion, null, the namespaces and the magic idents.
//!
//! Every case here is an observation from the CDP run of 2026-10-25
//! (`tools/obsidian-probe/results/2026-10-25-bases-expr.json`, steps
//! `e06-values`, `e09-globals`, `e17-case-and-namespaces`) or from the reading
//! of it in
//! [[Specification#Observed Bases expression language, Obsidian 1.14.5 (installer build 1.13.7)]].

use indexmap::IndexMap;
use silica_bases::{eval_str, matches_filter, FileData, LinkValue, SimpleContext, Value};

/// The run's own row: `Silica.md` at the vault root, with the frontmatter the
/// probe seeded.
fn row() -> SimpleContext {
    let mut properties = IndexMap::new();
    properties.insert("density".into(), Value::Number(2.4));
    properties.insert("status".into(), Value::Str("todo".into()));
    // A text property whose text looks like a number: `numish + 1` is the
    // String "421", not 43.
    properties.insert("numish".into(), Value::Str("42".into()));
    properties.insert("empty".into(), Value::Str(String::new()));
    properties.insert(
        "listish".into(),
        Value::List(vec![
            Value::Str("alpha".into()),
            Value::Str("beta".into()),
            Value::Str("alpha".into()),
        ]),
    );
    properties.insert(
        "nums".into(),
        Value::List(vec![
            Value::Number(3.0),
            Value::Number(1.0),
            Value::Number(2.0),
        ]),
    );
    properties.insert(
        "linkish".into(),
        Value::Link(LinkValue {
            path: "Pumice".into(),
            display: None,
        }),
    );

    let file = FileData {
        name: "Silica".into(),
        basename: "Silica".into(),
        fullname: "Silica.md".into(),
        path: "Silica.md".into(),
        folder: "/".into(),
        ext: "md".into(),
        size: 1024.0,
        tags: vec!["#rock".into()],
        properties,
        ..FileData::default()
    };
    SimpleContext::new().with_file(file)
}

#[track_caller]
fn shows(source: &str, expected: &str) {
    let ctx = row();
    match eval_str(source, &ctx) {
        Ok(v) => assert_eq!(v.to_string(), expected, "{source}"),
        Err(e) => panic!("{source}: refused with {e}"),
    }
}

#[track_caller]
fn refuses(source: &str, expected: &str) {
    let ctx = row();
    match eval_str(source, &ctx) {
        Ok(v) => panic!("{source}: answered {v} rather than refusing"),
        Err(e) => assert_eq!(e.to_string(), expected, "{source}"),
    }
}

#[track_caller]
fn is_null(source: &str) {
    let ctx = row();
    match eval_str(source, &ctx) {
        Ok(v) => assert!(v.is_null(), "{source}: answered {v}, not null"),
        Err(e) => panic!("{source}: refused with {e}"),
    }
}

/// "3" > 2 is true, 2 > "3" false, "3" == 3 true, true == 1 true, "abc" > 1
/// false, and two strings compare lexicographically.
#[test]
fn comparison_coerces() {
    shows("\"3\" > 2", "true");
    shows("2 > \"3\"", "false");
    shows("\"3\" == 3", "true");
    shows("true == 1", "true");
    shows("\"abc\" > 1", "false");
    shows("\"10\" > \"9\"", "false");
    shows("10 > 9", "true");
    shows("numish > 41", "true");
}

/// Arithmetic is typed and refuses, and `+` with a String on either side
/// concatenates.
#[test]
fn arithmetic_does_not_coerce() {
    refuses("1 + true", "Invalid operator between Number and Boolean");
    refuses("true + 1", "Invalid operator between Boolean and Number");
    shows("1 + \"2\"", "12");
    shows("\"a\" + 1", "a1");
    shows("numish + 1", "421");
    shows("1 + 2 * 3", "7");
}

/// Numbers are IEEE doubles printed as JavaScript prints them.
#[test]
fn numbers_print_as_javascript_does() {
    shows("0.1 + 0.2", "0.30000000000000004");
    shows("1 / 0", "Infinity");
    shows("0 / 0", "NaN");
    shows("1 % 0", "NaN");
    shows("-1 + 2", "1");
}

/// A missing property is null, and null answers every call and every access
/// with null. Only three names differ.
#[test]
fn null_absorbs() {
    is_null("nosuchprop");
    is_null("nosuchprop + 1");
    is_null("nosuchprop.length");
    is_null("nosuchprop.toFixed(2)");
    is_null("nosuchprop.nosuch.deeper");
    shows("nosuchprop.isEmpty()", "true");
    shows("nosuchprop.isTruthy()", "false");
    shows("nosuchprop.toString()", "null");
}

/// Comparison against null is three-valued: null, not false.
#[test]
fn null_comparison_is_three_valued() {
    is_null("nosuchprop > 1");
    shows("nosuchprop == null", "true");
    shows("nosuchprop == \"\"", "false");
    shows("nosuchprop == 0", "false");
    // An empty string is not null.
    shows("empty == null", "false");
    shows("empty.length", "0");
}

/// `&&` and `||` coerce through truthiness and answer a Boolean.
#[test]
fn boolean_operators_coerce_through_truthiness() {
    shows("nosuchprop && 1", "false");
    shows("nosuchprop || 1", "true");
    shows("true && false", "false");
    shows("!nosuchprop", "true");
}

/// Every *name* is case-insensitive; values are not.
#[test]
fn names_are_case_insensitive() {
    shows("file.name", "Silica");
    shows("file.NAME", "Silica");
    shows("file.Name", "Silica");
    shows("DENSITY", "2.4");
    shows("note.DENSITY", "2.4");
    shows("file.properties.STATUS", "todo");
}

/// A bare ident is a frontmatter property, `note` is an Object of them, and
/// `file` is the row's File.
#[test]
fn the_namespaces() {
    shows("density", "2.4");
    shows("note.density", "2.4");
    shows("file.path", "Silica.md");
    shows("nosuchprop", "null");
}

/// **`this` is the base file, not the row.** With no base file in the
/// context it is null, and `this.status` is null even when the row has one.
#[test]
fn this_is_not_the_row() {
    is_null("this.status");
    is_null("this.density");
}

/// A `formula.<name>` is the base's own statement, evaluated in this row.
#[test]
fn formulas_are_a_namespace() {
    let ctx = row().with_formula("ppu", "density * 2");
    assert_eq!(eval_str("formula.ppu", &ctx).unwrap().to_string(), "4.8");
    assert_eq!(
        eval_str("formula.PPU", &ctx).unwrap().to_string(),
        "4.8",
        "a formula name is case-insensitive like every other name"
    );
    assert_eq!(
        eval_str("formula.nosuch", &ctx).unwrap_err().to_string(),
        "Cannot find \"nosuch\" on type Formulas"
    );
}

/// A formula that refers to itself must not hang the view.
#[test]
fn a_formula_cycle_terminates() {
    let ctx = row().with_formula("loop", "formula.loop + 1");
    let _ = eval_str("formula.loop", &ctx);
}

/// A statement that does not parse is kept as an `invalid` node and renders an
/// empty cell — it is not an error.
#[test]
fn an_unparseable_statement_is_null() {
    is_null("foo((((");
    is_null("a ? b : c");
}

/// A filter that fails, by either road, silently empties the view.
#[test]
fn a_failed_filter_matches_nothing() {
    let ctx = row();
    assert!(matches_filter("density > 1", &ctx));
    assert!(!matches_filter("density > 10", &ctx));
    assert!(!matches_filter("foo((((", &ctx), "a parse failure is false");
    assert!(
        !matches_filter("1 + true", &ctx),
        "an eval failure is false"
    );
    assert!(
        !matches_filter("nosuchprop > 1", &ctx),
        "a null answer is not truthy"
    );
}

/// Array literals and indexing.
#[test]
fn arrays_and_indexing() {
    shows("[1, 2][0]", "1");
    shows("[1, 2]", "1, 2");
    is_null("[1, 2][9]");
    is_null("[1, 2][-1]");
    shows("nums[0]", "3");
}

/// A List renders joined with ", " and a Link as a wikilink.
#[test]
fn rendering() {
    shows("listish", "alpha, beta, alpha");
    shows("linkish.toString()", "[[Pumice]]");
    shows("true", "true");
    shows("null", "null");
}
