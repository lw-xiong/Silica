//! What one cell shows.
//!
//! Obsidian tags a rendered cell with `data-property-type="text|number|
//! checkbox"`, which is the *rendered* type and not the value's own: a
//! boolean renders as a **disabled checkbox**, so the cell's text is empty
//! even when the value is `true`. A statement that failed to parse renders
//! empty; one whose evaluation failed renders the refusal as the cell's
//! content. All three are observed — see
//! [[Specification#Observed Bases write behavior, Obsidian 1.14.5 (installer build 1.13.7)]].
//!
//! **A cell's text is not always `toString()`.** The one trap in the raw run
//! is that a step's `str` is `String(value.data)` rather than `toString()`, and
//! where the two differ the cell follows the *data*: a List cell shows
//! `3,1,2` where `[3,1,2].toString()` is `3, 1, 2`, and a Link cell shows
//! `Pumice` where its `toString()` is `[[Pumice]]`. [`Value`]'s [`Display`]
//! is `toString()` and stays that way; [`text_of`] is the cell's own
//! rendering, and the two agree on every type but List, Tags and Link.
//!
//! [`Display`]: std::fmt::Display

use crate::eval::EvalError;
use crate::value::Value;

/// What a cell renders as: Obsidian's own `data-property-type`.
///
/// Exactly three tags were ever seen on a base's cells across the whole run
/// (`text`, `number`, `checkbox`), although the note properties panel — a
/// different renderer — also writes `date` and `tags`. A Date, a Duration, a
/// List and a Link all landed on `text` here, which is **unprobed**: no
/// column of one of those types was inside the table's rendered window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellKind {
    Text,
    Number,
    Checkbox,
}

/// One cell of a row.
#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    pub value: Value,
    /// The evaluation's refusal, which the cell renders as its content. A
    /// statement that failed to *parse* is not this: that is an empty cell,
    /// because an unparseable statement evaluates to null rather than to an
    /// error.
    pub error: Option<String>,
    pub kind: CellKind,
    /// The text the cell draws. Empty for a checkbox and for null.
    pub text: String,
}

/// A cell from whatever evaluating its statement answered.
///
/// Observed on `BadFormula.base`, whose three formulas separate the two
/// failures and the one that is not a failure at all:
///
/// | Statement | Cell |
/// | --- | --- |
/// | `density +` | empty — it does not *parse*, so it answers null, and `of(Ok(Value::Null))` is that case |
/// | `nosuchfunction(1)` | `Cannot find function "nosuchfunction"` — the refusal is the content |
/// | `density.nosuchmethod()` | empty on a row with no `density`, because null answers every call with null |
///
/// Nothing distinguishes a statement that failed to parse from a genuine
/// null, and nothing should: both reach here as `Ok(Value::Null)`.
pub fn of(answer: Result<Value, EvalError>) -> Cell {
    match answer {
        Ok(value) => Cell {
            kind: kind_of(&value),
            text: text_of(&value),
            value,
            error: None,
        },
        // Unprobed: an error cell's own `data-property-type` was never read,
        // only its text. `text` is the most defensible reading of a cell whose
        // content is a sentence.
        Err(error) => Cell {
            value: Value::Null,
            text: error.message.clone(),
            error: Some(error.message),
            kind: CellKind::Text,
        },
    }
}

/// The rendered type tag for a value.
///
/// Observed in `e11-cells`: `1 + 2 * 3` is `number`, `"ab" + "cd"` and
/// `1 + "2"` (a String, because `+` with a String concatenates) are `text`,
/// `"3" > 2` is `checkbox`, and all three of `nosuchprop + 1`,
/// `nosuchprop.length` and `nosuchprop.toFixed(2)` — every one of them null —
/// are `text`. So the tag follows the *value's* type, not the column's.
pub fn kind_of(value: &Value) -> CellKind {
    match value {
        Value::Bool(_) => CellKind::Checkbox,
        Value::Number(_) => CellKind::Number,
        _ => CellKind::Text,
    }
}

/// The text a cell draws for a value.
///
/// Observed, from the rendered cells of `e11-cells` beside the oracle's answer
/// for the same statement:
///
/// | Value | Cell |
/// | --- | --- |
/// | String `abcd` | `abcd` |
/// | String `12` | `12` |
/// | Number `7` | `7` |
/// | Boolean `true` | *empty* — a disabled checkbox, so there is no text |
/// | null | *empty*, although `null.toString()` is the String `null` |
///
/// The rest follow `String(value.data)`, which is what the run records for
/// every value it could not render in the table's visible column window:
/// a Date `2026-01-02`, a DateTime `2026-01-02T03:04:05`, a Duration its
/// humanized wording (`10 months`, `a day`), a File its path, and a number
/// as JavaScript prints one (`Infinity`, `NaN`, `0.30000000000000004`).
/// Those agree with [`Value`]'s `toString()`. Three do not, and they are the
/// reason this is not just `value.to_string()`:
///
/// - a **List** joins with `,` and not `", "` — `listish` is
///   `alpha,beta,alpha`, `file.tags` is `#rock,#igneous/extrusive` — while
///   each *element* is still its own `toString()`, which is how `file.links`
///   reads `[[Pumice|Pumice]],[[Pumice|Pumice]],[[Basalt|Basalt]]`;
/// - a **Link** shows its text and not its wikilink syntax — `linkish` is
///   `Pumice`, not `[[Pumice]]`. The only rendered link in the run is the
///   `file.name` column's, an `internal-link` span whose text is the bare
///   name, which agrees.
///
/// **Unprobed.** Whether an aliased Link shows the alias or the path: the
/// oracle's `str` for `link("Pumice", "alias")` is `Pumice`, the path, but
/// `str` is `String(value.data)` and not what the cell draws, and a rendered
/// anchor shows its display text — so the alias is what this answers. An
/// Object is the same question and goes the other way: `String(value.data)`
/// on one is the useless `[object Object]`, plainly an artifact of reading a
/// raw map through `String`, so an Object renders as its `toString()`, the
/// JSON Obsidian builds. And `icon()`, `image()` and `html()` answer a value
/// the view *renders* rather than writes; this hands back the source, and a
/// view that can do better should read [`Cell::value`] instead.
pub fn text_of(value: &Value) -> String {
    match value {
        // Both empty, and for different reasons: a boolean is a disabled
        // checkbox, and a null cell is blank where `toString()` is `null`.
        Value::Null | Value::Bool(_) => String::new(),
        Value::List(items) => items
            .iter()
            .map(|item| item.to_string())
            .collect::<Vec<_>>()
            .join(","),
        Value::Tags(tags) => tags.join(","),
        Value::Link(link) => link.text().to_string(),
        other => other.to_string(),
    }
}
