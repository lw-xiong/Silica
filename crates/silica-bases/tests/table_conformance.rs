//! What a Bases table shows, held to what Obsidian 1.14.5 was observed to do.
//!
//! Every test here names its evidence, and the evidence is the raw probe run
//! rather than the reading of it: `tools/obsidian-probe/results/`
//! `2026-10-25-bases.json` (the format and write-back run, step names `sNN-*`)
//! and `2026-10-25-bases-expr.json` (the expression run, `eNN-*`, whose
//! `e11-cells` step is whole cells rendered in a real table and whose
//! `e10-invalid` step is the three places a statement can fail). Where the raw
//! run and
//! [[Specification#Observed Bases write behavior, Obsidian 1.14.5 (installer build 1.13.7)]]
//! disagree the run wins, and the test says so.
//!
//! The fixtures are the probe's own. `BadFilter.base`, `BadFormula.base` and
//! `Cells.base` are quoted byte for byte from `tools/obsidian-probe/
//! bases-expr.mjs`, and the vault they ran against is [`probe_vault`]: four
//! notes and the five `.base` files, which are rows of the table too. That is
//! why the run says "9 results".
//!
//! Three things in this file are *not* observed and are marked where they are
//! asserted: a view's own `filters` failing (only a base's was broken in the
//! run, but the key and the toolbar's "This view" section are both observed),
//! `limit` actually cutting rows (the one limited view had `limit: 10` over
//! four results), and a `displayName` under a qualified `note.*` key (only
//! `formula.ppu` and a bare key were ever rendered). Nothing else here is an
//! inference.

use std::path::PathBuf;
use std::sync::Arc;

use chrono::NaiveDate;
use indexmap::IndexMap;
use silica_bases::table::{self, Env};
use silica_bases::{CellKind, DateValue, FileData, LinkValue, Table, Value};
use silica_formats::base::Base;

// ---------------------------------------------------------------------------
// The in-memory vault
// ---------------------------------------------------------------------------

fn props(pairs: Vec<(&str, Value)>) -> IndexMap<String, Value> {
    pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

fn num(n: f64) -> Value {
    Value::Number(n)
}

fn text(s: &str) -> Value {
    Value::Str(s.to_string())
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

/// One row's file. `file.name` is the file name with a `.md` dropped and any
/// other extension kept — observed: `Silica`, but `Main.base`.
fn file(path: &str, tags: &[&str], pairs: Vec<(&str, Value)>) -> Arc<FileData> {
    let (folder, full) = match path.rsplit_once('/') {
        Some((folder, full)) => (folder.to_string(), full.to_string()),
        None => (String::new(), path.to_string()),
    };
    let (stem, ext) = match full.rsplit_once('.') {
        Some((stem, ext)) => (stem.to_string(), ext.to_string()),
        None => (full.clone(), String::new()),
    };
    Arc::new(FileData {
        name: if ext == "md" {
            stem.clone()
        } else {
            full.clone()
        },
        basename: stem,
        fullname: full,
        path: path.to_string(),
        folder,
        ext,
        size: 0.0,
        ctime: None,
        mtime: None,
        tags: tags.iter().map(|t| format!("#{t}")).collect(),
        links: Vec::new(),
        embeds: Vec::new(),
        backlinks: Vec::new(),
        properties: props(pairs),
    })
}

/// The vault `e10-invalid` and `e11-cells` ran against, in the order the
/// rendered table listed it: the four notes and the five `.base` files, nine
/// rows in all ("9 results"). Seeded in `tools/obsidian-probe/bases-expr.mjs`.
fn probe_vault() -> Vec<Arc<FileData>> {
    let mut files: Vec<Arc<FileData>> = vec![
        file("BadFilter.base", &[], vec![]),
        file("BadFormula.base", &[], vec![]),
        file("BadSummary.base", &[], vec![]),
        file("Bare.md", &[], vec![]),
        file(
            "Basalt.md",
            &["rock"],
            vec![
                ("type", text("rock")),
                ("status", text("todo")),
                ("density", num(2.9)),
                ("silica", num(49.0)),
            ],
        ),
        file("Cells.base", &[], vec![]),
        file("Main.base", &[], vec![]),
        file(
            "Pumice.md",
            &["rock"],
            vec![
                ("type", text("rock")),
                ("status", text("done")),
                ("density", num(0.64)),
                ("silica", num(72.0)),
                ("count", num(10.0)),
                ("when", date(2026, 3, 4)),
            ],
        ),
    ];
    files.push(silica_note());
    files
}

/// `Silica.md`, the probe's one note with a property of every type. Its
/// frontmatter is quoted from `bases-expr.mjs`; only the properties the cell
/// tests read are filled in.
fn silica_note() -> Arc<FileData> {
    file(
        "Silica.md",
        &["rock", "igneous/extrusive"],
        vec![
            ("type", text("rock")),
            ("status", text("todo")),
            ("density", num(1.21)),
            ("silica", num(50.0)),
            ("count", num(3.0)),
            ("negative", num(-4.0)),
            ("flag", Value::Bool(true)),
            ("when", date(2026, 1, 2)),
            ("numish", text("42")),
            (
                "listish",
                Value::List(vec![text("alpha"), text("beta"), text("alpha")]),
            ),
            ("nums", Value::List(vec![num(3.0), num(1.0), num(2.0)])),
            (
                "linkish",
                Value::Link(LinkValue {
                    path: "Pumice".into(),
                    display: None,
                }),
            ),
        ],
    )
}

/// `now()`, fixed: the run answered `now() - "1 week"` with
/// `2026-10-18T15:29:15`.
fn env(files: Vec<Arc<FileData>>) -> Env {
    Env {
        files,
        base_file: None,
        now: NaiveDate::from_ymd_opt(2026, 10, 25)
            .unwrap()
            .and_hms_opt(15, 29, 15)
            .unwrap(),
    }
}

fn base(yaml: &str) -> Base {
    Base::from_yaml(yaml).unwrap_or_else(|e| panic!("the base must parse, and did not: {e}"))
}

/// Every row's `file.name`, which is the first column of every fixture here.
fn names(table: &Table) -> Vec<String> {
    table.rows.iter().map(|r| r.file.name.clone()).collect()
}

/// One column's text, down the rows.
fn column_text(table: &Table, col: usize) -> Vec<String> {
    table
        .rows
        .iter()
        .map(|r| r.cells[col].text.clone())
        .collect()
}

fn headers(table: &Table) -> Vec<String> {
    table.columns.iter().map(|c| c.display.clone()).collect()
}

// ---------------------------------------------------------------------------
// A failing filter empties the view
// ---------------------------------------------------------------------------

/// `BadFilter.base`, verbatim from `bases-expr.mjs`: its only filter is
/// `density >>> 1`, which does not parse.
const BAD_FILTER: &str = "filters:\n  and:\n    - 'density >>> 1'\nviews:\n  - type: table\n    name: BadFilter\n    order:\n      - file.name\n";

#[test]
fn a_base_filter_that_cannot_parse_empties_the_view() {
    // e10-invalid / BadFilter.base: `filtersJson` holds
    // `{"type":"invalid","parseError":"Unable to parse formula. (position 0)"}`,
    // and the view shows `rowCount: 0`, `resultCount: "0 results"`,
    // `cellText: []` — with `errorDom` `[{text: "", hidden: true}]`. No banner,
    // no notice: the view is simply empty.
    let base = base(BAD_FILTER);
    let table = table::build(&base, 0, &env(probe_vault()));

    assert!(table.rows.is_empty(), "a broken filter shows no rows");
    assert_eq!(table.unknown_kind, None, "and it is not a view-type error");
    // The header row is still drawn: the step read `headers: ["file name"]`
    // off the same empty view.
    assert_eq!(headers(&table), vec!["file name".to_string()]);
}

#[test]
fn a_base_filter_that_cannot_evaluate_empties_the_view() {
    // The same silence for the other failure. `when - "2026-01-01"` parses
    // (`parseErr: null`) and then refuses: the e11-cells oracle answers
    // `valueErr: "Invalid operator between Date and String"`. A refusal is
    // false, exactly as an unparseable statement is, so no row survives —
    // including the eight rows with no `when` at all, for which the statement
    // is null rather than an error.
    let yaml = "filters:\n  and:\n    - 'when - \"2026-01-01\"'\nviews:\n  - type: table\n    name: V\n    order:\n      - file.name\n";
    let table = table::build(&base(yaml), 0, &env(probe_vault()));

    assert!(table.rows.is_empty());
    assert_eq!(table.unknown_kind, None);
}

#[test]
fn a_views_own_filter_that_fails_empties_the_view_too() {
    // The run only ever broke a *base's* filter, so this is the one rule here
    // carried across by symmetry. Both halves of the symmetry are observed:
    // `filters` is a view's own key, written between `name` and `groupBy`
    // (s11-bytes, the view key order), and the filter panel holds two
    // sections, "All views" and "This view" (s20-panels-table). The statement
    // is `BadFilter.base`'s own.
    let yaml = "views:\n  - type: table\n    name: V\n    filters:\n      and:\n        - 'density >>> 1'\n    order:\n      - file.name\n";
    let table = table::build(&base(yaml), 0, &env(probe_vault()));

    assert!(table.rows.is_empty());
    assert_eq!(headers(&table), vec!["file name".to_string()]);
}

#[test]
fn a_view_filter_narrows_the_bases_filter() {
    // Two sections, both applied: "All views" is the base's `filters` and
    // "This view" is the view's own (s20-panels-table's filter panel). The
    // base's keeps the three rocks; the view's keeps the two whose status is
    // `todo`.
    let yaml = "filters:\n  and:\n    - file.hasTag(\"rock\")\nviews:\n  - type: table\n    name: V\n    filters:\n      and:\n        - 'status == \"todo\"'\n    order:\n      - file.name\n";
    let table = table::build(&base(yaml), 0, &env(probe_vault()));

    assert_eq!(names(&table), vec!["Basalt", "Silica"]);
}

// ---------------------------------------------------------------------------
// A formula cell: parse failure, evaluation failure, and null
// ---------------------------------------------------------------------------

/// `BadFormula.base`, verbatim from `bases-expr.mjs`. Its three formulas
/// separate the three outcomes.
const BAD_FORMULA: &str = "formulas:\n  broken: 'density +'\n  alsoBroken: 'nosuchfunction(1)'\n  badMethod: 'density.nosuchmethod()'\nviews:\n  - type: table\n    name: BadFormula\n    order:\n      - file.name\n      - formula.broken\n      - formula.alsoBroken\n      - formula.badMethod\n";

#[test]
fn a_formula_that_cannot_parse_renders_an_empty_cell() {
    // e10-invalid / BadFormula.base: `broken: "density +"` has
    // `ast: {type: "invalid", parseError: "Unable to parse formula. (position
    // 0)"}`, and every captured row's second cell is `""`. The view is *not*
    // emptied — `rowCount: 9`, "9 results" — so a broken formula costs one
    // cell, where a broken filter costs every row.
    let table = table::build(&base(BAD_FORMULA), 0, &env(probe_vault()));

    assert_eq!(table.rows.len(), 9, "9 results");
    assert_eq!(column_text(&table, 1), vec![""; 9]);
    let cell = &table.rows[0].cells[1];
    assert_eq!(
        cell.error, None,
        "an unparseable statement is null, not an error"
    );
    assert_eq!(cell.value, Value::Null);
}

#[test]
fn a_formula_whose_evaluation_fails_renders_the_refusal() {
    // The same step's third column. `nosuchfunction(1)` parses — its `ast` is
    // a `function` node — and refuses at evaluation, and every captured row
    // reads `Cannot find function "nosuchfunction"`. The refusal *is* the
    // cell's content.
    let table = table::build(&base(BAD_FORMULA), 0, &env(probe_vault()));

    let want = "Cannot find function \"nosuchfunction\"".to_string();
    assert_eq!(column_text(&table, 2), vec![want.clone(); 9]);
    assert_eq!(table.rows[0].cells[2].error, Some(want));
}

#[test]
fn a_call_on_a_null_property_is_empty_rather_than_a_refusal() {
    // The fourth column, which is the one that could be mistaken for the
    // third. `density.nosuchmethod()` would refuse on a Number, but the four
    // rows the step captured — `BadFilter.base`, `BadFormula.base`,
    // `BadSummary.base` and `Bare` — have no `density`, and every one of them
    // reads `""`. Null absorbs the call before the name is ever looked up.
    let table = table::build(&base(BAD_FORMULA), 0, &env(probe_vault()));

    assert_eq!(&column_text(&table, 3)[..4], &[""; 4]);
    assert_eq!(table.rows[0].cells[3].error, None);
}

// ---------------------------------------------------------------------------
// What a cell renders as
// ---------------------------------------------------------------------------

/// A base whose columns are the expressions themselves, as `Cells.base` is.
fn cells_base(exprs: &[(&str, &str)]) -> Base {
    let mut yaml = String::from("formulas:\n");
    for (name, statement) in exprs {
        yaml.push_str(&format!("  {name}: {}\n", serde_json_string(statement)));
    }
    yaml.push_str("views:\n  - type: table\n    name: Cells\n    order:\n      - file.name\n");
    for (name, _) in exprs {
        yaml.push_str(&format!("      - formula.{name}\n"));
    }
    base(&yaml)
}

/// A double-quoted YAML scalar, which is what `bases-expr.mjs` wrote with
/// `JSON.stringify`.
fn serde_json_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The `Silica.md` row of a cells base — the row `e11-cells` read.
fn silica_row(exprs: &[(&str, &str)]) -> Vec<silica_bases::Cell> {
    let base = cells_base(exprs);
    let table = table::build(&base, 0, &env(vec![silica_note()]));
    table.rows[0].cells.clone()
}

#[test]
fn a_boolean_renders_as_a_disabled_checkbox() {
    // e11-cells, `firstRowHtml`: the `"3" > 2` column is
    // `<div class="bases-table-cell bases-rendered-value"
    //  data-property-type="checkbox"><input disabled="true"
    //  type="checkbox"></div>`, and the cell's text is `""` — while the
    // oracle's answer for the same statement is the *true* Boolean
    // `{icon: "lucide-check-square", str: "true"}`. So a true boolean draws no
    // text at all.
    let cells = silica_row(&[("cCoerceGt", "\"3\" > 2")]);

    assert_eq!(cells[1].value, Value::Bool(true));
    assert_eq!(cells[1].kind, CellKind::Checkbox);
    assert_eq!(cells[1].text, "", "a checkbox cell has no text");
}

#[test]
fn a_cells_type_tag_is_text_number_or_checkbox() {
    // The same `firstRowHtml`, read across: `data-property-type` was `text`
    // for `"ab" + "cd"` (abcd), `number` for `1 + 2 * 3` (7), `checkbox` for
    // `"3" > 2`, `text` for `1 + "2"` — a String, because `+` with a String
    // concatenates, so the tag follows the value and not the column — and
    // `text` with empty content for all three of the null statements.
    let cells = silica_row(&[
        ("cStr", "\"ab\" + \"cd\""),
        ("cNum", "1 + 2 * 3"),
        ("cCoerceGt", "\"3\" > 2"),
        ("cCoerceAdd", "1 + \"2\""),
        ("cNullAdd", "nosuchprop + 1"),
        ("cNullLen", "nosuchprop.length"),
        ("cNullFixed", "nosuchprop.toFixed(2)"),
    ]);

    let seen: Vec<(CellKind, String)> = cells[1..]
        .iter()
        .map(|c| (c.kind, c.text.clone()))
        .collect();
    assert_eq!(
        seen,
        vec![
            (CellKind::Text, "abcd".to_string()),
            (CellKind::Number, "7".to_string()),
            (CellKind::Checkbox, String::new()),
            (CellKind::Text, "12".to_string()),
            (CellKind::Text, String::new()),
            (CellKind::Text, String::new()),
            (CellKind::Text, String::new()),
        ]
    );
}

// ---------------------------------------------------------------------------
// The view itself
// ---------------------------------------------------------------------------

#[test]
fn an_unknown_view_type_shows_a_message_instead_of_rows() {
    // s12-broken / BrokenView.base, verbatim: the file parses — `serializable`
    // holds the view with `type: "nonsense"` and the bytes survive both the
    // open and the close — and the view shows `errorDom`
    // `"Unknown view type: nonsense"` with `rowCount: 0`.
    let yaml = "views:\n  - type: nonsense\n    name: Odd\n    order:\n      - file.name\n";
    let parsed = Base::from_yaml(yaml).expect("an unknown view type is not a parse error");
    assert_eq!(parsed.views[0].kind, "nonsense");

    let table = table::build(&parsed, 0, &env(probe_vault()));
    assert_eq!(table.unknown_kind.as_deref(), Some("nonsense"));
    assert_eq!(
        table
            .unknown_kind
            .as_ref()
            .map(|k| format!("Unknown view type: {k}")),
        Some("Unknown view type: nonsense".to_string())
    );
    assert!(table.rows.is_empty());
}

#[test]
fn an_empty_base_file_is_one_default_table_view() {
    // s13-commands: `bases:new-file` created `Untitled.base` as a zero-byte
    // file (`size: 0`, `text: ""`), and the view's model reports
    // `serializable: {views: [{type: "table", name: "View"}]}` with
    // `rowCount: 22` — every note in the vault. s11-bytes says the same of
    // `Zero.base` and of `EmptyMap.base` (`{}`), both `viewName: "View"`,
    // both 22 rows. So an empty base is not an empty view: it is one default
    // table over the whole vault.
    for yaml in ["", "{}\n"] {
        let parsed = base(yaml);
        assert!(parsed.views.is_empty(), "the file itself has no views");

        let table = table::build(&parsed, 0, &env(probe_vault()));
        assert_eq!(table.unknown_kind, None, "the default view is a table");
        assert_eq!(table.rows.len(), 9, "and it shows every file");
        assert_eq!(table.groups.len(), 1, "with no grouping");
    }
}

#[test]
fn a_view_with_no_sort_keeps_the_order_it_was_given() {
    // e11-cells rendered its nine rows as BadFilter.base, BadFormula.base,
    // BadSummary.base, Bare, Basalt, Cells.base, Main.base, Pumice, Silica —
    // the vault's own order, with no `sort` in the view at all. The engine
    // must not invent one: whoever fills `Env::files` decides.
    let table = table::build(
        &base("views:\n  - type: table\n    name: V\n    order:\n      - file.name\n"),
        0,
        &env(probe_vault()),
    );

    assert_eq!(
        names(&table),
        vec![
            "BadFilter.base",
            "BadFormula.base",
            "BadSummary.base",
            "Bare",
            "Basalt",
            "Cells.base",
            "Main.base",
            "Pumice",
            "Silica",
        ]
    );
}

#[test]
fn limit_cuts_the_rows_off_the_end() {
    // **Not observed cutting.** The only limited view in the run was
    // `Main.base` with `limit: 10` over five results, so no fixture ever lost
    // a row. What is observed is the key (`limit: 10`, written as a bare
    // number between `sort` and `summaries`) and the control that writes it:
    // the results panel's "Limit number of results" number input, with
    // `min="1"` and the placeholder "e.g. 10", beside a "Show all (3)" action
    // that counts rows (s20-panels-table). Cutting the end of the view's own
    // order is the only reading of those two together.
    let yaml =
        "views:\n  - type: table\n    name: V\n    order:\n      - file.name\n    limit: 3\n";
    let table = table::build(&base(yaml), 0, &env(probe_vault()));

    assert_eq!(
        names(&table),
        vec!["BadFilter.base", "BadFormula.base", "BadSummary.base"]
    );
    assert!(table.limited, "the view knows it was cut");

    let whole =
        "views:\n  - type: table\n    name: V\n    order:\n      - file.name\n    limit: 20\n";
    let table = table::build(&base(whole), 0, &env(probe_vault()));
    assert_eq!(table.rows.len(), 9);
    assert!(!table.limited, "a limit no row reached did not cut");
}

#[test]
fn sort_follows_the_property_and_the_direction() {
    // s11-bytes: `sort:` / `  - property: status` / `    direction: DESC`,
    // and s23-header-and-results watched *Sort Z → A* prepend
    // `{property: file.name, direction: DESC}` to that list while the sort
    // panel labelled the pair "status" / "Z → A". So `ASC` is A→Z and `DESC`
    // is Z→A; an absent direction is the ascending one, which is what the
    // panel shows a fresh sort as.
    let rocks = vec![
        silica_note(),
        probe_vault()[4].clone(),
        probe_vault()[7].clone(),
    ];
    let asc = "views:\n  - type: table\n    name: V\n    order:\n      - file.name\n    sort:\n      - property: density\n        direction: ASC\n";
    let desc = "views:\n  - type: table\n    name: V\n    order:\n      - file.name\n    sort:\n      - property: density\n        direction: DESC\n";
    let bare = "views:\n  - type: table\n    name: V\n    order:\n      - file.name\n    sort:\n      - property: density\n";

    assert_eq!(
        names(&table::build(&base(asc), 0, &env(rocks.clone()))),
        vec!["Pumice", "Silica", "Basalt"]
    );
    assert_eq!(
        names(&table::build(&base(desc), 0, &env(rocks.clone()))),
        vec!["Basalt", "Silica", "Pumice"]
    );
    assert_eq!(
        names(&table::build(&base(bare), 0, &env(rocks))),
        vec!["Pumice", "Silica", "Basalt"],
        "no direction is ascending"
    );
}

#[test]
fn a_second_sort_entry_breaks_the_first_ones_ties() {
    // `sort` is a list, and s23-header-and-results left a real two-entry one
    // behind: `[{property: file.name, direction: DESC}, {property: status,
    // direction: DESC}]`. Basalt and Silica are both `todo`; `density` is
    // what separates them.
    let rocks = vec![
        silica_note(),
        probe_vault()[4].clone(),
        probe_vault()[7].clone(),
    ];
    let yaml = "views:\n  - type: table\n    name: V\n    order:\n      - file.name\n    sort:\n      - property: status\n        direction: ASC\n      - property: density\n        direction: DESC\n";

    assert_eq!(
        names(&table::build(&base(yaml), 0, &env(rocks))),
        vec!["Pumice", "Basalt", "Silica"]
    );
}

#[test]
fn group_by_splits_the_rows_and_its_direction_orders_the_groups() {
    // s11-bytes: `groupBy:` / `  property: status` / `  direction: ASC`, and
    // the group panel is the same property-plus-direction pair the sort panel
    // is, labelled "A → Z" (s20-panels-table). `Table::rows` is the groups
    // flattened, so the grouping decides the row order before `sort` does
    // inside each group.
    let rocks = vec![
        silica_note(),
        probe_vault()[4].clone(),
        probe_vault()[7].clone(),
    ];
    let asc = "views:\n  - type: table\n    name: V\n    groupBy:\n      property: status\n      direction: ASC\n    order:\n      - file.name\n    sort:\n      - property: density\n        direction: DESC\n";

    let table = table::build(&base(asc), 0, &env(rocks.clone()));
    let groups: Vec<(Option<String>, Vec<String>)> = table
        .groups
        .iter()
        .map(|g| {
            (
                g.key.clone(),
                g.rows.iter().map(|r| r.file.name.clone()).collect(),
            )
        })
        .collect();
    assert_eq!(
        groups,
        vec![
            (Some("done".to_string()), vec!["Pumice".to_string()]),
            (
                Some("todo".to_string()),
                vec!["Basalt".to_string(), "Silica".to_string()]
            ),
        ]
    );
    assert_eq!(names(&table), vec!["Pumice", "Basalt", "Silica"]);

    let desc = asc.replace("direction: ASC", "direction: DESC");
    let table = table::build(&base(&desc), 0, &env(rocks));
    assert_eq!(
        table
            .groups
            .iter()
            .map(|g| g.key.clone())
            .collect::<Vec<_>>(),
        vec![Some("todo".to_string()), Some("done".to_string())]
    );
}

#[test]
fn the_no_value_group_is_its_own_group_and_comes_first() {
    // s24-names-and-copy copied the rendered table as TSV:
    // `"status\tPrice per unit\n\t\n\t\ntodo\t0.02\ntodo\t0.06"` — two rows
    // with no status above two `todo` rows, under
    // `groupBy: {property: status, direction: ASC}` and in spite of a
    // `sort` on `status` that was `DESC`. So a row with no value for the
    // grouped property lands in a group of its own, and that group is first
    // in an ascending one.
    let rows = vec![
        probe_vault()[7].clone(), // Pumice, status done
        file("Empty.md", &["rock"], vec![("type", text("rock"))]),
        silica_note(), // status todo
    ];
    let yaml = "views:\n  - type: table\n    name: V\n    groupBy:\n      property: status\n      direction: ASC\n    order:\n      - file.name\n";

    let table = table::build(&base(yaml), 0, &env(rows));
    assert_eq!(
        table
            .groups
            .iter()
            .map(|g| g.key.clone())
            .collect::<Vec<_>>(),
        vec![None, Some("done".to_string()), Some("todo".to_string())]
    );
    assert_eq!(names(&table), vec!["Empty", "Pumice", "Silica"]);
}

// ---------------------------------------------------------------------------
// Column headers
// ---------------------------------------------------------------------------

#[test]
fn a_column_header_is_the_display_name_the_base_gives_it() {
    // s06-column-resize read the real header row off `Main.base`, whose
    // `order` is `[file.name, status, formula.ppu]` and whose `properties`
    // are `status: {displayName: Status, unknownPropKey: kept}`,
    // `formula.ppu: {displayName: Price per unit}` and
    // `file.ext: {displayName: Extension}`. The headers came back
    // `["file name", "status", "Price per unit"]`, and s24's copied TSV
    // repeats two of them in its header line.
    //
    // Two things follow, and both contradict "the id's last segment":
    //
    // - `formula.ppu`'s display name lands, so a `properties` key does name a
    //   column — when it is the *qualified* id.
    // - `file.name` is "file name", not "name".
    let yaml = "properties:\n  status:\n    displayName: Status\n    unknownPropKey: kept\n  formula.ppu:\n    displayName: Price per unit\n  file.ext:\n    displayName: Extension\nformulas:\n  ppu: '(density / silica).toFixed(2)'\nviews:\n  - type: table\n    name: My table\n    order:\n      - file.name\n      - status\n      - formula.ppu\n";
    let table = table::build(&base(yaml), 0, &env(vec![silica_note()]));

    assert_eq!(
        headers(&table),
        vec![
            "file name".to_string(),
            "status".to_string(),
            "Price per unit".to_string(),
        ]
    );
}

#[test]
fn a_bare_properties_key_does_not_name_a_bare_column() {
    // The same header row, isolated. `properties.status.displayName` is
    // `Status` and the column `status` still rendered as `status`, while
    // `formula.ppu`'s landed — so the lookup is by the qualified id
    // (`note.status`), which a hand-written bare key never matches. The
    // *writer* is the other way round and leaves a bare id bare: "a
    // hand-written bare `status` came back `status`".
    let bare = "properties:\n  status:\n    displayName: Status\nviews:\n  - type: table\n    name: V\n    order:\n      - status\n";
    let table = table::build(&base(bare), 0, &env(vec![silica_note()]));
    assert_eq!(headers(&table), vec!["status".to_string()]);

    // **Not separately observed**: no fixture carried a qualified `note.*`
    // key. This is the `formula.ppu` mechanism applied to the namespace the
    // UI writes (`image: note.aliases`, s22-view-config), and the demo
    // vault's own bases rely on it.
    let qualified = "properties:\n  note.status:\n    displayName: Status\nviews:\n  - type: table\n    name: V\n    order:\n      - note.status\n";
    let table = table::build(&base(qualified), 0, &env(vec![silica_note()]));
    assert_eq!(headers(&table), vec!["Status".to_string()]);
}

// ---------------------------------------------------------------------------
// The demo vault's own bases
// ---------------------------------------------------------------------------

fn demo_base(name: &str) -> (String, Base) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/silica-demo")
        .join(name);
    let bytes =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let parsed = Base::from_yaml(&bytes)
        .unwrap_or_else(|e| panic!("{} does not parse: {e}", path.display()));
    (bytes, parsed)
}

#[test]
fn the_demo_vaults_bases_are_the_bytes_obsidian_writes() {
    // The standing rule: never write a file Silica did not change. A `.base`
    // is canonicalized rather than preserved, so "unchanged" means the bytes
    // the serializer would emit — opening one of these in Silica, or in
    // Obsidian, must not move its mtime.
    for name in ["Rocks.base", "Volcanoes.base"] {
        let (bytes, parsed) = demo_base(name);
        assert_eq!(parsed.to_yaml(), bytes, "{name} is not in canonical form");
        assert!(bytes.ends_with('\n') && !bytes.ends_with("\n\n"));
        assert!(!bytes.contains('\r'), "LF only");
        assert!(!bytes.starts_with('\u{feff}'), "no BOM");
    }
}

/// The demo vault's rocks, as `docs/silica-demo/Rocks/*.md` carry them.
fn demo_rocks() -> Vec<Arc<FileData>> {
    let rock =
        |name: &str, composition: &str, silica: Option<f64>, density: f64, ves: Option<f64>| {
            let mut pairs = vec![("type", text("rock")), ("composition", text(composition))];
            if let Some(s) = silica {
                pairs.push(("silica", num(s)));
            }
            pairs.push(("density", num(density)));
            if let Some(v) = ves {
                pairs.push(("vesicularity", num(v)));
            }
            file(&format!("Rocks/{name}.md"), &["rock"], pairs)
        };
    vec![
        rock("Andesite", "intermediate", Some(58.0), 2.7, None),
        rock("Basalt", "mafic", Some(49.0), 2.9, None),
        rock("Obsidian", "rhyolitic glass", Some(74.0), 2.4, None),
        rock(
            "Pumice",
            "rhyolitic to dacitic",
            Some(72.0),
            0.6,
            Some(75.0),
        ),
        rock("Reticulite", "basaltic glass", Some(50.0), 0.1, Some(97.0)),
        rock("Rhyolite", "felsic", Some(73.0), 2.5, None),
        rock(
            "Silica",
            "basaltic to andesitic",
            Some(49.0),
            1.3,
            Some(55.0),
        ),
        rock("Tuff", "any", None, 1.6, None),
    ]
}

/// The demo vault's volcanoes.
fn demo_volcanoes() -> Vec<Arc<FileData>> {
    let volcano = |name: &str, landform: &str, height: f64, status: &str| {
        file(
            &format!("Volcanoes/{name}.md"),
            &["volcano"],
            vec![
                ("type", text("volcano")),
                ("landform", text(landform)),
                ("height_m", num(height)),
                ("status", text(status)),
            ],
        )
    };
    vec![
        file(
            "Volcanoes/Cinder cone.md",
            &["landform"],
            vec![("type", text("landform"))],
        ),
        volcano("Kīlauea", "shield volcano", 1247.0, "active"),
        volcano("Mount Etna", "stratovolcano", 3357.0, "active"),
        volcano("Parícutin", "cinder cone", 424.0, "extinct"),
        volcano("Stromboli", "stratovolcano", 924.0, "active"),
        volcano("Sunset Crater", "cinder cone", 340.0, "dormant"),
    ]
}

#[test]
fn the_demo_vaults_rock_table_shows_the_rocks() {
    let (_, parsed) = demo_base("Rocks.base");

    let all = table::build(&parsed, 0, &env(demo_rocks()));
    assert_eq!(
        headers(&all),
        vec![
            "file name".to_string(),
            "composition".to_string(),
            "Silica %".to_string(),
            "density".to_string(),
            "% of DRE".to_string(),
        ]
    );
    assert_eq!(
        names(&all),
        vec![
            "Basalt",
            "Andesite",
            "Rhyolite",
            "Obsidian",
            "Tuff",
            "Silica",
            "Pumice",
            "Reticulite",
        ],
        "densest first"
    );
    // `(density / 2.8 * 100).round()` on Basalt's 2.9.
    assert_eq!(all.rows[0].cells[4].text, "104");
    assert_eq!(all.rows[0].cells[4].kind, CellKind::Number);
    // Tuff has no `silica`, and a missing property is an empty cell.
    assert_eq!(all.rows[4].cells[2].text, "");

    let vesicular = table::build(&parsed, 1, &env(demo_rocks()));
    assert_eq!(names(&vesicular), vec!["Reticulite", "Pumice", "Silica"]);
}

#[test]
fn the_demo_vaults_volcano_table_groups_and_limits() {
    let (_, parsed) = demo_base("Volcanoes.base");

    let by_status = table::build(&parsed, 0, &env(demo_volcanoes()));
    assert_eq!(
        by_status
            .groups
            .iter()
            .map(|g| g.key.clone())
            .collect::<Vec<_>>(),
        vec![
            Some("active".to_string()),
            Some("dormant".to_string()),
            Some("extinct".to_string()),
        ],
        "the landform is filtered out, so there is no no-value group"
    );
    assert_eq!(
        names(&by_status),
        vec![
            "Mount Etna",
            "Kīlauea",
            "Stromboli",
            "Sunset Crater",
            "Parícutin",
        ]
    );
    assert_eq!(headers(&by_status)[2], "Height (m)");

    let tallest = table::build(&parsed, 1, &env(demo_volcanoes()));
    assert_eq!(names(&tallest), vec!["Mount Etna", "Kīlauea", "Stromboli"]);
    assert!(tallest.limited);
}
