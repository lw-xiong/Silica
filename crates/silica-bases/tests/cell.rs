//! What one cell renders as, and what text it shows.
//!
//! Every case is lifted from the CDP run of 2026-10-25
//! (`tools/obsidian-probe/results/2026-10-25-bases-expr.json`). Step
//! `e11-cells` is a table whose columns *are* the expressions, so a cell's
//! `textContent` and its `data-property-type` are read off the real view;
//! `e10-invalid` is `BadFormula.base`, the three places a statement can fail;
//! `e06-values` and `e09-globals` are the oracle's answer for one value of
//! each type. Where a reading is the oracle's `str` rather than a rendered
//! cell it is marked, because `str` is `String(value.data)` and not
//! `toString()` — see
//! [[Specification#What building the evaluator settled, 2026-10-25]].

use std::sync::Arc;

use chrono::NaiveDate;
use indexmap::IndexMap;
use silica_bases::cell::{self, CellKind};
use silica_bases::{eval_str, DateValue, DurationValue, FileData, LinkValue, Value};
use silica_bases::{SimpleContext, Type};

/// The probe's own `now()`, so `now() - "1 week"` is not a clock race. The
/// run's was 2026-10-25T15:29:15.
fn now() -> chrono::NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, 10, 25)
        .unwrap()
        .and_hms_opt(15, 29, 15)
        .unwrap()
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

/// `Silica.md`, with the frontmatter the probe seeded, opened under
/// `Main.base` — the base file `this` resolves to.
fn row() -> SimpleContext {
    let mut properties: IndexMap<String, Value> = IndexMap::new();
    properties.insert("type".into(), Value::str("rock"));
    properties.insert("status".into(), Value::str("todo"));
    properties.insert("density".into(), Value::Number(1.21));
    properties.insert("silica".into(), Value::Number(50.0));
    properties.insert("flag".into(), Value::Bool(true));
    properties.insert("off".into(), Value::Bool(false));
    properties.insert("when".into(), date(2026, 1, 2));
    properties.insert("numish".into(), Value::str("42"));
    properties.insert("empty".into(), Value::str(""));
    properties.insert(
        "listish".into(),
        Value::List(vec![
            Value::str("alpha"),
            Value::str("beta"),
            Value::str("alpha"),
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

    let silica = FileData {
        name: "Silica".into(),
        basename: "Silica".into(),
        fullname: "Silica.md".into(),
        path: "Silica.md".into(),
        folder: "/".into(),
        ext: "md".into(),
        size: 483.0,
        tags: vec!["#rock".into(), "#igneous/extrusive".into()],
        properties,
        ..FileData::default()
    };
    let pumice = FileData {
        name: "Pumice".into(),
        basename: "Pumice".into(),
        fullname: "Pumice.md".into(),
        path: "Pumice.md".into(),
        folder: "/".into(),
        ext: "md".into(),
        ..FileData::default()
    };
    // `this` is the base file, not the row.
    let main_base = FileData {
        name: "Main.base".into(),
        basename: "Main".into(),
        fullname: "Main.base".into(),
        path: "Main.base".into(),
        folder: "/".into(),
        ext: "base".into(),
        ..FileData::default()
    };

    let mut ctx = SimpleContext::new()
        .with_file(silica)
        .with_vault_file(pumice)
        .with_now(now());
    ctx.base_file = Some(Arc::new(main_base));
    ctx
}

/// A statement, through the whole path a formula column takes: parse,
/// evaluate, render.
#[track_caller]
fn cell_of(source: &str) -> silica_bases::Cell {
    cell::of(eval_str(source, &row()))
}

#[track_caller]
fn shows(source: &str, text: &str, kind: CellKind) {
    let got = cell_of(source);
    assert_eq!(got.text, text, "text of `{source}`");
    assert_eq!(got.kind, kind, "data-property-type of `{source}`");
}

/// The seven columns of `e11-cells` that were inside the table's rendered
/// window: a cell's `textContent` and its `data-property-type`, verbatim.
#[test]
fn rendered_cells_of_e11() {
    // <div data-property-type="text">abcd</div>
    shows(r#""ab" + "cd""#, "abcd", CellKind::Text);
    // <div data-property-type="number">7</div>
    shows("1 + 2 * 3", "7", CellKind::Number);
    // <div data-property-type="checkbox"><input disabled="true"
    // type="checkbox"></div> — the oracle's value is `true`, and the cell's
    // text is empty all the same.
    shows(r#""3" > 2"#, "", CellKind::Checkbox);
    // <div data-property-type="text">12</div> — a String, because `+` with a
    // String on either side concatenates.
    shows(r#"1 + "2""#, "12", CellKind::Text);
    // Three nulls, each `<div data-property-type="text"></div>`, where the
    // oracle's `str` for all three is the String `null`.
    shows("nosuchprop + 1", "", CellKind::Text);
    shows("nosuchprop.length", "", CellKind::Text);
    shows("nosuchprop.toFixed(2)", "", CellKind::Text);
}

/// The same seven, as values: a checkbox cell still carries its boolean, so a
/// view can draw the box checked.
#[test]
fn a_checkbox_cell_keeps_its_boolean() {
    let got = cell_of(r#""3" > 2"#);
    assert_eq!(got.value, Value::Bool(true));
    assert_eq!(got.text, "");
    assert_eq!(got.error, None);

    let off = cell::of(Ok(Value::Bool(false)));
    assert_eq!(off.kind, CellKind::Checkbox);
    assert_eq!(off.text, "");
}

/// `BadFormula.base` from `e10-invalid`, whose three formulas separate the two
/// ways a statement fails. Its rendered rows are
/// `["BadFilter.base", "", "Cannot find function \"nosuchfunction\"", ""]`.
#[test]
fn the_two_failures_render_differently() {
    // `broken: density +` does not parse, so it answers null and the cell is
    // empty — not an error.
    let broken = cell_of("density +");
    assert_eq!(broken.text, "");
    assert_eq!(broken.error, None);
    assert_eq!(broken.value, Value::Null);
    assert_eq!(broken.kind, CellKind::Text);

    // `alsoBroken: nosuchfunction(1)` parses and refuses, and the refusal is
    // the cell's content.
    let refused = cell_of("nosuchfunction(1)");
    assert_eq!(refused.text, r#"Cannot find function "nosuchfunction""#);
    assert_eq!(
        refused.error.as_deref(),
        Some(r#"Cannot find function "nosuchfunction""#)
    );
    assert_eq!(refused.value, Value::Null);
    assert_eq!(refused.kind, CellKind::Text);

    // `badMethod: density.nosuchmethod()` is empty on the probe's first rows,
    // which are `.base` files with no `density`: null answers every call with
    // null, with no error.
    let on_null = cell_of("nosuchprop.nosuchmethod()");
    assert_eq!(on_null.text, "");
    assert_eq!(on_null.error, None);
}

/// An unparseable statement and a genuine null are the same cell, and nothing
/// should tell them apart.
#[test]
fn a_parse_failure_is_a_null_cell() {
    assert_eq!(cell_of("density +"), cell::of(Ok(Value::Null)));
    assert_eq!(cell::of(Ok(Value::Null)).text, "");
    assert_eq!(cell::of(Ok(Value::Null)).kind, CellKind::Text);
}

/// The rest of `e11-cells`' columns. The table rendered only its first eight
/// columns, so these are the oracle's answer for the same statement put
/// through the cell's rules rather than a `textContent` — marked, because the
/// oracle's `str` is `String(value.data)`.
#[test]
fn the_columns_beyond_the_rendered_window() {
    // str "true" / "false", and a boolean is a checkbox.
    shows("nosuchprop == null", "", CellKind::Checkbox);
    shows(r#"nosuchprop == """#, "", CellKind::Checkbox);
    shows(r#"when > "2026-01-01""#, "", CellKind::Checkbox);
    shows("flag && true", "", CellKind::Checkbox);
    // Two refusals, each the cell's content.
    shows(
        r#"when - "2026-01-01""#,
        "Invalid operator between Date and String",
        CellKind::Text,
    );
    shows(
        "(now() - when).days()",
        r#"Cannot find function "days" on type Duration"#,
        CellKind::Text,
    );
    // A DateTime: the run's own now() less a week, to the second.
    shows(r#"now() - "1 week""#, "2026-10-18T15:29:15", CellKind::Text);
    // Strings out of the list functions.
    shows(r#"listish.join("|")"#, "alpha|beta|alpha", CellKind::Text);
    shows(
        r#"listish.filter(value != "beta").join("|")"#,
        "alpha|alpha",
        CellKind::Text,
    );
    shows(r#"nums.map(value * 2).join("|")"#, "6|2|4", CellKind::Text);
    // `this` is the base file, not the row.
    shows("this.file.name", "Main.base", CellKind::Text);
    shows(
        r#"if(density > 1, "heavy", "light")"#,
        "heavy",
        CellKind::Text,
    );
    // Numbers as JavaScript prints them.
    shows("1 / 0", "Infinity", CellKind::Number);
    shows("0 / 0", "NaN", CellKind::Number);
    // A text property whose text looks like a number concatenates.
    shows("numish + 1", "421", CellKind::Text);
    shows("linkish.asFile().name", "Pumice", CellKind::Text);
}

/// Where a cell's text is **not** `toString()`. Both readings are the
/// oracle's `str` from `e06-values`, which is `String(value.data)`; the
/// `toString()` beside each is what [`Value`]'s `Display` answers.
#[test]
fn a_list_joins_with_a_bare_comma() {
    // `listish` → str "alpha,beta,alpha"; toString() is "alpha, beta, alpha".
    let listish = Value::List(vec![
        Value::str("alpha"),
        Value::str("beta"),
        Value::str("alpha"),
    ]);
    assert_eq!(listish.to_string(), "alpha, beta, alpha");
    assert_eq!(cell::text_of(&listish), "alpha,beta,alpha");
    assert_eq!(cell::of(Ok(listish)).kind, CellKind::Text);

    // `nums` → str "3,1,2".
    assert_eq!(
        cell::text_of(&Value::List(vec![
            Value::Number(3.0),
            Value::Number(1.0),
            Value::Number(2.0),
        ])),
        "3,1,2"
    );

    // `file.tags` → str "#rock,#igneous/extrusive", with the `#` kept.
    let tags = Value::Tags(vec!["#rock".into(), "#igneous/extrusive".into()]);
    assert_eq!(cell::text_of(&tags), "#rock,#igneous/extrusive");
    assert_eq!(tags.ty().icon(), "lucide-tags");

    // `file.links` → str "[[Pumice|Pumice]],[[Pumice|Pumice]],[[Basalt|Basalt]]":
    // the join is a bare comma, but each *element* is still its own
    // `toString()`, brackets and all.
    let links = Value::List(vec![
        Value::Link(LinkValue {
            path: "Pumice".into(),
            display: Some("Pumice".into()),
        }),
        Value::Link(LinkValue {
            path: "Basalt".into(),
            display: Some("Basalt".into()),
        }),
    ]);
    assert_eq!(cell::text_of(&links), "[[Pumice|Pumice]],[[Basalt|Basalt]]");

    // An empty List is an empty cell: `file.embeds` → str "".
    assert_eq!(cell::text_of(&Value::List(vec![])), "");
}

#[test]
fn a_link_drops_its_wikilink_syntax() {
    // `linkish` → str "Pumice"; toString() is "[[Pumice]]".
    let link = Value::Link(LinkValue {
        path: "Pumice".into(),
        display: None,
    });
    assert_eq!(link.to_string(), "[[Pumice]]");
    assert_eq!(cell::text_of(&link), "Pumice");
    assert_eq!(cell::of(Ok(link)).kind, CellKind::Text);
}

/// Every other type's cell text, which is its `toString()`. The readings are
/// the oracle's `str` for a value of each type, and for these the two agree
/// because the value carries no `data` of its own.
#[test]
fn the_types_whose_cell_is_their_to_string() {
    // `when` → "2026-01-02"; `stamp` → "2026-01-02T03:04:05".
    assert_eq!(cell::text_of(&date(2026, 1, 2)), "2026-01-02");
    let stamp = Value::Date(DateValue {
        at: NaiveDate::from_ymd_opt(2026, 1, 2)
            .unwrap()
            .and_hms_opt(3, 4, 5)
            .unwrap(),
        has_time: true,
    });
    assert_eq!(cell::text_of(&stamp), "2026-01-02T03:04:05");
    assert_eq!(cell::of(Ok(stamp)).kind, CellKind::Text);

    // `duration("1d")` → "a day": humanized, not a count.
    let a_day = Value::Duration(DurationValue {
        days: 1.0,
        ..DurationValue::default()
    });
    assert_eq!(cell::text_of(&a_day), "a day");
    assert_eq!(cell::of(Ok(a_day)).kind, CellKind::Text);

    // `file("Silica.md")` → "Silica.md", the path.
    let file = Value::File(Arc::new(FileData {
        name: "Silica".into(),
        path: "Silica.md".into(),
        ..FileData::default()
    }));
    assert_eq!(cell::text_of(&file), "Silica.md");

    // A number prints as JavaScript prints one.
    assert_eq!(
        cell::text_of(&Value::Number(0.1 + 0.2)),
        "0.30000000000000004"
    );
    assert_eq!(cell::text_of(&Value::Number(f64::INFINITY)), "Infinity");
    assert_eq!(cell::text_of(&Value::Number(f64::NAN)), "NaN");
    assert_eq!(cell::of(Ok(Value::Number(483.0))).kind, CellKind::Number);

    // An empty string is a cell with no text, and is not a null: its type tag
    // is still `text`.
    let empty = cell::of(Ok(Value::str("")));
    assert_eq!(empty.text, "");
    assert_eq!(empty.kind, CellKind::Text);
    assert_eq!(empty.value.ty(), Type::String);
}

/// `kind_of` on its own: the tag is the value's type, and only a boolean and a
/// number leave `text`.
#[test]
fn kind_of_every_type() {
    assert_eq!(kind_of_v(Value::Bool(true)), CellKind::Checkbox);
    assert_eq!(kind_of_v(Value::Number(1.0)), CellKind::Number);
    assert_eq!(kind_of_v(Value::str("x")), CellKind::Text);
    assert_eq!(kind_of_v(Value::Null), CellKind::Text);
    assert_eq!(kind_of_v(Value::List(vec![])), CellKind::Text);
    assert_eq!(kind_of_v(date(2026, 1, 2)), CellKind::Text);
    assert_eq!(
        kind_of_v(Value::Duration(DurationValue::default())),
        CellKind::Text
    );
    assert_eq!(
        kind_of_v(Value::Link(LinkValue {
            path: "Pumice".into(),
            display: None
        })),
        CellKind::Text
    );
}

fn kind_of_v(value: Value) -> CellKind {
    let kind = cell::kind_of(&value);
    // `of` tags a cell with exactly what `kind_of` answers.
    assert_eq!(cell::of(Ok(value)).kind, kind);
    kind
}
