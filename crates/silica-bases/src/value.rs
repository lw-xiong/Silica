//! The value model.
//!
//! Obsidian's own values are objects `{icon, data}` whose `icon` is the type
//! tag — `lucide-text` String, `lucide-binary` Number, `lucide-check-square`
//! Boolean, and so on — and the names in its error messages are String,
//! Number, Boolean, List, Date, Object, Link, File, RegExp, Duration and
//! Formulas. Both are observed; see
//! [[Specification#Observed Bases expression language, Obsidian 1.14.5 (installer build 1.13.7)]].
//!
//! Dates are *civil* here: Obsidian parses `"2026-01-02"` to local midnight and
//! prints a DateTime back in local time, so a naive date-time plus a `has_time`
//! flag is what the observations actually pin down. Nothing in this crate knows
//! a time zone.

use std::cmp::Ordering;
use std::fmt;
use std::sync::Arc;

use chrono::NaiveDateTime;
use indexmap::IndexMap;

/// A type tag, as Obsidian names it in an error message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Type {
    Null,
    String,
    Number,
    Boolean,
    List,
    Object,
    Date,
    DateTime,
    Duration,
    Link,
    File,
    RegExp,
    Tags,
    Formulas,
    Rendered,
}

impl Type {
    /// The name Obsidian uses for this type in `Cannot find function "x" on
    /// type T` and in `Expected X, given Y`.
    pub fn as_str(self) -> &'static str {
        match self {
            Type::Null => "Null",
            Type::String => "String",
            Type::Number => "Number",
            Type::Boolean => "Boolean",
            Type::List => "List",
            Type::Object => "Object",
            // Obsidian's value carries `time: true|false` on one Date class and
            // names both "Date" in an error.
            Type::Date | Type::DateTime => "Date",
            Type::Duration => "Duration",
            Type::Link => "Link",
            Type::File => "File",
            Type::RegExp => "RegExp",
            // Observed: a Tags value names itself `List` in a refusal —
            // `tags.includes()` is `Cannot find function "includes" on type
            // List`. The icon is its own; the error name is not.
            Type::Tags => "List",
            Type::Formulas => "Formulas",
            Type::Rendered => "Object",
        }
    }

    /// The `lucide-*` icon Obsidian tags a value of this type with. The view
    /// reads it; nothing in evaluation does.
    pub fn icon(self) -> &'static str {
        match self {
            Type::Null => "lucide-file-question",
            Type::String => "lucide-text",
            Type::Number => "lucide-binary",
            Type::Boolean => "lucide-check-square",
            Type::List | Type::Object => "lucide-list",
            // Only `html()` keeps the generic tag; `icon()` and `image()` are
            // `lucide-image`, which [`Value::icon`] answers because the tag
            // depends on the kind rather than on the type.
            Type::Rendered => "lucide-list",
            Type::Date => "lucide-calendar",
            Type::DateTime => "lucide-clock",
            Type::Duration => "lucide-calendar-range",
            Type::Link => "lucide-link",
            Type::File => "lucide-file",
            Type::RegExp => "lucide-regex",
            Type::Tags => "lucide-tags",
            Type::Formulas => "lucide-square-function",
        }
    }
}

/// A date or a date-time. `has_time` is Obsidian's own `time` flag: `today()`
/// is a Date and `now()` a DateTime, and the flag is what `date()` drops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateValue {
    pub at: NaiveDateTime,
    pub has_time: bool,
}

/// A duration, held as the components Obsidian's own object carries. Its
/// *properties* are fractional totals rather than these components — see
/// `funcs::time`, which owns every conversion.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct DurationValue {
    pub years: f64,
    pub months: f64,
    pub days: f64,
    pub hours: f64,
    pub minutes: f64,
    pub seconds: f64,
    pub milliseconds: f64,
}

/// A wikilink value. `display` is the alias when the link carries one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkValue {
    pub path: String,
    pub display: Option<String>,
}

impl LinkValue {
    /// What a Link shows and what its String functions receive: the alias when
    /// there is one, the path otherwise.
    pub fn text(&self) -> &str {
        self.display.as_deref().unwrap_or(&self.path)
    }
}

/// A regular expression literal. The pattern is kept as written: Obsidian
/// holds a live `RegExp` and its literal cannot be read back out of one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegexpValue {
    pub pattern: String,
    pub flags: String,
}

/// What `icon()`, `image()` and `html()` return: a thing the view renders
/// rather than a value evaluation looks into.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedValue {
    pub kind: RenderKind,
    pub source: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderKind {
    Icon,
    Image,
    Html,
}

/// A file as a value. The host fills this in: nothing in this crate reads a
/// vault.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FileData {
    pub name: String,
    pub basename: String,
    pub fullname: String,
    pub path: String,
    pub folder: String,
    pub ext: String,
    pub size: f64,
    pub ctime: Option<DateValue>,
    pub mtime: Option<DateValue>,
    /// With their `#`, as Obsidian carries them.
    pub tags: Vec<String>,
    pub links: Vec<LinkValue>,
    pub embeds: Vec<LinkValue>,
    pub backlinks: Vec<LinkValue>,
    /// The note's frontmatter, in file order.
    pub properties: IndexMap<String, Value>,
}

/// A value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Number(f64),
    Str(String),
    List(Vec<Value>),
    Object(IndexMap<String, Value>),
    Date(DateValue),
    Duration(DurationValue),
    Link(LinkValue),
    File(Arc<FileData>),
    Regexp(RegexpValue),
    /// `file.tags`. Observed as its own type tag; only `includes` was ever
    /// reached on it, and it was refused, so it behaves as a List here.
    Tags(Vec<String>),
    /// What `relative()` answers: the humanized wording — "10 months ago" —
    /// under a **clock** tag rather than a text one, which is observed and is
    /// the only value whose rendering and whose type disagree. What a further
    /// call or access on one does is unprobed.
    Relative(String),
    /// The `formula` namespace.
    Formulas,
    Rendered(RenderedValue),
}

impl Value {
    pub fn ty(&self) -> Type {
        match self {
            Value::Null => Type::Null,
            Value::Bool(_) => Type::Boolean,
            Value::Number(_) => Type::Number,
            Value::Str(_) => Type::String,
            Value::List(_) => Type::List,
            Value::Object(_) => Type::Object,
            Value::Date(d) if d.has_time => Type::DateTime,
            Value::Date(_) => Type::Date,
            Value::Duration(_) => Type::Duration,
            Value::Link(_) => Type::Link,
            Value::File(_) => Type::File,
            Value::Regexp(_) => Type::RegExp,
            Value::Tags(_) => Type::Tags,
            Value::Relative(_) => Type::DateTime,
            Value::Formulas => Type::Formulas,
            Value::Rendered(_) => Type::Rendered,
        }
    }

    /// The `lucide-*` tag Obsidian gives this value. Almost always the
    /// type's own, but a rendered value's tag depends on its kind:
    /// `icon("rock")` and `image("pic.png")` are both `lucide-image`.
    pub fn icon(&self) -> &'static str {
        match self {
            Value::Rendered(r) => match r.kind {
                RenderKind::Icon | RenderKind::Image => "lucide-image",
                RenderKind::Html => Type::Rendered.icon(),
            },
            // `relative()` answers the humanized wording under a clock tag.
            Value::Relative(_) => Type::DateTime.icon(),
            other => other.ty().icon(),
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    pub fn list(items: Vec<Value>) -> Value {
        Value::List(items)
    }

    pub fn str(s: impl Into<String>) -> Value {
        Value::Str(s.into())
    }

    /// `isTruthy()`, and what `&&`, `||` and `!` coerce through.
    pub fn is_truthy(&self) -> bool {
        match self {
            Value::Null => false,
            Value::Bool(b) => *b,
            Value::Number(n) => *n != 0.0 && !n.is_nan(),
            Value::Str(s) => !s.is_empty(),
            Value::List(items) => !items.is_empty(),
            Value::Tags(items) => !items.is_empty(),
            Value::Object(map) => !map.is_empty(),
            Value::Link(l) => !l.text().is_empty(),
            _ => true,
        }
    }

    /// The elements a List-ish value iterates as.
    pub fn as_list(&self) -> Option<Vec<Value>> {
        match self {
            Value::List(items) => Some(items.clone()),
            Value::Tags(tags) => Some(tags.iter().map(|t| Value::Str(t.clone())).collect()),
            _ => None,
        }
    }

    /// The text a String-receiver function works on. A Link is a String
    /// receiver, which is observed: `linkish.lower()` and `linkish.split()`
    /// both work.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            Value::Link(l) => Some(l.text()),
            _ => None,
        }
    }

    pub fn as_number(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            _ => None,
        }
    }
}

/// `toString()`. **Not** what a cell of a base's table renders — see
/// [`crate::cell::text_of`], which differs for null, Boolean, List, Tags and
/// Link, each read off a rendered cell.
impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Null => f.write_str("null"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Number(n) => f.write_str(&js_number(*n)),
            Value::Str(s) => f.write_str(s),
            // Observed: `List.toString()` joins with ", ".
            Value::List(items) => {
                let parts: Vec<String> = items.iter().map(|v| v.to_string()).collect();
                f.write_str(&parts.join(", "))
            }
            Value::Tags(tags) => f.write_str(&tags.join(", ")),
            Value::Object(map) => f.write_str(&json_of_object(map)),
            Value::Date(d) => {
                if d.has_time {
                    write!(f, "{}", d.at.format("%Y-%m-%dT%H:%M:%S"))
                } else {
                    write!(f, "{}", d.at.format("%Y-%m-%d"))
                }
            }
            // Humanized, and `funcs::time` owns the wording.
            Value::Duration(d) => f.write_str(&crate::funcs::time::humanize(d)),
            // Observed: `Link.toString()` is `[[Pumice]]`.
            Value::Link(l) => match &l.display {
                Some(alias) => write!(f, "[[{}|{}]]", l.path, alias),
                None => write!(f, "[[{}]]", l.path),
            },
            Value::File(file) => f.write_str(&file.path),
            Value::Regexp(r) => write!(f, "/{}/{}", r.pattern, r.flags),
            Value::Relative(text) => f.write_str(text),
            Value::Formulas => f.write_str("Formulas[]"),
            Value::Rendered(r) => f.write_str(&r.source),
        }
    }
}

/// A number as JavaScript prints one: `0.1 + 0.2` is `0.30000000000000004`,
/// `1 / 0` is `Infinity`, `0 / 0` is `NaN`. Rust's own shortest round-trip
/// formatting agrees with V8's for every finite value in the observed set.
pub fn js_number(n: f64) -> String {
    if n.is_nan() {
        return "NaN".into();
    }
    if n.is_infinite() {
        return if n > 0.0 {
            "Infinity".into()
        } else {
            "-Infinity".into()
        };
    }
    if n == 0.0 {
        // -0 prints as 0.
        return "0".into();
    }
    let mut s = format!("{n}");
    if s.ends_with(".0") {
        s.truncate(s.len() - 2);
    }
    s
}

fn json_of_object(map: &IndexMap<String, Value>) -> String {
    let parts: Vec<String> = map
        .iter()
        .map(|(k, v)| format!("{}:{}", json_string(k), json_of_value(v)))
        .collect();
    format!("{{{}}}", parts.join(","))
}

/// Obsidian stringifies every *value* before serializing the map, so an
/// Object's JSON carries no JSON types but null: `file.properties.toString()`
/// is `{"density":"1.21","flag":"true","nums":"3, 1, 2"}` — the number, the
/// boolean and the list all quoted.
fn json_of_value(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Object(map) => json_of_object(map),
        other => json_string(&other.to_string()),
    }
}

fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Loose equality, as `==` and `!=` use it. `null == null` is true and
/// `null == ""` and `null == 0` are false, which is JavaScript's rule; every
/// other pair coerces the way JavaScript's `==` does.
pub fn loose_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::Null, _) | (_, Value::Null) => false,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Number(x), Value::Number(y)) => x == y,
        (Value::Str(x), Value::Str(y)) => x == y,
        // A Date against a String parses the String as a date.
        (Value::Date(_), Value::Str(s)) | (Value::Str(s), Value::Date(_)) => {
            match crate::funcs::time::parse_date(s) {
                Some(parsed) => {
                    let other = if let Value::Date(d) = a {
                        *d
                    } else {
                        *b_date(b)
                    };
                    other.at == parsed.at
                }
                None => false,
            }
        }
        (Value::Date(x), Value::Date(y)) => x.at == y.at,
        (Value::Duration(x), Value::Duration(y)) => {
            crate::funcs::time::total_millis(x) == crate::funcs::time::total_millis(y)
        }
        (Value::List(x), Value::List(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| loose_eq(p, q))
        }
        (Value::Link(x), Value::Link(y)) => x == y,
        (Value::File(x), Value::File(y)) => x.path == y.path,
        // Boolean coerces to a number against anything but a boolean, as in
        // JavaScript: `true == 1` is true.
        (Value::Bool(x), other) => loose_eq(&Value::Number(if *x { 1.0 } else { 0.0 }), other),
        (other, Value::Bool(y)) => loose_eq(other, &Value::Number(if *y { 1.0 } else { 0.0 })),
        (Value::Number(x), Value::Str(s)) => str_as_number(s).is_some_and(|y| *x == y),
        (Value::Str(s), Value::Number(y)) => str_as_number(s).is_some_and(|x| x == *y),
        (Value::Link(l), other) => loose_eq(&Value::Str(l.text().to_string()), other),
        (other, Value::Link(l)) => loose_eq(other, &Value::Str(l.text().to_string())),
        _ => false,
    }
}

fn b_date(v: &Value) -> &DateValue {
    match v {
        Value::Date(d) => d,
        _ => unreachable!("only called on the Date side of a Date/String pair"),
    }
}

/// What a `<`, `<=`, `>` or `>=` comparison answers. Three-valued, and the
/// two failures are not the same: a **null** operand makes the comparison
/// itself null (`nosuchprop > 1` is null, not false), while two values that
/// simply do not order answer false (`"abc" > 1` is false).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmp {
    Null,
    Unordered,
    Ordered(Ordering),
}

impl Cmp {
    fn of(ord: Option<Ordering>) -> Cmp {
        match ord {
            Some(o) => Cmp::Ordered(o),
            None => Cmp::Unordered,
        }
    }
}

/// Ordering for `<`, `<=`, `>` and `>=`.
pub fn compare(a: &Value, b: &Value) -> Cmp {
    match (a, b) {
        (Value::Null, _) | (_, Value::Null) => Cmp::Null,
        (Value::Str(x), Value::Str(y)) => Cmp::Ordered(x.cmp(y)),
        (Value::Date(x), Value::Date(y)) => Cmp::Ordered(x.at.cmp(&y.at)),
        // A Date against a String parses the String as a date.
        (Value::Date(x), Value::Str(s)) => {
            Cmp::of(crate::funcs::time::parse_date(s).map(|y| x.at.cmp(&y.at)))
        }
        (Value::Str(s), Value::Date(y)) => {
            Cmp::of(crate::funcs::time::parse_date(s).map(|x| x.at.cmp(&y.at)))
        }
        (Value::Duration(x), Value::Duration(y)) => Cmp::of(
            crate::funcs::time::total_millis(x).partial_cmp(&crate::funcs::time::total_millis(y)),
        ),
        (Value::Link(l), other) => compare(&Value::Str(l.text().to_string()), other),
        (other, Value::Link(l)) => compare(other, &Value::Str(l.text().to_string())),
        // Everything else compares as numbers, and a side that is not one
        // compares as NaN does: false, whichever way the operator points.
        _ => match (numeric(a), numeric(b)) {
            (Some(x), Some(y)) => Cmp::of(x.partial_cmp(&y)),
            _ => Cmp::Unordered,
        },
    }
}

/// JavaScript's `ToNumber`, for the comparison operators only. Arithmetic does
/// not coerce: `1 + true` is an error, not 2.
pub fn numeric(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => Some(*n),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        Value::Str(s) => str_as_number(s),
        _ => None,
    }
}

/// A string read as a number the way JavaScript reads one: the whole string,
/// trimmed, or nothing. An empty string is 0.
pub fn str_as_number(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() {
        return Some(0.0);
    }
    t.parse::<f64>().ok()
}
