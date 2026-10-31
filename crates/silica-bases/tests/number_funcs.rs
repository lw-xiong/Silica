//! Number and Boolean, against the observed run.
//!
//! Every case here is lifted from
//! `tools/obsidian-probe/results/2026-10-25-bases-expr.json` (Obsidian 1.14.5,
//! 2026-10-25) unless its row says otherwise: `e14-function-table` →
//! `byType.Number` / `byType.Boolean` for the table and its refusals,
//! `e06-values` for the values, `e08-methods` for the names that are *not*
//! functions.
//!
//! The seeds are the probe's own note: `density: 1.21`, `negative: -4`,
//! `silica: 50`, and the Number seed of the function table is `12.345`.

use indexmap::IndexMap;
use silica_bases::context::SimpleContext;
use silica_bases::eval::eval_str;
use silica_bases::value::{FileData, Value};

fn ctx() -> SimpleContext {
    let mut properties: IndexMap<String, Value> = IndexMap::new();
    properties.insert("density".into(), Value::Number(1.21));
    properties.insert("silica".into(), Value::Number(50.0));
    properties.insert("negative".into(), Value::Number(-4.0));
    properties.insert("zeroish".into(), Value::Number(0.0));
    properties.insert("flag".into(), Value::Bool(true));
    properties.insert("off".into(), Value::Bool(false));
    SimpleContext::new().with_file(FileData {
        name: "Silica.md".into(),
        basename: "Silica".into(),
        path: "Silica.md".into(),
        properties,
        ..FileData::default()
    })
}

#[track_caller]
fn renders(expr: &str, want: &str) {
    match eval_str(expr, &ctx()) {
        Ok(v) => assert_eq!(v.to_string(), want, "{expr}"),
        Err(e) => panic!("{expr}: expected {want:?}, got the refusal {:?}", e.message),
    }
}

#[track_caller]
fn refuses(expr: &str, want: &str) {
    match eval_str(expr, &ctx()) {
        Ok(v) => panic!(
            "{expr}: expected the refusal {want:?}, got {:?}",
            v.to_string()
        ),
        Err(e) => assert_eq!(e.message, want, "{expr}"),
    }
}

/// `e14-function-table`, `byType.Number`: the whole table on the seed 12.345.
#[test]
fn number_function_table() {
    for (expr, want) in [
        ("12.345.abs()", "12.345"),
        ("12.345.ceil()", "13"),
        ("12.345.floor()", "12"),
        ("12.345.isEmpty()", "false"),
        ("12.345.round()", "12"),
        // Every type has these three, and the dispatcher owns them; they are
        // here because the table observed them on a Number.
        ("12.345.isTruthy()", "true"),
        ("12.345.toString()", "12.345"),
    ] {
        renders(expr, want);
    }
}

/// `e06-values`: the Number cases evaluated against the probe's own note.
#[test]
fn number_values() {
    renders("density.toFixed(3)", "1.210");
    renders("negative.abs()", "4");
    // Null absorbs: the dispatcher answers before this module is reached.
    renders("nosuchprop.toFixed(2)", "null");
}

/// `toFixed` returns a **String**: `density.toFixed(3)` is
/// `{icon: "lucide-text", data: "1.210"}`, tagged as text rather than as a
/// number.
#[test]
fn to_fixed_returns_a_string() {
    let v = eval_str("density.toFixed(3)", &ctx()).expect("a value");
    assert_eq!(v, Value::Str("1.210".into()));
}

/// `e14-function-table`: minimum arity, then the parameter's type, then the
/// maximum — three regexps are still a type error, not *too many arguments*.
#[test]
fn to_fixed_refusals() {
    refuses(
        "12.345.toFixed()",
        "Cannot call function \"toFixed\", not enough arguments.",
    );
    for expr in [
        "12.345.toFixed(/zz/)",
        "12.345.toFixed(/zz/, /zz/)",
        "12.345.toFixed(/zz/, /zz/, /zz/)",
    ] {
        refuses(
            expr,
            "Type error in \"toFixed\", parameter \"precision\". Expected Number, given RegExp.",
        );
    }
}

/// `e08-methods`, `byType.number`: the JavaScript prototype's own name is not
/// a function of the language.
#[test]
fn number_is_not_the_prototype() {
    refuses(
        "12.345.renderTo()",
        "Cannot find function \"renderTo\" on type Number",
    );
    refuses(
        "12.345.nosuchmethod()",
        "Cannot find function \"nosuchmethod\" on type Number",
    );
}

/// `e14-function-table`, `byType.Boolean`: nothing but the three every type
/// has. `e08-methods` observed `true.isTruthy()` → true and
/// `true.renderTo()` refused.
#[test]
fn boolean_has_no_functions_of_its_own() {
    renders("true.isTruthy()", "true");
    renders("true.toString()", "true");
    refuses(
        "true.renderTo()",
        "Cannot find function \"renderTo\" on type Boolean",
    );
    refuses("flag.abs()", "Cannot find function \"abs\" on type Boolean");
    refuses(
        "flag.isEmpty()",
        "Cannot find function \"isEmpty\" on type Boolean",
    );
}

/// Names are case-insensitive, values are not: `e17-case-and-namespaces`
/// observed `12.3.TOFIXED(1)`.
#[test]
fn names_are_case_insensitive() {
    renders("12.3.toFixed(1)", "12.3");
    renders("12.3.TOFIXED(1)", "12.3");
    renders("negative.ABS()", "4");
}

/// Beyond the probe. `round()` was only ever seen on 12.345 and `toFixed` only
/// on 1.21, so the tie rule and the padding below are this crate's reading of
/// the one observation (a trailing zero is written, so the formatting is
/// fixed-point with half rounding away from the cut) and are marked `unprobed`
/// in `funcs/number.rs`.
#[test]
fn unprobed_rounding_edges() {
    renders("(0 - 2.5).round()", "-2");
    renders("2.5.round()", "3");
    renders("2.5.toFixed(0)", "3");
    renders("1.005.toFixed(2)", "1.00");
    renders("0.125.toFixed(2)", "0.13");
    renders("1.21.toFixed(0)", "1");
    renders("(0 - 1.21).toFixed(3)", "-1.210");
    // Infinity and NaN keep the way they print.
    renders("(1 / 0).toFixed(2)", "Infinity");
    renders("(0 / 0).toFixed(2)", "NaN");
    renders("(0 / 0).abs()", "NaN");
    renders("(1 / 0).floor()", "Infinity");
}
