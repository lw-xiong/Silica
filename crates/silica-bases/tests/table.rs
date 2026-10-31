//! The table engine against Obsidian 1.14.5's own rendered views.
//!
//! Every case marked *observed* is lifted from the CDP runs of 2026-10-25
//! (`tools/obsidian-probe/bases.mjs` and `bases-expr.mjs`, raw results in
//! `tools/obsidian-probe/results/2026-10-25-bases.json` and
//! `…-bases-expr.json`): the rendered header row of `s06-column-resize`, the
//! properties panel of `s20-panels-table`, the copied TSV of
//! `s23-header-and-results`, the registrations of `s01-registrations`, and
//! `e10-invalid`/`e11-cells` on the expression vault.
//!
//! Nothing here touches a vault: a row is a [`FileData`] built in memory, and
//! the base is parsed from the bytes the run recorded.

use std::sync::Arc;

use chrono::NaiveDateTime;
use indexmap::IndexMap;
use silica_bases::table::{self, ColumnSource, Env};
use silica_bases::{FileData, Value};
use silica_formats::base::Base;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn now() -> NaiveDateTime {
    "2026-10-25T15:29:15"
        .parse::<NaiveDateTime>()
        .expect("a fixed clock")
}

/// A note, built the way the probe vault's notes are: a root file with an
/// `.md` dropped from `file.name`.
fn note(name: &str, props: &[(&str, Value)], tags: &[&str]) -> Arc<FileData> {
    let mut properties = IndexMap::new();
    for (k, v) in props {
        properties.insert((*k).to_string(), v.clone());
    }
    Arc::new(FileData {
        name: name.into(),
        basename: name.into(),
        fullname: format!("{name}.md"),
        path: format!("{name}.md"),
        folder: "/".into(),
        ext: "md".into(),
        size: 0.0,
        tags: tags.iter().map(|t| (*t).to_string()).collect(),
        properties,
        ..FileData::default()
    })
}

fn env(files: Vec<Arc<FileData>>) -> Env {
    Env {
        files,
        base_file: None,
        now: now(),
    }
}

fn base(yaml: &str) -> Base {
    Base::from_yaml(yaml).expect("the fixture parses")
}

/// The run's own fixture, as Obsidian wrote it back at the moment the table
/// was copied to the clipboard (`s23-header-and-results`, `limitText`).
const FIXTURE: &str = r#"summaries:
  customAverage: values.mean().round(3)
  plainSummary: values.sum()
unknownTopKey:
  nested: 1
filters:
  and:
    - status != "done"
    - file.hasTag("rock")
    - or:
        - density > 1
        - not:
            - silica < 50
formulas:
  ppu: (density / silica).toFixed(2)
properties:
  status:
    unknownPropKey: kept
    displayName: Status
  formula.ppu:
    displayName: Price per unit
  file.ext:
    displayName: Extension
views:
  - type: table
    name: My table
    groupBy:
      property: status
      direction: ASC
    order:
      - status
      - formula.ppu
    sort:
      - property: file.name
        direction: DESC
      - property: status
        direction: DESC
    limit: 103
  - type: cards
    name: Cards
    order:
      - file.name
      - formula.ppu
"#;

/// The same fixture before the UI hid `file.name`: the `order` the file was
/// written with, whose rendered header row the run recorded.
const FIXTURE_WITH_FILE_NAME: &str = r#"properties:
  status:
    displayName: Status
    unknownPropKey: kept
  formula.ppu:
    displayName: Price per unit
  file.ext:
    displayName: Extension
formulas:
  ppu: "(density / silica).toFixed(2)"
views:
  - type: table
    name: My table
    order: [file.name, status, formula.ppu]
"#;

fn displays(table: &table::Table) -> Vec<&str> {
    table.columns.iter().map(|c| c.display.as_str()).collect()
}

fn texts(table: &table::Table) -> Vec<Vec<&str>> {
    table
        .rows
        .iter()
        .map(|r| r.cells.iter().map(|c| c.text.as_str()).collect())
        .collect()
}

// ---------------------------------------------------------------------------
// Column ids
// ---------------------------------------------------------------------------

#[test]
fn an_id_names_its_namespace() {
    let cases: &[(&str, ColumnSource)] = &[
        ("status", ColumnSource::Property("status".into())),
        ("note.status", ColumnSource::Property("status".into())),
        // Names are case-insensitive everywhere in the language.
        ("NOTE.status", ColumnSource::Property("status".into())),
        ("file.name", ColumnSource::File("name".into())),
        ("File.ctime", ColumnSource::File("ctime".into())),
        ("formula.ppu", ColumnSource::Formula("ppu".into())),
        // A dot that names no namespace is part of the property's own name:
        // a frontmatter key may hold one.
        ("my.key", ColumnSource::Property("my.key".into())),
        ("note.", ColumnSource::Property("note.".into())),
    ];
    for (id, want) in cases {
        assert_eq!(&table::source_of(id), want, "{id}");
    }
}

/// Observed: `order: [file.name, status, formula.ppu]` rendered the header row
/// `['file name', 'status', 'Price per unit']` (`s06-column-resize`), with
/// `properties.status.displayName: Status` sitting in the same file and *not*
/// applying.
#[test]
fn the_header_row_is_the_one_the_run_rendered() {
    let base = base(FIXTURE_WITH_FILE_NAME);
    let table = table::build(&base, 0, &env(vec![]));
    assert_eq!(displays(&table), ["file name", "status", "Price per unit"]);
}

/// A `displayName` is keyed by the **qualified** id. `formula.ppu` and
/// `file.ext` landed; a hand-written bare `status` key did not, so the column
/// kept the id's own last segment.
#[test]
fn a_display_name_is_keyed_by_the_qualified_id() {
    let base = base(
        r#"properties:
  status:
    displayName: Bare
  note.phase:
    displayName: Phase
  formula.ppu:
    displayName: Price per unit
  file.ext:
    displayName: Extension
formulas:
  ppu: "1"
views:
  - type: table
    name: V
    order: [status, phase, note.phase, formula.ppu, file.ext]
"#,
    );
    let table = table::build(&base, 0, &env(vec![]));
    assert_eq!(
        displays(&table),
        ["status", "Phase", "Phase", "Price per unit", "Extension"]
    );
}

/// The fallback header for every `file.*` column, read off the properties
/// panel of the real view (`s20-panels-table`), which lists each column under
/// the name it would show it by.
#[test]
fn a_file_column_takes_obsidians_own_label() {
    let cases: &[(&str, &str)] = &[
        ("file.name", "file name"),
        ("file.backlinks", "file backlinks"),
        ("file.basename", "file base name"),
        ("file.ctime", "created time"),
        ("file.embeds", "file embeds"),
        ("file.folder", "folder"),
        ("file.fullname", "file full name"),
        ("file.links", "file links"),
        ("file.mtime", "modified time"),
        ("file.path", "file path"),
        ("file.size", "file size"),
        ("file.tags", "file tags"),
    ];
    let base = Base::default();
    for (id, want) in cases {
        assert_eq!(&table::column(&base, id).display, want, "{id}");
    }
}

/// Observed on `e10-invalid`: `order: [file.name, density]` with no
/// `properties` at all rendered `['file name', 'density']`, and
/// `order: [file.name, formula.broken, …]` rendered the formula names.
#[test]
fn an_unconfigured_column_shows_its_last_segment() {
    let base = base(
        r#"formulas:
  broken: 'density +'
  alsoBroken: 'nosuchfunction(1)'
views:
  - type: table
    name: BadFormula
    order: [file.name, density, formula.broken, formula.alsoBroken]
"#,
    );
    let table = table::build(&base, 0, &env(vec![]));
    assert_eq!(
        displays(&table),
        ["file name", "density", "broken", "alsoBroken"]
    );
}

// ---------------------------------------------------------------------------
// Filters
// ---------------------------------------------------------------------------

#[test]
fn a_filter_group_is_all_any_and_none() {
    let rows = vec![
        note("A", &[("n", Value::Number(1.0))], &[]),
        note("B", &[("n", Value::Number(2.0))], &[]),
        note("C", &[("n", Value::Number(3.0))], &[]),
    ];
    let cases: &[(&str, &[&str])] = &[
        ("filters:\n  and: ['n > 1', 'n < 3']\n", &["B"]),
        ("filters:\n  or: ['n == 1', 'n == 3']\n", &["A", "C"]),
        ("filters:\n  not: ['n == 2']\n", &["A", "C"]),
        // Nested, as the run's own fixture nests them.
        (
            "filters:\n  and:\n    - 'n != 2'\n    - or:\n        - 'n > 2'\n        - not:\n            - 'n > 0'\n",
            &["C"],
        ),
        // An empty group follows the conjunction's own wording: all of
        // nothing is true, any of nothing is not.
        ("filters:\n  and: []\n", &["A", "B", "C"]),
        ("filters:\n  or: []\n", &[]),
        ("filters:\n  not: []\n", &["A", "B", "C"]),
    ];
    for (filters, want) in cases {
        let yaml =
            format!("{filters}views:\n  - type: table\n    name: V\n    order: [file.name]\n");
        let table = table::build(&base(&yaml), 0, &env(rows.clone()));
        let got: Vec<&str> = table.rows.iter().map(|r| r.file.name.as_str()).collect();
        assert_eq!(got, *want, "{filters}");
    }
}

/// Observed on `BadFilter.base`, whose only filter is `density >>> 1`: the
/// view renders with its header row intact, the error banner stays hidden,
/// and the result count is "0 results". A filter that fails to parse or to
/// evaluate silently empties the view.
#[test]
fn a_broken_filter_silently_empties_the_view() {
    let rows = vec![note("Silica", &[("density", Value::Number(1.21))], &[])];
    let cases = [
        "density >>> 1",
        "nosuchfunction(1)",
        "density.nosuchmethod()",
    ];
    for statement in cases {
        let yaml = format!(
            "filters:\n  and:\n    - '{statement}'\nviews:\n  - type: table\n    name: BadFilter\n    order:\n      - file.name\n"
        );
        let table = table::build(&base(&yaml), 0, &env(rows.clone()));
        assert!(table.rows.is_empty(), "{statement}");
        assert_eq!(displays(&table), ["file name"], "{statement}");
        assert!(table.unknown_kind.is_none(), "{statement}");
    }
}

/// Both the base's filters and the view's apply.
#[test]
fn the_views_filters_and_the_bases_both_apply() {
    let rows = vec![
        note("A", &[("n", Value::Number(1.0))], &[]),
        note("B", &[("n", Value::Number(2.0))], &[]),
        note("C", &[("n", Value::Number(3.0))], &[]),
    ];
    let table = table::build(
        &base(
            r#"filters:
  and: ['n > 1']
views:
  - type: table
    name: V
    order: [file.name]
    filters:
      and: ['n < 3']
"#,
        ),
        0,
        &env(rows),
    );
    let got: Vec<&str> = table.rows.iter().map(|r| r.file.name.as_str()).collect();
    assert_eq!(got, ["B"]);
}

// ---------------------------------------------------------------------------
// Cells
// ---------------------------------------------------------------------------

/// A cell is whatever evaluating its column's source answered, handed to
/// `cell::of`. Observed on `BadFormula.base`: a statement that failed to
/// *parse* renders empty, one whose *evaluation* failed renders the refusal.
#[test]
fn a_cell_is_the_columns_own_statement() {
    let rows = vec![note(
        "Silica",
        &[
            ("density", Value::Number(1.21)),
            ("silica", Value::Number(50.0)),
        ],
        &[],
    )];
    let table = table::build(
        &base(
            r#"formulas:
  ppu: "(density / silica).toFixed(2)"
  broken: 'density +'
  alsoBroken: 'nosuchfunction(1)'
views:
  - type: table
    name: V
    order: [file.name, density, formula.ppu, formula.broken, formula.alsoBroken]
"#,
        ),
        0,
        &env(rows),
    );
    assert_eq!(
        texts(&table),
        [[
            "Silica",
            "1.21",
            "0.02",
            "",
            "Cannot find function \"nosuchfunction\"",
        ]]
    );
    assert_eq!(
        table.rows[0].cells[4].error.as_deref(),
        Some("Cannot find function \"nosuchfunction\"")
    );
}

/// A frontmatter key that is not a parseable identifier is still a column: a
/// property column reads the row's properties rather than going back through
/// the expression parser.
#[test]
fn a_property_column_does_not_need_to_parse() {
    let rows = vec![note("A", &[("due date", Value::str("soon"))], &[])];
    let table = table::build(
        &base("views:\n  - type: table\n    name: V\n    order: ['due date']\n"),
        0,
        &env(rows),
    );
    assert_eq!(texts(&table), [["soon"]]);
}

// ---------------------------------------------------------------------------
// Sorting
// ---------------------------------------------------------------------------

#[test]
fn sort_orders_by_each_key_in_turn() {
    let rows = vec![
        note(
            "A",
            &[("g", Value::str("x")), ("n", Value::Number(2.0))],
            &[],
        ),
        note(
            "B",
            &[("g", Value::str("x")), ("n", Value::Number(1.0))],
            &[],
        ),
        note(
            "C",
            &[("g", Value::str("w")), ("n", Value::Number(9.0))],
            &[],
        ),
    ];
    let yaml = |sort: &str| {
        format!("views:\n  - type: table\n    name: V\n    order: [file.name]\n    sort:\n{sort}")
    };
    let cases: &[(&str, &[&str])] = &[
        ("      - property: g\n        direction: ASC\n", &["C", "A", "B"]),
        ("      - property: g\n        direction: DESC\n", &["A", "B", "C"]),
        // No direction is ascending.
        ("      - property: n\n", &["B", "A", "C"]),
        (
            "      - property: g\n        direction: ASC\n      - property: n\n        direction: DESC\n",
            &["C", "A", "B"],
        ),
        (
            "      - property: g\n        direction: DESC\n      - property: n\n        direction: ASC\n",
            &["B", "A", "C"],
        ),
        // A file property sorts like any other.
        ("      - property: file.name\n        direction: DESC\n", &["C", "B", "A"]),
    ];
    for (sort, want) in cases {
        let table = table::build(&base(&yaml(sort)), 0, &env(rows.clone()));
        let got: Vec<&str> = table.rows.iter().map(|r| r.file.name.as_str()).collect();
        assert_eq!(got, *want, "{sort}");
    }
}

/// With no `sort` the engine never reorders: the nine rows of `Cells.base`
/// came out in the order the vault handed them over. Rows that tie on every
/// sort key keep that order too, because the sort is stable.
#[test]
fn an_unsorted_view_keeps_the_vaults_order() {
    let rows = vec![
        note("Zeta", &[("g", Value::str("x"))], &[]),
        note("Alpha", &[("g", Value::str("x"))], &[]),
        note("Mu", &[("g", Value::str("x"))], &[]),
    ];
    let plain = base("views:\n  - type: table\n    name: V\n    order: [file.name]\n");
    let table = table::build(&plain, 0, &env(rows.clone()));
    assert_eq!(
        table
            .rows
            .iter()
            .map(|r| r.file.name.as_str())
            .collect::<Vec<_>>(),
        ["Zeta", "Alpha", "Mu"]
    );

    let tied = base(
        "views:\n  - type: table\n    name: V\n    order: [file.name]\n    sort:\n      - property: g\n",
    );
    let table = table::build(&tied, 0, &env(rows));
    assert_eq!(
        table
            .rows
            .iter()
            .map(|r| r.file.name.as_str())
            .collect::<Vec<_>>(),
        ["Zeta", "Alpha", "Mu"]
    );
}

/// A null sorts first, which a `DESC` sort therefore puts last. The one
/// observation is a grouped view, where the two rows with no `status` came
/// above the `todo` rows under `direction: ASC`; this is the same rule read
/// across to `sort`, and is **unprobed** there.
#[test]
fn a_null_sorts_first_and_mixed_types_hold_still() {
    let rows = vec![
        note("A", &[("n", Value::Number(2.0))], &[]),
        note("B", &[], &[]),
        note("C", &[("n", Value::Number(1.0))], &[]),
    ];
    let yaml = |dir: &str| {
        format!(
            "views:\n  - type: table\n    name: V\n    order: [file.name]\n    sort:\n      - property: n\n        direction: {dir}\n"
        )
    };
    let table = table::build(&base(&yaml("ASC")), 0, &env(rows.clone()));
    assert_eq!(
        table
            .rows
            .iter()
            .map(|r| r.file.name.as_str())
            .collect::<Vec<_>>(),
        ["B", "C", "A"]
    );
    let table = table::build(&base(&yaml("DESC")), 0, &env(rows));
    assert_eq!(
        table
            .rows
            .iter()
            .map(|r| r.file.name.as_str())
            .collect::<Vec<_>>(),
        ["A", "C", "B"]
    );

    // `"abc" > 1` is false and so is `1 > "abc"`, so there is no ordering to
    // recover: the two hold the order they came in.
    let mixed = vec![
        note("A", &[("n", Value::str("abc"))], &[]),
        note("B", &[("n", Value::Number(1.0))], &[]),
    ];
    let table = table::build(&base(&yaml("ASC")), 0, &env(mixed));
    assert_eq!(
        table
            .rows
            .iter()
            .map(|r| r.file.name.as_str())
            .collect::<Vec<_>>(),
        ["A", "B"]
    );
}

// ---------------------------------------------------------------------------
// Limit
// ---------------------------------------------------------------------------

#[test]
fn limit_cuts_the_sorted_rows_and_says_so() {
    let rows = vec![
        note("A", &[("n", Value::Number(3.0))], &[]),
        note("B", &[("n", Value::Number(1.0))], &[]),
        note("C", &[("n", Value::Number(2.0))], &[]),
    ];
    let yaml = |limit: &str| {
        format!(
            "views:\n  - type: table\n    name: V\n    order: [file.name]\n    sort:\n      - property: n\n    limit: {limit}\n"
        )
    };
    let table = table::build(&base(&yaml("2")), 0, &env(rows.clone()));
    assert_eq!(
        table
            .rows
            .iter()
            .map(|r| r.file.name.as_str())
            .collect::<Vec<_>>(),
        ["B", "C"]
    );
    assert!(table.limited);

    // A limit that cuts nothing does not set the flag — the run's own view
    // had `limit: 10` over four results.
    let table = table::build(&base(&yaml("10")), 0, &env(rows));
    assert_eq!(table.rows.len(), 3);
    assert!(!table.limited);
}

/// `limit` counts results, not groups: the panel's control is "Limit number
/// of results" and its neighbour is "Show all (4)", a row count. So the cut
/// lands before grouping. **Unprobed** — nothing in the run cut a grouped
/// view.
#[test]
fn limit_cuts_before_grouping() {
    let rows = vec![
        note("A", &[("g", Value::str("x"))], &[]),
        note("B", &[("g", Value::str("y"))], &[]),
        note("C", &[("g", Value::str("x"))], &[]),
    ];
    let table = table::build(
        &base(
            "views:\n  - type: table\n    name: V\n    order: [file.name]\n    groupBy:\n      property: g\n    limit: 2\n",
        ),
        0,
        &env(rows),
    );
    assert!(table.limited);
    assert_eq!(table.rows.len(), 2);
    let groups: Vec<(Option<&str>, usize)> = table
        .groups
        .iter()
        .map(|g| (g.key.as_deref(), g.rows.len()))
        .collect();
    assert_eq!(groups, [(Some("x"), 1), (Some("y"), 1)]);
}

// ---------------------------------------------------------------------------
// Grouping
// ---------------------------------------------------------------------------

#[test]
fn groups_come_out_in_the_group_bys_direction() {
    let rows = vec![
        note("A", &[("g", Value::str("beta"))], &[]),
        note("B", &[], &[]),
        note("C", &[("g", Value::str("alpha"))], &[]),
        note("D", &[("g", Value::str("beta"))], &[]),
    ];
    let yaml = |dir: &str| {
        format!(
            "views:\n  - type: table\n    name: V\n    order: [file.name]\n    groupBy:\n      property: g\n      direction: {dir}\n"
        )
    };
    let table = table::build(&base(&yaml("ASC")), 0, &env(rows.clone()));
    let keys: Vec<Option<&str>> = table.groups.iter().map(|g| g.key.as_deref()).collect();
    assert_eq!(keys, [None, Some("alpha"), Some("beta")]);
    // `rows` is the groups flattened, in view order.
    assert_eq!(
        table
            .rows
            .iter()
            .map(|r| r.file.name.as_str())
            .collect::<Vec<_>>(),
        ["B", "C", "A", "D"]
    );

    let table = table::build(&base(&yaml("DESC")), 0, &env(rows));
    let keys: Vec<Option<&str>> = table.groups.iter().map(|g| g.key.as_deref()).collect();
    assert_eq!(keys, [Some("beta"), Some("alpha"), None]);
}

/// A `groupOrder` entry may be a null, which is the "no value" group. Groups
/// it does not name follow, in the `groupBy` direction's own order.
#[test]
fn group_order_names_the_groups_including_the_null_one() {
    let rows = vec![
        note("A", &[("g", Value::str("alpha"))], &[]),
        note("B", &[], &[]),
        note("C", &[("g", Value::str("beta"))], &[]),
        note("D", &[("g", Value::str("gamma"))], &[]),
    ];
    let table = table::build(
        &base(
            r#"views:
  - type: table
    name: V
    order: [file.name]
    groupBy:
      property: g
    groupOrder: [beta, null, alpha]
"#,
        ),
        0,
        &env(rows),
    );
    let keys: Vec<Option<&str>> = table.groups.iter().map(|g| g.key.as_deref()).collect();
    assert_eq!(keys, [Some("beta"), None, Some("alpha"), Some("gamma")]);
}

/// A group's identity is its key's rendered string, which is how Obsidian
/// counts distinctness elsewhere ("`[3,1,2].unique()` answers `1,2,3`"), and a
/// `groupOrder` number names a numeric group.
#[test]
fn a_numeric_group_key_renders_the_way_a_cell_does() {
    let rows = vec![
        note("A", &[("g", Value::Number(2.0))], &[]),
        note("B", &[("g", Value::Number(10.0))], &[]),
    ];
    let table = table::build(
        &base(
            "views:\n  - type: table\n    name: V\n    order: [file.name]\n    groupBy:\n      property: g\n    groupOrder: [10, 2]\n",
        ),
        0,
        &env(rows),
    );
    let keys: Vec<Option<&str>> = table.groups.iter().map(|g| g.key.as_deref()).collect();
    assert_eq!(keys, [Some("10"), Some("2")]);
}

// ---------------------------------------------------------------------------
// Views
// ---------------------------------------------------------------------------

/// Observed on `BrokenView.base`: `type: nonsense` shows
/// "Unknown view type: nonsense" and no rows, and the file still parses. The
/// four registered types — `table`, `cards`, `list`, `kanban` — all draw the
/// same rows, so only a fifth stops here.
#[test]
fn an_unregistered_view_type_draws_no_rows() {
    let rows = vec![note("Silica", &[], &[])];
    for kind in ["table", "cards", "list", "kanban"] {
        let yaml =
            format!("views:\n  - type: {kind}\n    name: Odd\n    order:\n      - file.name\n");
        let table = table::build(&base(&yaml), 0, &env(rows.clone()));
        assert_eq!(table.unknown_kind, None, "{kind}");
        assert_eq!(table.rows.len(), 1, "{kind}");
    }
    let table = table::build(
        &base("views:\n  - type: nonsense\n    name: Odd\n    order:\n      - file.name\n"),
        0,
        &env(rows),
    );
    assert_eq!(table.unknown_kind.as_deref(), Some("nonsense"));
    assert!(table.rows.is_empty());
    assert!(table.columns.is_empty());
}

/// Observed: "the parsed query of an empty file is one default view", and
/// closing a zero-byte base writes `views:` / `  - type: table` / `    name:
/// View`.
#[test]
fn an_empty_base_has_one_default_table_view() {
    let rows = vec![note("A", &[("status", Value::str("todo"))], &[])];
    for text in ["", "{}\n"] {
        let table = table::build(&base(text), 0, &env(rows.clone()));
        assert_eq!(table.unknown_kind, None, "{text:?}");
        assert_eq!(table.rows.len(), 1, "{text:?}");
    }
    // There is only the one; asking for a second answers nothing.
    let table = table::build(&base(""), 1, &env(vec![]));
    assert!(table.rows.is_empty() && table.columns.is_empty() && table.unknown_kind.is_none());
}

/// A view with no `order` falls back to the properties its rows carry, as the
/// qualified `note.<name>` ids Obsidian writes when it qualifies one itself.
/// **Unprobed:** no fixture reached the renderer without an `order`.
#[test]
fn a_view_with_no_order_takes_the_rows_own_properties() {
    let rows = vec![
        note(
            "A",
            &[("type", Value::str("rock")), ("status", Value::str("todo"))],
            &[],
        ),
        note(
            "B",
            &[
                ("status", Value::str("done")),
                ("density", Value::Number(1.0)),
            ],
            &[],
        ),
    ];
    let table = table::build(&base(""), 0, &env(rows));
    let ids: Vec<&str> = table.columns.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, ["note.type", "note.status", "note.density"]);
    assert_eq!(displays(&table), ["type", "status", "density"]);
    assert_eq!(texts(&table), [["rock", "todo", ""], ["", "done", "1"]]);

    // `columns()` cannot see the rows, so it answers only the view's `order`.
    let empty = base("");
    assert!(table::columns(&empty, &silica_formats::base::View::default()).is_empty());
}

// ---------------------------------------------------------------------------
// The whole run, end to end
// ---------------------------------------------------------------------------

/// The copied TSV of `s23-header-and-results`, reproduced.
///
/// At the moment *Copy to clipboard* was pressed the fixture's view had
/// `order: [status, formula.ppu]`, `groupBy: {property: status, direction:
/// ASC}`, `sort: [file.name DESC, status DESC]` and `limit: 103`, over the
/// base's own three-deep filter. The clipboard came back
/// `status\tPrice per unit\n\t\n\t\ntodo\t0.02\ntodo\t0.06` — a header row of
/// display names and four rows, the two with no `status` first.
#[test]
fn the_copied_table_is_the_one_the_run_copied() {
    let rock = ["#rock"];
    let rows = vec![
        // Two rows with no `status`: `null != "done"` is true, they carry the
        // tag, and `not: [silica < 50]` is true of a null, so they pass. Their
        // `ppu` is null, which renders empty.
        note("Pumice", &[], &rock),
        note("Basalt", &[], &rock),
        // `density > 1` fails but `silica < 50` is false, so the `not` passes.
        note(
            "Zeta",
            &[
                ("status", Value::str("todo")),
                ("density", Value::Number(1.0)),
                ("silica", Value::Number(50.0)),
            ],
            &rock,
        ),
        note(
            "Alpha",
            &[
                ("status", Value::str("todo")),
                ("density", Value::Number(3.0)),
                ("silica", Value::Number(50.0)),
            ],
            &rock,
        ),
        // Filtered out three different ways.
        note(
            "Done",
            &[
                ("status", Value::str("done")),
                ("density", Value::Number(3.0)),
            ],
            &rock,
        ),
        note("NoTag", &[("density", Value::Number(3.0))], &[]),
        note(
            "Silicic",
            &[
                ("status", Value::str("todo")),
                ("density", Value::Number(1.0)),
                ("silica", Value::Number(10.0)),
            ],
            &rock,
        ),
    ];

    let base = base(FIXTURE);
    let table = table::build(&base, 0, &env(rows));

    assert_eq!(displays(&table), ["status", "Price per unit"]);
    assert_eq!(
        texts(&table),
        [["", ""], ["", ""], ["todo", "0.02"], ["todo", "0.06"]]
    );
    assert!(!table.limited);

    // The null-status group first, then `todo`; inside it, `file.name` DESC.
    let keys: Vec<Option<&str>> = table.groups.iter().map(|g| g.key.as_deref()).collect();
    assert_eq!(keys, [None, Some("todo")]);
    assert_eq!(
        table.groups[1]
            .rows
            .iter()
            .map(|r| r.file.name.as_str())
            .collect::<Vec<_>>(),
        ["Zeta", "Alpha"]
    );

    // The base's second view is the cards one, and it resolves the same way.
    let cards = table::build(&base, 1, &env(vec![note("Pumice", &[], &rock)]));
    assert_eq!(cards.unknown_kind, None);
    assert_eq!(displays(&cards), ["file name", "Price per unit"]);
}
