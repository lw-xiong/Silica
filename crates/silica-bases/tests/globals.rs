//! The sixteen globals, against the observed run.
//!
//! Every case here is lifted from
//! `tools/obsidian-probe/results/2026-10-25-bases-expr.json` (Obsidian 1.14.5,
//! 2026-10-25) unless its row says otherwise: `e09-globals` for the values,
//! `e15-globals-table` for every global's tries and every refusal,
//! `e07-durations` for `date()` and `duration()`, `e18-completions` for the
//! set itself.
//!
//! The clock is pinned to the moment of the run (`now()` → `2026-10-25T15:28:50`)
//! and `random()` to the number it answered, so nothing here is a clock race.

use chrono::NaiveDate;
use indexmap::IndexMap;
use silica_bases::context::SimpleContext;
use silica_bases::eval::eval_str;
use silica_bases::value::{DateValue, FileData, LinkValue, Value};

fn ctx() -> SimpleContext {
    let when = DateValue {
        at: NaiveDate::from_ymd_opt(2026, 1, 2)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap(),
        has_time: false,
    };
    let mut properties: IndexMap<String, Value> = IndexMap::new();
    properties.insert("density".into(), Value::Number(1.21));
    properties.insert("when".into(), Value::Date(when));
    properties.insert("text".into(), Value::str("Frothed basalt"));
    properties.insert(
        "linkish".into(),
        Value::Link(LinkValue {
            path: "Pumice".into(),
            display: None,
        }),
    );
    properties.insert(
        "nums".into(),
        Value::List(vec![
            Value::Number(3.0),
            Value::Number(1.0),
            Value::Number(2.0),
        ]),
    );

    let silica = FileData {
        name: "Silica.md".into(),
        basename: "Silica".into(),
        fullname: "Silica.md".into(),
        path: "Silica.md".into(),
        folder: "/".into(),
        ext: "md".into(),
        properties,
        ..FileData::default()
    };
    let pumice = FileData {
        name: "Pumice.md".into(),
        basename: "Pumice".into(),
        fullname: "Pumice.md".into(),
        path: "Pumice.md".into(),
        folder: "/".into(),
        ext: "md".into(),
        ..FileData::default()
    };

    let mut ctx = SimpleContext::new()
        .with_file(silica.clone())
        .with_vault_file(silica)
        .with_vault_file(pumice)
        .with_now(
            NaiveDate::from_ymd_opt(2026, 10, 25)
                .unwrap()
                .and_hms_opt(15, 28, 50)
                .unwrap(),
        );
    ctx.random = 0.11417541285520261;
    ctx
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

/// `e09-globals` and `e15-globals-table`: the values the sixteen answer.
#[test]
fn global_values() {
    for (expr, want) in [
        ("now()", "2026-10-25T15:28:50"),
        ("today()", "2026-10-25"),
        ("number(\"42\")", "42"),
        ("list(\"a\")", "a"),
        ("list(/zz/)", "/zz/"),
        ("if(true, 1, 2)", "1"),
        ("if(true, 1)", "1"),
        ("if(/zz/, /zz/)", "/zz/"),
        ("icon(\"rock\")", "rock"),
        ("image(\"pic.png\")", "pic.png"),
        ("file(\"Silica.md\")", "Silica.md"),
        ("min(1, 2)", "1"),
        ("max(1, 2)", "2"),
        ("min()", "null"),
        ("max()", "null"),
        ("random()", "0.11417541285520261"),
    ] {
        renders(expr, want);
    }
}

/// `e09-globals` and `e15-globals-table.__case`: the name is matched
/// case-insensitively.
#[test]
fn global_names_are_case_insensitive() {
    for expr in ["now()", "NOW()", "Now()", "nOw()"] {
        renders(expr, "2026-10-25T15:28:50");
    }
    renders("ToDay()", "2026-10-25");
    renders("TODAY()", "2026-10-25");
}

/// `e09-globals`: a name outside the sixteen fails with no type suffix, and
/// the refusal quotes the name as it was written.
#[test]
fn a_name_outside_the_set() {
    for name in [
        "string",
        "bool",
        "boolean",
        "choice",
        "ifs",
        "switch",
        "coalesce",
        "sum",
        "average",
        "avg",
        "mean",
        "median",
        "round",
        "floor",
        "ceil",
        "abs",
        "sqrt",
        "pow",
        "log",
        "exp",
        "trunc",
        "sign",
        "regexp",
        "regex",
        "object",
        "type",
        "empty",
        "notEmpty",
        "isEmpty",
        "concat",
        "join",
        "contains",
        "length",
        "count",
        "first",
        "last",
        "reverse",
        "sort",
        "unique",
        "filter",
        "map",
        "reduce",
        "element",
        "markdown",
        "embed",
        "tag",
        "tags",
        "prop",
        "property",
        "note",
        "formula",
        "taskList",
        "progress",
        "checkbox",
        "color",
        "rgb",
        "parseInt",
        "parseFloat",
        "toNumber",
        "toString",
        "asDate",
        "asNumber",
        "asString",
        "asLink",
        "asFile",
        "dateDiff",
        "dateModify",
        "nowUtc",
        "utc",
        "self",
        "nosuchfunction",
        "NoSuchFunction",
    ] {
        refuses(
            &format!("{name}(1)"),
            &format!("Cannot find function \"{name}\""),
        );
    }
}

/// `e15-globals-table`: the minimum arity, refused before anything else.
#[test]
fn not_enough_arguments() {
    for name in [
        "date", "duration", "number", "list", "link", "file", "icon", "image", "html",
    ] {
        refuses(
            &format!("{name}()"),
            &format!("Cannot call function \"{name}\", not enough arguments."),
        );
    }
    refuses("if()", "Cannot call function \"if\", not enough arguments.");
    refuses(
        "if(/zz/)",
        "Cannot call function \"if\", not enough arguments.",
    );
}

/// `e15-globals-table`: a declared parameter's type is checked **before** the
/// maximum arity, so four regexps are still a type error.
#[test]
fn a_parameter_of_the_wrong_type() {
    for name in ["date", "duration", "file", "icon", "image", "html"] {
        for args in [
            "/zz/",
            "/zz/, /zz/",
            "/zz/, /zz/, /zz/",
            "/zz/, /zz/, /zz/, /zz/",
        ] {
            refuses(
                &format!("{name}({args})"),
                &format!(
                    "Type error in \"{name}\", parameter \"input\". Expected String, given RegExp."
                ),
            );
        }
    }
    // `link`'s first parameter is named `self`, not `path`.
    for args in [
        "/zz/",
        "/zz/, /zz/",
        "/zz/, /zz/, /zz/",
        "/zz/, /zz/, /zz/, /zz/",
    ] {
        refuses(
            &format!("link({args})"),
            "Type error in \"link\", parameter \"self\". Expected String, given RegExp.",
        );
    }
    refuses(
        "duration(1, \"week\")",
        "Type error in \"duration\", parameter \"input\". Expected String, given Number.",
    );
}

/// `e15-globals-table`: `number` and `list` have an untyped parameter, so the
/// maximum arity is what refuses them — `number(/zz/, /zz/)` is *too many
/// arguments* although `number(/zz/)` is the conversion refusal.
#[test]
fn too_many_arguments() {
    refuses(
        "list(1, 2)",
        "Cannot call function \"list\", too many arguments.",
    );
    for args in ["/zz/, /zz/", "/zz/, /zz/, /zz/", "/zz/, /zz/, /zz/, /zz/"] {
        refuses(
            &format!("number({args})"),
            "Cannot call function \"number\", too many arguments.",
        );
    }
}

/// `e15-globals-table`: the conversion refusal, verbatim.
#[test]
fn number_refuses_by_type() {
    refuses("number(/zz/)", "Unable to convert \"RegExp\" to a number.");
}

/// `e09-globals`: `link()` could not be serialized, so what is pinned down is
/// the link's own data (`Pumice`) and its type tag. `Link.toString()` →
/// `[[Pumice]]` is `e08-methods`, on a link read from frontmatter.
#[test]
fn link_builds_a_link() {
    assert_eq!(
        eval_str("link(\"Pumice\")", &ctx()).expect("a value"),
        Value::Link(LinkValue {
            path: "Pumice".into(),
            display: None,
        })
    );
    assert_eq!(
        eval_str("link(\"Pumice\", \"alias\")", &ctx()).expect("a value"),
        Value::Link(LinkValue {
            path: "Pumice".into(),
            display: Some("alias".into()),
        })
    );
    renders("link(\"Pumice\").toString()", "[[Pumice]]");
    renders("link(\"Pumice\", \"alias\").toString()", "[[Pumice|alias]]");
}

/// `e09-globals`: `file()` resolves a path against the vault. The probe's
/// `file("Silica.md")` answered a File tagged `lucide-file` whose string is
/// the path.
#[test]
fn file_resolves_a_path() {
    renders("file(\"Silica.md\")", "Silica.md");
    renders("file(\"Pumice.md\").basename", "Pumice");
    // "Attempt to locate": a path that resolves to nothing is null. `unprobed`.
    renders("file(\"NoSuchNote.md\")", "null");
}

/// `e07-durations`: `date(when)` on a Date property answers that date, so the
/// parameter takes a date object as well as a String although the refusal
/// names String.
#[test]
fn date_accepts_a_date() {
    renders("date(when)", "2026-01-02");
}

/// `if()` must not evaluate the branch it does not take. `unprobed` — the run
/// never called it with a failing branch — but it is the only reading under
/// which a formula can guard a refusal.
#[test]
fn if_is_lazy() {
    renders("if(true, 1, nosuchfunction())", "1");
    renders("if(false, nosuchfunction(), 2)", "2");
    // A false condition with no third argument is null. `unprobed`.
    renders("if(false, 1)", "null");
    // The condition is coerced through truthiness, not required to be Boolean.
    renders("if(\"\", 1, 2)", "2");
    renders("if(density, 1, 2)", "1");
    renders("if(nosuchprop, 1, 2)", "2");
}

/// Beyond the probe, and marked `unprobed` in `funcs/globals.rs`: the run
/// never called `escapeHTML`, never passed a null to a global, and never
/// asked `min` for anything but two numbers.
#[test]
fn unprobed_readings() {
    renders(
        "escapeHTML(\"<a href='x'>&</a>\")",
        "&lt;a href=&#39;x&#39;&gt;&amp;&lt;/a&gt;",
    );
    refuses(
        "escapeHTML()",
        "Cannot call function \"escapeHTML\", not enough arguments.",
    );
    // Null absorbs, as it does for a method call and for arithmetic.
    for expr in [
        "number(nosuchprop)",
        "list(nosuchprop)",
        "link(nosuchprop)",
        "file(nosuchprop)",
        "icon(nosuchprop)",
        "image(nosuchprop)",
        "html(nosuchprop)",
        "escapeHTML(nosuchprop)",
        "date(nosuchprop)",
        "duration(nosuchprop)",
    ] {
        renders(expr, "null");
    }
    // `list` returns a list unchanged.
    renders("list(nums)", "3, 1, 2");
    // `number` converts by type: a Boolean is 1 or 0, a string that is not a
    // number is NaN, and a type with no number in it refuses by name.
    renders("number(true)", "1");
    renders("number(false)", "0");
    renders("number(12.5)", "12.5");
    renders("number(\"abc\")", "NaN");
    refuses("number(nums)", "Unable to convert \"List\" to a number.");
    // `min`/`max` coerce the way a comparison does and pass over the rest.
    renders("min(3, 1, 2)", "1");
    renders("max(3, 1, 2)", "3");
    renders("min(\"3\", 2)", "2");
    renders("min(text)", "null");
    renders("max(1)", "1");
    // `now()`, `today()` and `random()` take no arguments.
    refuses(
        "now(1)",
        "Cannot call function \"now\", too many arguments.",
    );
    refuses(
        "random(1)",
        "Cannot call function \"random\", too many arguments.",
    );
}

/// These four are `super::time`'s to answer, and they fail until
/// `funcs/time.rs` lands its parser: `date()` and `duration()` call
/// `parse_date` and `parse_duration`, which are stubs that answer `None`, and
/// a Duration prints through `humanize`.
mod needs_time {
    use super::{ctx, renders};
    use silica_bases::eval::eval_str;

    /// `e07-durations` and `e09-globals`.
    #[test]
    fn date_parses_a_string() {
        renders("date(\"2026-01-02\")", "2026-01-02");
        renders("date(\"2026-01-02T03:04:05\")", "2026-01-02T03:04:05");
    }

    /// `e07-durations`: a string that is not a date is null, not a refusal.
    /// This one passes against the stub as well.
    #[test]
    fn date_of_nonsense_is_null() {
        renders("date(\"not a date\")", "null");
    }

    /// `e09-globals`: `duration("1d")` prints humanized, as "a day".
    #[test]
    fn duration_parses_a_string() {
        renders("duration(\"1d\")", "a day");
        renders("duration(\"1 week\")", "7 days");
    }

    /// `e07-durations`: a compound duration is null — Obsidian supports one
    /// unit at a time.
    #[test]
    fn a_compound_duration_is_null() {
        renders("duration(\"1y 2M 3d\")", "null");
    }

    /// `today()` is a Date and `now()` a DateTime, both built by
    /// `time::now_of` from the host's clock.
    #[test]
    fn now_and_today_are_the_hosts_clock() {
        let now = eval_str("now()", &ctx()).expect("a value");
        let today = eval_str("today()", &ctx()).expect("a value");
        assert_eq!(now.ty().as_str(), "Date");
        assert_eq!(now.ty().icon(), "lucide-clock");
        assert_eq!(today.ty().icon(), "lucide-calendar");
    }
}
