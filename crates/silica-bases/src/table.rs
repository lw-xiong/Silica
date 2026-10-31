//! What a view shows: a base's rows, its columns and each cell's value.
//!
//! This is the layer between [`silica_formats::base::Base`] — the file as
//! Obsidian writes it — and whatever draws a table. It reads no vault: the
//! caller hands it one [`FileData`] per note, so the engine is testable in
//! memory and the app's bridge (`silica::bases::rows`) is the only thing that
//! knows about a cache.
//!
//! The crate graph only ever points this way. `silica-formats` does not depend
//! on this crate and must not, or a writer could route a statement through an
//! AST and lose the file's own bytes.
//!
//! Three behaviours are observed and belong here rather than in the view (see
//! [[Specification#Observed Bases write behavior, Obsidian 1.14.5 (installer build 1.13.7)]]):
//!
//! - A filter that fails — to parse or to evaluate — **silently empties the
//!   view**: no rows, no banner, no notice.
//! - A formula cell whose statement failed to *parse* renders empty; one whose
//!   *evaluation* failed renders the error text as the cell's content.
//! - A cell carries `data-property-type="text|number|checkbox"`, which is the
//!   rendered type rather than the value's own, and a boolean renders as a
//!   disabled checkbox — so its text is empty even when the value is `true`.
//!
//! Everything the cells themselves do lives in [`crate::cell`]; this module
//! only decides *which* statement each cell evaluates and in what order the
//! rows come out.

use std::cmp::Ordering;
use std::sync::Arc;

use chrono::NaiveDateTime;
use indexmap::IndexMap;
use silica_formats::base::{
    Base, Filter, GroupBy, Sort, View, DEFAULT_VIEW_NAME, DEFAULT_VIEW_TYPE,
};

use crate::cell::Cell;
use crate::context::Context;
use crate::eval::{matches_filter, EvalError, Evaluator};
use crate::value::{Cmp, FileData, Value};

/// The view types Obsidian registers, read straight off
/// `plugin.getRegistrations()` in the raw run: `table` (Table,
/// `lucide-table`), `cards` (Cards, `lucide-layout-grid`), `list` (List,
/// `lucide-list`) and `kanban` (Kanban, `lucide-kanban-square`). Anything else
/// is [`Table::unknown_kind`].
///
/// All four draw the same rows and columns — only the layout differs — so
/// [`build`] resolves a view of any of them and leaves the drawing to the
/// view. `map` is **not** one of them in 1.14.5.
pub const VIEW_KINDS: &[&str] = &["table", "cards", "list", "kanban"];

/// Where a column's values come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColumnSource {
    /// A frontmatter property, by its id (`status`, or `note.status`).
    Property(String),
    /// A property of the row's file (`file.name`).
    File(String),
    /// One of the base's own formulas, by name.
    Formula(String),
}

/// A column of the table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    /// The id as the base writes it in `order`.
    pub id: String,
    /// What the header shows: `properties.<qualified id>.displayName` when
    /// the base gives one — and the key is the *qualified* id, so
    /// `properties.status` never lands where `properties['note.status']`
    /// does — else Obsidian's own label for a `file.*` property (`file name`,
    /// `created time`), else the id's last segment.
    pub display: String,
    pub source: ColumnSource,
}

/// One row: a file that passed the filters.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub file: Arc<FileData>,
    pub cells: Vec<Cell>,
    /// What the layout's `image` property answers for this row, when the
    /// view names one — the cards and kanban layouts' own key, and the only
    /// value a layout reads that is not one of the columns. `None` means the
    /// view names no image property; a named property that answers nothing
    /// is a cell with no text.
    pub image: Option<Cell>,
}

/// A `groupBy` group, in the order the view shows them.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    /// The grouped property's rendered value; `None` is Obsidian's "no value
    /// for this property" group, which `groupOrder` may name as a null.
    pub key: Option<String>,
    pub rows: Vec<Row>,
}

/// A view, resolved against the vault.
#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    pub columns: Vec<Column>,
    /// Every row in view order, groups flattened. A view with no `groupBy`
    /// has one group holding these same rows.
    pub rows: Vec<Row>,
    pub groups: Vec<Group>,
    /// Set when `limit` cut rows off the end.
    pub limited: bool,
    /// A view whose `type` is not one this build draws. Obsidian shows
    /// "Unknown view type: x" rather than rows, and the file still parses.
    pub unknown_kind: Option<String>,
}

/// What the engine needs besides the rows themselves.
pub struct Env {
    /// Every note the view may show.
    pub files: Vec<Arc<FileData>>,
    /// The `.base` file itself, which is what `this` resolves to.
    pub base_file: Option<Arc<FileData>>,
    pub now: NaiveDateTime,
}

/// One row's evaluation context: the file for `file` and a bare ident, the
/// base file for `this`, the base's formulas for `formula.<name>`.
pub struct RowContext<'a> {
    pub env: &'a Env,
    pub file: Arc<FileData>,
    pub base: &'a Base,
}

impl Context for RowContext<'_> {
    fn file(&self) -> Option<Arc<FileData>> {
        Some(self.file.clone())
    }

    fn base_file(&self) -> Option<Arc<FileData>> {
        self.env.base_file.clone()
    }

    fn formula(&self, name: &str) -> Option<String> {
        self.base
            .formulas
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.clone())
    }

    fn lookup_file(&self, path: &str) -> Option<Arc<FileData>> {
        let want = path.trim();
        self.env
            .files
            .iter()
            .find(|f| {
                f.path.eq_ignore_ascii_case(want)
                    || f.name.eq_ignore_ascii_case(want)
                    || f.basename.eq_ignore_ascii_case(want)
            })
            .cloned()
    }

    fn now(&self) -> NaiveDateTime {
        self.env.now
    }
}

// ---------------------------------------------------------------------------
// Property ids
// ---------------------------------------------------------------------------

/// A property id, split into its namespace and the name inside it.
///
/// Four spellings reach here, and all four were written by Obsidian itself in
/// the raw run's `order` lists: `file.name`, `formula.ppu`, `note.status` and
/// a bare `status`. A bare id is a frontmatter property, which is what the
/// language says too — "a bare ident resolves to a frontmatter property, so
/// `density` and `note.density` are the same value".
pub fn source_of(id: &str) -> ColumnSource {
    let (head, rest) = match id.split_once('.') {
        Some((head, rest)) => (head, rest),
        None => return ColumnSource::Property(id.to_string()),
    };
    if rest.is_empty() {
        return ColumnSource::Property(id.to_string());
    }
    match head.to_ascii_lowercase().as_str() {
        "file" => ColumnSource::File(rest.to_string()),
        "formula" => ColumnSource::Formula(rest.to_string()),
        "note" => ColumnSource::Property(rest.to_string()),
        _ => ColumnSource::Property(id.to_string()),
    }
}

/// The id a `properties` entry is keyed by, which is the **qualified** one.
///
/// This is the one surprise in the observed display names, and it is not
/// guessable: the fixture carried `properties.status.displayName: Status` and
/// a column `status`, and the rendered header was `status`, not `Status` —
/// while `properties['formula.ppu'].displayName` and
/// `properties['file.ext'].displayName` both *did* land, as "Price per unit"
/// and "Extension". The header row for `order: [file.name, status,
/// formula.ppu]` came back `['file name', 'status', 'Price per unit']`. So a
/// bare id is looked up as `note.<id>` and a hand-written bare `properties`
/// key matches nothing. (The *writer* does not qualify a bare id in `order`:
/// "a hand-written bare `status` came back `status`".)
pub fn qualified_id(id: &str) -> String {
    match source_of(id) {
        ColumnSource::Property(name) => format!("note.{name}"),
        ColumnSource::File(name) => format!("file.{name}"),
        ColumnSource::Formula(name) => format!("formula.{name}"),
    }
}

/// The header a `file.*` column shows when the base gives no `displayName`.
///
/// Read off the properties panel of the real view, which lists every column
/// the base could show under the name it would show it by: `file name`,
/// `file backlinks`, `file base name`, `created time`, `file embeds`,
/// `folder`, `file full name`, `file links`, `modified time`, `file path`,
/// `file size`, `file tags`. So the rule is `"file " + name` with five
/// exceptions, and `file.name`'s header is "file name" rather than the id's
/// last segment.
///
/// **Unprobed:** `file.ext`, whose own label the fixture's `displayName:
/// Extension` covered up, and `file.properties` and `file.file`, which the
/// panel does not offer. All three fall through to the `"file " + name` rule.
fn file_display(name: &str) -> String {
    match name.to_ascii_lowercase().as_str() {
        "basename" => "file base name".to_string(),
        "fullname" => "file full name".to_string(),
        "ctime" => "created time".to_string(),
        "mtime" => "modified time".to_string(),
        "folder" => "folder".to_string(),
        other => format!("file {other}"),
    }
}

/// One column, from a property id and the base's `properties` map.
///
/// `displayName` wins; otherwise a `file.*` id takes Obsidian's own label for
/// that file property and every other id takes its last segment — `status`
/// for `note.status`, `ppu` for `formula.ppu`, `density` for a bare
/// `density`, each of them observed as a rendered header.
pub fn column(base: &Base, id: &str) -> Column {
    let source = source_of(id);
    let display = base
        .properties
        .get(&qualified_id(id))
        .and_then(|c| c.display_name.clone())
        .unwrap_or_else(|| match &source {
            ColumnSource::File(name) => file_display(name),
            ColumnSource::Property(name) => name.clone(),
            ColumnSource::Formula(name) => name.clone(),
        });
    Column {
        id: id.to_string(),
        display,
        source,
    }
}

// ---------------------------------------------------------------------------
// Evaluation
// ---------------------------------------------------------------------------

/// What one column's statement answers for one row.
///
/// Each source is evaluated through the language rather than around it, but
/// without going back through the parser: a property name with a space or a
/// dash in it is a perfectly good frontmatter key and not a parseable
/// expression, so routing `Property` through `eval_str` would lose it.
pub fn value_of(source: &ColumnSource, ctx: &dyn Context) -> Result<Value, EvalError> {
    match source {
        // "A bare ident resolves to a frontmatter property"; a missing one is
        // null, not an error.
        ColumnSource::Property(name) => Ok(ctx.property(name).unwrap_or(Value::Null)),
        ColumnSource::File(name) => match ctx.file() {
            // Null absorbs everything, so a row with no file answers null
            // rather than refusing.
            None => Ok(Value::Null),
            Some(file) => {
                let mut ev = Evaluator::new(ctx);
                crate::funcs::file::property(&mut ev, &Value::File(file), name)
            }
        },
        ColumnSource::Formula(name) => Evaluator::new(ctx).formula(name),
    }
}

/// Whether a row passes a view's filters: the view's own, and the base's,
/// both of which may be a group of `and`/`or`/`not`. A statement that fails
/// is false, which is what empties the view.
///
/// Observed on `BadFilter.base`, whose only filter is `density >>> 1`: the
/// view renders, its header row is still there, the error banner stays
/// hidden, and the result count is "0 results".
///
/// An empty group follows the conjunctions the UI names — "All the following
/// are true", "Any of the following are true", "None of the following are
/// true" — and so answers true, false and true. **Unprobed:** Obsidian's own
/// UI cannot write an empty group, and none was in the fixtures.
pub fn passes(filters: Option<&Filter>, ctx: &dyn Context) -> bool {
    match filters {
        None => true,
        Some(Filter::Statement(statement)) => matches_filter(statement, ctx),
        Some(Filter::And(inner)) => inner.iter().all(|f| passes(Some(f), ctx)),
        Some(Filter::Or(inner)) => inner.iter().any(|f| passes(Some(f), ctx)),
        Some(Filter::Not(inner)) => !inner.iter().any(|f| passes(Some(f), ctx)),
    }
}

/// The columns a view shows: its `order`, one column per id.
///
/// A view with no `order` answers **no columns here**, because the properties
/// the rows carry are not reachable from a [`View`] alone. [`build`] fills
/// those in from the rows it just filtered; a caller that wants them without
/// building a table should use [`build`] and read [`Table::columns`].
pub fn columns(base: &Base, view: &View) -> Vec<Column> {
    view.order.iter().map(|id| column(base, id)).collect()
}

// ---------------------------------------------------------------------------
// Ordering
// ---------------------------------------------------------------------------

/// Whether a `sort` or `groupBy` direction is descending. Obsidian writes
/// `ASC` and `DESC` and labels them "A → Z" and "Z → A"; an absent direction
/// is ascending.
fn descending(direction: Option<&str>) -> bool {
    matches!(direction, Some(d) if d.eq_ignore_ascii_case("desc"))
}

/// How two values order inside a sort or a group list.
///
/// The comparison itself is [`crate::value::compare`], which is Obsidian's own
/// `<`/`>` — strings lexicographically, dates by instant, everything else
/// numerically with a coercion. Two things it leaves open are decided here:
///
/// - **A null sorts first**, and a `DESC` sort therefore puts it last. The
///   one observation is a grouped view: `groupBy: {property: status,
///   direction: ASC}` put the two rows with no `status` above the two `todo`
///   rows in the copied TSV. **Unprobed** for a `sort` rather than a group,
///   and unprobed for `DESC`.
/// - **Two values that do not order are equal**, so a stable sort leaves them
///   in the order the vault handed them over. `"abc" > 1` is false and
///   `1 > "abc"` is false too, so there is no ordering to recover.
fn order_values(a: &Value, b: &Value) -> Ordering {
    match (a.is_null(), b.is_null()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        (false, false) => match crate::value::compare(a, b) {
            Cmp::Ordered(ord) => ord,
            Cmp::Null | Cmp::Unordered => Ordering::Equal,
        },
    }
}

/// One row, with the values its sort and group keys evaluated to.
struct Keyed {
    row: Row,
    sort_keys: Vec<Value>,
    group_key: Option<Value>,
}

fn sorted(mut keyed: Vec<Keyed>, sort: &[Sort]) -> Vec<Keyed> {
    if sort.is_empty() {
        return keyed;
    }
    // Stable, so rows that tie on every key keep the order the caller gave
    // them — which is the vault's own order, and what an unsorted view shows.
    keyed.sort_by(|l, r| {
        for (i, entry) in sort.iter().enumerate() {
            let (Some(a), Some(b)) = (l.sort_keys.get(i), r.sort_keys.get(i)) else {
                continue;
            };
            let mut ord = order_values(a, b);
            if descending(entry.direction.as_deref()) {
                ord = ord.reverse();
            }
            if ord != Ordering::Equal {
                return ord;
            }
        }
        Ordering::Equal
    });
    keyed
}

/// What each `groupOrder` entry names, as a group key.
///
/// The entries are YAML values rather than strings because one of them may be
/// `null`, which is the files with no value for the grouped property. A number
/// is rendered the way a cell renders one, so `2` names the group a
/// `Value::Number(2.0)` key makes.
///
/// It takes the whole [`View`] rather than the list so that it never has to
/// name `serde_yaml`: `silica-bases` does not depend on it, and the entries'
/// type arrives through [`silica_formats`].
fn group_order_keys(view: &View) -> Vec<Option<String>> {
    view.group_order
        .iter()
        .map(|value| {
            if value.is_null() {
                None
            } else if let Some(text) = value.as_str() {
                Some(text.to_string())
            } else if let Some(flag) = value.as_bool() {
                Some(flag.to_string())
            } else if let Some(int) = value.as_i64() {
                Some(int.to_string())
            } else {
                value.as_f64().map(crate::value::js_number)
            }
        })
        .collect()
}

/// Splits the rows into groups and puts the groups in the view's order.
///
/// A group's identity is its key's **rendered string**, which is how Obsidian
/// counts distinctness elsewhere: "`[3,1,2].unique()` answers `1,2,3` while
/// `listish.unique()` answers `alpha,beta`, so distinctness is by rendered
/// string". A null key is its own group — Obsidian's "no value" one — and is
/// the group a `groupOrder` null names.
///
/// **Unprobed:** the text a group header draws, for a null key or any other
/// (no group header was ever rendered in the run), and where a group that
/// `groupOrder` does not name goes. Those groups are appended after the named
/// ones, in the `groupBy` direction's own order, which is what a hand-dragged
/// prefix implies.
fn grouped(keyed: Vec<Keyed>, group_by: &GroupBy, named: &[Option<String>]) -> Vec<Group> {
    let mut keys: Vec<Option<String>> = Vec::new();
    let mut values: Vec<Option<Value>> = Vec::new();
    let mut buckets: Vec<Vec<Row>> = Vec::new();

    for item in keyed {
        let value = item.group_key.filter(|v| !v.is_null());
        let key = value.as_ref().map(crate::cell::text_of);
        match keys.iter().position(|k| *k == key) {
            Some(ix) => buckets[ix].push(item.row),
            None => {
                keys.push(key);
                values.push(value);
                buckets.push(vec![item.row]);
            }
        }
    }

    let mut order: Vec<usize> = (0..keys.len()).collect();
    let desc = descending(group_by.direction.as_deref());
    order.sort_by(|&l, &r| {
        let a = values[l].clone().unwrap_or(Value::Null);
        let b = values[r].clone().unwrap_or(Value::Null);
        let ord = order_values(&a, &b);
        if desc {
            ord.reverse()
        } else {
            ord
        }
    });

    if !named.is_empty() {
        order.sort_by_key(|&ix| match named.iter().position(|n| *n == keys[ix]) {
            Some(at) => (0usize, at),
            None => (1, 0),
        });
    }

    order
        .into_iter()
        .map(|ix| Group {
            key: keys[ix].clone(),
            rows: std::mem::take(&mut buckets[ix]),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// build
// ---------------------------------------------------------------------------

/// The view a base with no `views` key shows. Observed: the parsed query of an
/// empty file is one default view, and closing such a file writes `views:` /
/// `  - type: table` / `    name: View`.
fn default_view() -> View {
    View {
        kind: DEFAULT_VIEW_TYPE.to_string(),
        name: DEFAULT_VIEW_NAME.to_string(),
        ..View::default()
    }
}

/// The columns a view with no `order` shows: every frontmatter property the
/// filtered rows carry, each as the qualified `note.<name>` id Obsidian writes
/// when it qualifies one itself, in the order the rows first mention them.
///
/// **Unprobed.** No fixture reached the renderer without an `order`, so
/// nothing says whether Obsidian shows every property, only the ones in its
/// own property registry, or `file.name` alongside them. What is observed is
/// the other half: `order` is exactly the columns, so *Hide column* dropping
/// `file.name` from `order` dropped it from the header row and from the copied
/// TSV, which means nothing is forced into a view that has an `order`.
fn implied_columns(base: &Base, rows: &[Arc<FileData>]) -> Vec<Column> {
    let mut seen: IndexMap<String, ()> = IndexMap::new();
    for file in rows {
        for name in file.properties.keys() {
            seen.entry(name.clone()).or_insert(());
        }
    }
    seen.keys()
        .map(|name| column(base, &format!("note.{name}")))
        .collect()
}

/// The property the view's layout draws as a row's image, when its layout
/// has an `image` key and the view sets it. Only the cards and kanban
/// layouts have one.
fn image_property(view: &View) -> Option<String> {
    match crate::layout::Layout::of(view) {
        crate::layout::Layout::Cards(cards) => cards.image,
        crate::layout::Layout::Kanban(kanban) => kanban.image,
        _ => None,
    }
}

/// Resolves one of a base's views against the vault.
///
/// `view_ix` indexes `base.views`; a base with no views has one default view,
/// which is what an empty `.base` parses to.
///
/// The order of the work is the order the observations pin down, and the one
/// step that is a choice is `limit`:
///
/// 1. **Filter.** Both the base's own filters and the view's must pass, and a
///    statement that fails either way is false — a broken filter leaves the
///    header row drawn and the result count at "0 results".
/// 2. **Columns and cells.** Every column's statement is evaluated once per
///    surviving row and handed to [`crate::cell::of`].
/// 3. **Sort**, stable, so an unsorted view keeps the order the caller handed
///    the files over in. The nine rows of `Cells.base` came out in the vault's
///    own name order with no `sort` at all, so this engine never reorders on
///    its own; whoever fills [`Env::files`] decides.
/// 4. **Limit**, applied to the sorted rows **before** grouping. *Unprobed*:
///    the one limited view in the run had `limit: 10` over four results, so
///    nothing cut. The results panel calls the control "Limit number of
///    results" and offers "Show all (4)" beside it, counting rows rather than
///    groups, which is why the cut lands here.
/// 5. **Group**, by `groupBy`, ordered by `groupOrder` and then by the
///    `groupBy` direction.
pub fn build(base: &Base, view_ix: usize, env: &Env) -> Table {
    let fallback;
    let view = match base.views.get(view_ix) {
        Some(view) => view,
        None if view_ix == 0 && base.views.is_empty() => {
            fallback = default_view();
            &fallback
        }
        None => {
            return Table {
                columns: Vec::new(),
                rows: Vec::new(),
                groups: Vec::new(),
                limited: false,
                unknown_kind: None,
            }
        }
    };

    // "Unknown view type: nonsense", no rows, and the file still parses. The
    // four registered types all draw the same rows, so only a fifth stops
    // here.
    if !VIEW_KINDS
        .iter()
        .any(|k| k.eq_ignore_ascii_case(&view.kind))
    {
        return Table {
            columns: Vec::new(),
            rows: Vec::new(),
            groups: Vec::new(),
            limited: false,
            unknown_kind: Some(view.kind.clone()),
        };
    }

    let passing: Vec<Arc<FileData>> = env
        .files
        .iter()
        .filter(|file| {
            let ctx = RowContext {
                env,
                file: Arc::clone(file),
                base,
            };
            passes(base.filters.as_ref(), &ctx) && passes(view.filters.as_ref(), &ctx)
        })
        .cloned()
        .collect();

    let columns = if view.order.is_empty() {
        implied_columns(base, &passing)
    } else {
        columns(base, view)
    };

    let group_source = view.group_by.as_ref().map(|g| source_of(&g.property));
    // The cards and kanban layouts' `image` key names a property that is not
    // one of the columns, so its value is evaluated beside the cells rather
    // than looked up among them.
    let image_source = image_property(view).map(|id| source_of(&id));
    let sort_sources: Vec<ColumnSource> =
        view.sort.iter().map(|s| source_of(&s.property)).collect();

    let mut keyed: Vec<Keyed> = passing
        .into_iter()
        .map(|file| {
            let ctx = RowContext {
                env,
                file: Arc::clone(&file),
                base,
            };
            let cells = columns
                .iter()
                .map(|col| crate::cell::of(value_of(&col.source, &ctx)))
                .collect();
            let image = image_source
                .as_ref()
                .map(|s| crate::cell::of(value_of(s, &ctx)));
            Keyed {
                // A sort or group key that refuses is null here: it orders,
                // it does not empty the view the way a filter does.
                sort_keys: sort_sources
                    .iter()
                    .map(|s| value_of(s, &ctx).unwrap_or(Value::Null))
                    .collect(),
                group_key: group_source
                    .as_ref()
                    .map(|s| value_of(s, &ctx).unwrap_or(Value::Null)),
                row: Row { file, cells, image },
            }
        })
        .collect();

    keyed = sorted(keyed, &view.sort);

    let mut limited = false;
    if let Some(limit) = view.limit {
        let limit = limit as usize;
        if keyed.len() > limit {
            keyed.truncate(limit);
            limited = true;
        }
    }

    let groups = match &view.group_by {
        Some(group_by) => {
            let named = group_order_keys(view);
            grouped(keyed, group_by, &named)
        }
        None => vec![Group {
            key: None,
            rows: keyed.into_iter().map(|k| k.row).collect(),
        }],
    };
    let rows: Vec<Row> = groups.iter().flat_map(|g| g.rows.iter().cloned()).collect();

    Table {
        columns,
        rows,
        groups,
        limited,
        unknown_kind: None,
    }
}
