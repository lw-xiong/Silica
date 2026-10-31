//! Frontmatter properties: the typed model, Obsidian's type inference, and
//! surgical edits to the YAML block.
//!
//! Obsidian rewrites the whole properties block whenever anything in it
//! changes. Observed with a throwaway plugin against 1.14.4, a
//! `processFrontMatter` call that mutates *nothing* still deletes YAML
//! comments, turns flow sequences into block sequences and drops blank lines
//! inside the block. Silica does not: [`plan`] returns byte ranges covering
//! only the entries that actually change, so comments, flow lists, blank
//! lines, indentation and quoting elsewhere in the block keep their bytes.
//!
//! The form Silica *writes* is Obsidian's own canonical form, which is
//! idempotent under its serializer (`---\ntitle: Hi\n---\n` round-trips
//! byte-identical through `processFrontMatter`), so a vault edited in both
//! apps converges instead of ping-ponging.
//!
//! Observed serialization, 1.14.4:
//!
//! - text: plain where YAML allows it, else double quoted (`k: "a: b"`,
//!   `k: "  padded  "`, `k: "42"`, `k: "[[A]]"`, `k: ""`); `say "hi"`,
//!   `Ä ö — ✓` and a 200-character run stay plain and unwrapped; a value
//!   with a newline becomes a `|-` block scalar indented by two spaces.
//! - number: `42`, `3.5`, `0`. checkbox: `true` / `false`.
//! - list: block sequence, two-space indent, `- ` per item, each item
//!   quoted by the scalar rules (`tags:\n  - "#a"\n  - b/c`); the empty
//!   list is the one flow form, `k: []`.
//! - null: the key with nothing after it (`k:`).
//! - a new property goes after the last entry; removing the last property
//!   removes the fences too, leaving the body alone (`---\na: 1\n---\nbody\n`
//!   becomes `body\n`).

use crate::config::PropertyTypes;
use std::ops::Range;

/// A property's type, as `types.json` records it and the properties UI shows
/// it. The names are Obsidian 1.14.4's `registeredTypeWidgets` keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PropertyType {
    Text,
    /// Obsidian calls this "List" in the UI; `multitext` on disk.
    MultiText,
    Number,
    Checkbox,
    Date,
    DateTime,
    /// Reserved for the `tags` key.
    Tags,
    /// Reserved for the `aliases` key.
    Aliases,
}

impl PropertyType {
    /// The `types.json` value.
    pub fn as_str(self) -> &'static str {
        match self {
            PropertyType::Text => "text",
            PropertyType::MultiText => "multitext",
            PropertyType::Number => "number",
            PropertyType::Checkbox => "checkbox",
            PropertyType::Date => "date",
            PropertyType::DateTime => "datetime",
            PropertyType::Tags => "tags",
            PropertyType::Aliases => "aliases",
        }
    }

    pub fn parse(s: &str) -> Option<PropertyType> {
        Some(match s {
            "text" => PropertyType::Text,
            "multitext" => PropertyType::MultiText,
            "number" => PropertyType::Number,
            "checkbox" => PropertyType::Checkbox,
            "date" => PropertyType::Date,
            "datetime" => PropertyType::DateTime,
            "tags" => PropertyType::Tags,
            "aliases" => PropertyType::Aliases,
            _ => return None,
        })
    }

    /// The label Obsidian's type menu shows, from its `registeredTypeWidgets`.
    pub fn label(self) -> &'static str {
        match self {
            PropertyType::Text => "Text",
            PropertyType::MultiText => "List",
            PropertyType::Number => "Number",
            PropertyType::Checkbox => "Checkbox",
            PropertyType::Date => "Date",
            PropertyType::DateTime => "Date & time",
            PropertyType::Tags => "Tags",
            PropertyType::Aliases => "Aliases",
        }
    }

    /// Whether the value is a sequence of strings.
    pub fn is_list(self) -> bool {
        matches!(
            self,
            PropertyType::MultiText | PropertyType::Tags | PropertyType::Aliases
        )
    }

    /// The types the properties menu offers, in Obsidian's menu order. `tags`
    /// and `aliases` are reserved for their keys and never offered.
    pub fn selectable() -> &'static [PropertyType] {
        &[
            PropertyType::Text,
            PropertyType::MultiText,
            PropertyType::Number,
            PropertyType::Checkbox,
            PropertyType::Date,
            PropertyType::DateTime,
        ]
    }
}

/// `propertiesInDocument` in `app.json`: how the editor shows the block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PropertiesInDocument {
    /// The properties table, editable. Obsidian's default.
    #[default]
    Visible,
    /// No block in the editor at all; the YAML is still in the file.
    Hidden,
    /// The raw YAML, as source mode shows it.
    Source,
}

impl PropertiesInDocument {
    /// Parses the config value. `None` and an unknown string are Obsidian's
    /// default, `visible` (confirmed through `vault.getConfig` with the key
    /// absent from `app.json`).
    pub fn from_config(value: Option<&str>) -> PropertiesInDocument {
        match value {
            Some("hidden") => PropertiesInDocument::Hidden,
            Some("source") => PropertiesInDocument::Source,
            _ => PropertiesInDocument::Visible,
        }
    }
}

/// Keys whose type Obsidian fixes regardless of their value, from the
/// `reservedKeys` of its type widgets. `cssclasses` is *not* one of them: it
/// infers as `multitext`, and that is what `types.json` records for it.
pub fn reserved_type(key: &str) -> Option<PropertyType> {
    match key {
        "tags" => Some(PropertyType::Tags),
        "aliases" => Some(PropertyType::Aliases),
        _ => None,
    }
}

/// A property value, in the shapes the properties UI can edit. Anything else
/// in the block (a nested mapping, a sequence of mappings) stays
/// [`Value::Other`] and is shown but not edited, the way Obsidian types it
/// `text`.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// A scalar string.
    Text(String),
    Number(f64),
    Checkbox(bool),
    List(Vec<String>),
    /// `key:` with no value, or an explicit `null`.
    Empty,
    /// Not editable; the entry's YAML text as written.
    Other(String),
}

impl Value {
    /// The type Obsidian infers from the value alone, before `types.json` and
    /// the reserved keys are consulted. Observed 1.14.4: strings are `text`
    /// except a bare `YYYY-MM-DD`, which is `date`; YAML numbers are `number`,
    /// YAML booleans `checkbox`, sequences `multitext`; null, a nested mapping
    /// and a block scalar are all `text`. An ISO *date-time* string infers as
    /// `text`, not `datetime` — `datetime` is only ever an assigned type.
    pub fn inferred_type(&self) -> PropertyType {
        match self {
            Value::Number(_) => PropertyType::Number,
            Value::Checkbox(_) => PropertyType::Checkbox,
            Value::List(_) => PropertyType::MultiText,
            Value::Text(s) if is_date(s) => PropertyType::Date,
            Value::Text(_) | Value::Empty | Value::Other(_) => PropertyType::Text,
        }
    }

    /// The value as the single-line text an editable field shows. A list joins
    /// with `, `; `Other` gives its YAML back.
    pub fn display(&self) -> String {
        match self {
            Value::Text(s) => s.clone(),
            Value::Number(n) => format_number(*n),
            Value::Checkbox(b) => b.to_string(),
            Value::List(items) => items.join(", "),
            Value::Empty => String::new(),
            Value::Other(s) => s.trim().to_string(),
        }
    }

    /// Reads `text` as a value of `ty`, the way a field commits what was typed.
    /// A number that does not parse and a checkbox that is not `true` or
    /// `false` stay text: Obsidian keeps what you typed and flags the row.
    pub fn parse_as(text: &str, ty: PropertyType) -> Value {
        if ty.is_list() {
            return Value::List(
                text.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect(),
            );
        }
        let trimmed = text.trim();
        match ty {
            PropertyType::Number if trimmed.is_empty() => Value::Empty,
            PropertyType::Number => match trimmed.parse::<f64>() {
                Ok(n) if n.is_finite() => Value::Number(n),
                _ => Value::Text(text.to_string()),
            },
            PropertyType::Checkbox => match trimmed {
                "true" => Value::Checkbox(true),
                "false" => Value::Checkbox(false),
                "" => Value::Empty,
                other => Value::Text(other.to_string()),
            },
            _ if text.is_empty() => Value::Empty,
            _ => Value::Text(text.to_string()),
        }
    }
}

/// `YYYY-MM-DD`, the one string shape Obsidian infers as `date`.
fn is_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[8..].iter().all(u8::is_ascii_digit)
}

/// `42` not `42.0`, and no exponent for the magnitudes a note carries.
fn format_number(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

/// One top-level entry of the properties block, with the byte ranges that let
/// an edit replace it without touching anything else. Offsets are relative to
/// the start of the *file*, not of the YAML text.
#[derive(Debug, Clone, PartialEq)]
pub struct Property {
    pub key: String,
    pub value: Value,
    /// The whole entry: the key through the last line of its value, with no
    /// trailing newline and no trailing blank or comment lines.
    pub span: Range<usize>,
    /// Just the key token, so a rename keeps the entry in place.
    pub key_span: Range<usize>,
}

/// The properties block of a note: its entries in file order, and the ranges
/// of the block itself.
#[derive(Debug, Clone, PartialEq)]
pub struct Properties {
    pub entries: Vec<Property>,
    /// The whole block including both fence lines and the newline after the
    /// closing fence.
    pub block: Range<usize>,
    /// The YAML between the fences, excluding both fence lines.
    pub yaml: Range<usize>,
    /// `\r\n` when the block's lines end that way, else `\n`.
    pub newline: String,
}

impl Properties {
    pub fn get(&self, key: &str) -> Option<&Property> {
        self.entries.iter().find(|p| p.key == key)
    }

    /// The type of `key`: `types.json` first, then the reserved keys, then
    /// inference from the value. Obsidian consults them in that order.
    pub fn type_of(&self, key: &str, types: &PropertyTypes) -> PropertyType {
        type_of(key, &self.entries, types)
    }
}

/// The type of `key` among `entries`: `types.json` first, then the reserved
/// keys, then inference from the value. Obsidian consults them in that order.
///
/// Takes the entries rather than a whole [`Properties`] so a renderer can ask
/// per row without rebuilding the block.
pub fn type_of(key: &str, entries: &[Property], types: &PropertyTypes) -> PropertyType {
    if let Some(ty) = types.types.get(key).and_then(|s| PropertyType::parse(s)) {
        return ty;
    }
    if let Some(ty) = reserved_type(key) {
        return ty;
    }
    entries
        .iter()
        .find(|p| p.key == key)
        .map(|p| p.value.inferred_type())
        .unwrap_or(PropertyType::Text)
}

/// Parses the properties block of `text`.
///
/// `yaml_range` is the range of the YAML between the fences and `block_range`
/// the whole block, as `silica_markdown::extract_frontmatter` reports them;
/// this crate does not depend on the parser.
pub fn parse(text: &str, yaml_range: Range<usize>, block_range: Range<usize>) -> Properties {
    let newline = if text[block_range.clone()].contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    Properties {
        entries: parse_entries(&text[yaml_range.clone()], yaml_range.start),
        block: block_range,
        yaml: yaml_range,
        newline: newline.to_string(),
    }
}

/// Splits the YAML text into top-level entries. A new entry starts on an
/// unindented line that is not blank, a comment, a document marker or a
/// sequence item; everything indented, blank or part of a block scalar belongs
/// to the entry above it. The value of each entry is then read with
/// `serde_yaml`, so the shapes are YAML's and only the boundaries are scanned
/// here.
fn parse_entries(yaml: &str, base: usize) -> Vec<Property> {
    // (entry start, key range within the line, key)
    let mut starts: Vec<(usize, Range<usize>, String)> = Vec::new();
    let mut at = 0;
    while at < yaml.len() {
        let end = yaml[at..].find('\n').map(|i| at + i).unwrap_or(yaml.len());
        let line = yaml[at..end].trim_end_matches('\r');
        if !line.starts_with([' ', '\t']) && !line.trim().is_empty() && !line.starts_with('#') {
            if let Some((key, key_range)) = scan_key(line) {
                starts.push((at, at + key_range.start..at + key_range.end, key));
            }
        }
        at = end + 1;
    }
    let mut out = Vec::with_capacity(starts.len());
    for (ix, (start, key_range, key)) in starts.iter().enumerate() {
        let next = starts.get(ix + 1).map(|(s, _, _)| *s).unwrap_or(yaml.len());
        let entry_end = entry_end(yaml, *start, next);
        out.push(Property {
            key: key.clone(),
            value: read_value(&yaml[*start..entry_end], key),
            span: base + start..base + entry_end,
            key_span: base + key_range.start..base + key_range.end,
        });
    }
    out
}

/// The key of a top-level `key:` line and its range within the line. YAML
/// allows a colon inside a plain key (`a:b: v` is the key `a:b`), so the split
/// is at the first `": "`, or at a trailing `:`. A quoted key is matched to its
/// closing quote first, so `"odd: key": v` keeps its colon.
fn scan_key(line: &str) -> Option<(String, Range<usize>)> {
    if line.starts_with("---") || line.starts_with("...") || line.starts_with("- ") {
        return None;
    }
    let bytes = line.as_bytes();
    let raw_end = if bytes[0] == b'"' || bytes[0] == b'\'' {
        let quote = bytes[0];
        let mut i = 1;
        while i < bytes.len() {
            if bytes[i] == b'\\' && quote == b'"' {
                i += 2;
                continue;
            }
            if bytes[i] == quote {
                i += 1;
                break;
            }
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b':' {
            return None;
        }
        i
    } else {
        match line.find(": ") {
            Some(i) => i,
            None if line.ends_with(':') => line.len() - 1,
            None => return None,
        }
    };
    let key = unquote(line[..raw_end].trim_end());
    if key.is_empty() {
        return None;
    }
    Some((key, 0..raw_end))
}

/// Strips one layer of YAML quoting from a key.
fn unquote(s: &str) -> String {
    let bytes = s.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        if first == bytes[bytes.len() - 1] && (first == b'"' || first == b'\'') {
            let inner = &s[1..s.len() - 1];
            return if first == b'"' {
                inner.replace("\\\"", "\"").replace("\\\\", "\\")
            } else {
                inner.replace("''", "'")
            };
        }
    }
    s.to_string()
}

/// Shrinks `[start, next)` to the entry itself: the trailing newline and any
/// blank or comment lines after the value belong to the block, not the entry,
/// so an edit must not swallow them.
fn entry_end(yaml: &str, start: usize, next: usize) -> usize {
    let mut end = next;
    loop {
        let body = &yaml[start..end];
        let trimmed = body.trim_end_matches(['\n', '\r']);
        if trimmed.len() < body.len() {
            end = start + trimmed.len();
            continue;
        }
        let line_start = match body.rfind('\n') {
            Some(i) => start + i + 1,
            None => return end,
        };
        let last = &yaml[line_start..end];
        if last.trim().is_empty() || last.trim_start().starts_with('#') {
            end = line_start;
            continue;
        }
        return end;
    }
}

/// Interprets one entry's YAML text as a value.
fn read_value(entry: &str, key: &str) -> Value {
    let parsed: serde_yaml::Value = match serde_yaml::from_str(entry) {
        Ok(v) => v,
        // A duplicate key or otherwise invalid entry: keep the text.
        Err(_) => return Value::Other(entry.to_string()),
    };
    let serde_yaml::Value::Mapping(map) = parsed else {
        return Value::Other(entry.to_string());
    };
    let found = map
        .iter()
        .find(|(k, _)| matches!(k, serde_yaml::Value::String(s) if s == key))
        .map(|(_, v)| v);
    match found {
        Some(value) => from_yaml(value, entry),
        None => Value::Other(entry.to_string()),
    }
}

fn from_yaml(value: &serde_yaml::Value, entry: &str) -> Value {
    match value {
        serde_yaml::Value::Null => Value::Empty,
        serde_yaml::Value::Bool(b) => Value::Checkbox(*b),
        serde_yaml::Value::Number(n) => match n.as_f64() {
            Some(f) => Value::Number(f),
            None => Value::Other(entry.to_string()),
        },
        serde_yaml::Value::String(s) => Value::Text(s.clone()),
        serde_yaml::Value::Sequence(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                match item {
                    serde_yaml::Value::String(s) => out.push(s.clone()),
                    serde_yaml::Value::Number(n) => out.push(n.to_string()),
                    serde_yaml::Value::Bool(b) => out.push(b.to_string()),
                    // A sequence of anything else is not an editable list.
                    _ => return Value::Other(entry.to_string()),
                }
            }
            Value::List(out)
        }
        _ => Value::Other(entry.to_string()),
    }
}

// ---------------------------------------------------------------- rendering

/// Renders `key: value` as Obsidian writes it, with `newline` between lines
/// and no trailing newline.
pub fn render_entry(key: &str, value: &Value, newline: &str) -> String {
    let key = render_key(key);
    match value {
        Value::Empty => format!("{key}:"),
        Value::Number(n) => format!("{key}: {}", format_number(*n)),
        Value::Checkbox(b) => format!("{key}: {b}"),
        Value::Text(s) if s.contains('\n') => {
            let mut out = format!("{key}: |-");
            for line in s.split('\n') {
                out.push_str(newline);
                out.push_str("  ");
                out.push_str(line.trim_end_matches('\r'));
            }
            out
        }
        Value::Text(s) => format!("{key}: {}", render_scalar(s)),
        Value::List(items) if items.is_empty() => format!("{key}: []"),
        Value::List(items) => {
            let mut out = format!("{key}:");
            for item in items {
                out.push_str(newline);
                out.push_str("  - ");
                out.push_str(&render_scalar(item));
            }
            out
        }
        Value::Other(text) => text.trim_end_matches(['\n', '\r']).to_string(),
    }
}

/// A key, quoted only where YAML needs it.
fn render_key(key: &str) -> String {
    if key.is_empty()
        || key.trim() != key
        || key.contains(": ")
        || key.ends_with(':')
        || key.starts_with([
            '#', '-', '?', '&', '*', '!', '[', ']', '{', '}', '"', '\'', ',',
        ])
    {
        return quote(key);
    }
    key.to_string()
}

/// A scalar, plain where YAML allows it and double quoted otherwise. The rules
/// are Obsidian 1.14.4's output: `say "hi"` and `https://x` stay plain, while
/// an empty string, padding, `a: b`, a leading `#`, and anything that would
/// read back as a number, boolean, null or date are quoted.
pub fn render_scalar(s: &str) -> String {
    if needs_quotes(s) {
        quote(s)
    } else {
        s.to_string()
    }
}

fn needs_quotes(s: &str) -> bool {
    if s.is_empty() || s.trim() != s {
        return true;
    }
    if s.contains('\n') || s.contains(": ") || s.ends_with(':') {
        return true;
    }
    // An inline comment would start at " #".
    if s.contains(" #") {
        return true;
    }
    if s.starts_with([
        '#', '&', '*', '!', '|', '>', '%', '@', '`', '\'', '"', '[', ']', '{', '}', ',',
    ]) {
        return true;
    }
    if s == "-" || s.starts_with("- ") {
        return true;
    }
    // Would read back as something other than a string. A bare `YYYY-MM-DD`
    // would not: Obsidian's YAML reads it back as the string it wrote, and
    // writes it unquoted (`d: 2026-10-18`), so dates stay bare.
    s.parse::<f64>().is_ok()
        || matches!(
            s,
            "true" | "false" | "True" | "False" | "TRUE" | "FALSE" | "null" | "Null" | "NULL" | "~"
        )
}

fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

// ------------------------------------------------------------------- edits

/// A byte range of the file to replace with `text`, as `silica_cache::links`
/// reports link rewrites.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub start: usize,
    pub end: usize,
    pub text: String,
}

/// What to do to one property.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    /// Replace the value of an existing key, or add the property after the
    /// last entry when the key is new.
    Set { key: String, value: Value },
    /// Rename a key, keeping the entry where it is. Obsidian's own
    /// `processFrontMatter` moves a renamed key to the end of the block;
    /// keeping it in place changes fewer bytes, so that is what Silica does.
    Rename { from: String, to: String },
    /// Remove a property. Removing the last one removes the block.
    Remove { key: String },
}

/// Plans `changes` against the block `props` describes, over the file `text`.
/// Returns the edits in file order, or `None` when nothing would change.
///
/// Removing every property removes the fences as well, which is what Obsidian
/// does: `---\na: 1\n---\nbody\n` becomes `body\n`.
pub fn plan(text: &str, props: &Properties, changes: &[Change]) -> Option<Vec<Edit>> {
    let nl = props.newline.as_str();
    let removes: Vec<&str> = changes
        .iter()
        .filter_map(|c| match c {
            Change::Remove { key } if props.get(key).is_some() => Some(key.as_str()),
            _ => None,
        })
        .collect();
    let adds = changes
        .iter()
        .any(|c| matches!(c, Change::Set { key, .. } if props.get(key).is_none()));
    let all_gone = !props.entries.is_empty()
        && props
            .entries
            .iter()
            .all(|p| removes.contains(&p.key.as_str()))
        && !adds;
    if all_gone {
        return Some(vec![Edit {
            start: props.block.start,
            end: props.block.end,
            text: String::new(),
        }]);
    }

    // A new property goes after the last entry that survives this plan; with
    // none left it goes at the start of the YAML, so the insertion point never
    // falls inside a range some other change removes.
    let kept_last = props
        .entries
        .iter()
        .rfind(|p| !removes.contains(&p.key.as_str()));
    let append_at = kept_last.map(|p| p.span.end).unwrap_or(props.yaml.start);
    let mut edits: Vec<Edit> = Vec::new();
    for change in changes {
        match change {
            Change::Set { key, value } => match props.get(key) {
                // An unchanged value is left exactly as written. Comparing
                // values rather than rendered text is what keeps a flow
                // sequence (`tags: [a, b]`) and its quoting intact: the
                // canonical block form is only written when the value really
                // changes.
                Some(existing) if existing.value == *value => {}
                Some(existing) => edits.push(Edit {
                    start: existing.span.start,
                    end: existing.span.end,
                    text: render_entry(key, value, nl),
                }),
                None => {
                    let entry = render_entry(key, value, nl);
                    let text = match kept_last {
                        Some(_) => format!("{nl}{entry}"),
                        None => format!("{entry}{nl}"),
                    };
                    edits.push(Edit {
                        start: append_at,
                        end: append_at,
                        text,
                    });
                }
            },
            Change::Rename { from, to } => {
                if let Some(existing) = props.get(from) {
                    edits.push(Edit {
                        start: existing.key_span.start,
                        end: existing.key_span.end,
                        text: render_key(to),
                    });
                }
            }
            Change::Remove { key } => {
                if let Some(existing) = props.get(key) {
                    // Take the newline that ends the entry with it.
                    let mut end = existing.span.end;
                    if text[end..].starts_with("\r\n") {
                        end += 2;
                    } else if text[end..].starts_with('\n') {
                        end += 1;
                    }
                    edits.push(Edit {
                        start: existing.span.start,
                        end,
                        text: String::new(),
                    });
                }
            }
        }
    }
    // Stable, so two properties added at the same point keep their order.
    edits.sort_by_key(|e| (e.start, e.end));
    edits.retain(|e| text.get(e.start..e.end) != Some(e.text.as_str()));
    if edits.is_empty() {
        return None;
    }
    Some(edits)
}

/// Applies planned edits to `text`.
pub fn apply(text: &str, edits: &[Edit]) -> String {
    let mut out = String::with_capacity(text.len() + 64);
    let mut at = 0;
    for edit in edits {
        if edit.start < at {
            continue;
        }
        out.push_str(&text[at..edit.start]);
        out.push_str(&edit.text);
        at = edit.end;
    }
    out.push_str(&text[at..]);
    out
}

/// The block to insert above a note that has none, as Obsidian does when a
/// property is added to a note without frontmatter: the block goes above
/// everything, with no blank line between it and the body.
pub fn new_block(key: &str, value: &Value, newline: &str) -> String {
    let entry = render_entry(key, value, newline);
    format!("---{newline}{entry}{newline}---{newline}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Splits a note the way `silica_markdown::extract_frontmatter` does, so
    /// these tests do not depend on the parser crate.
    fn split(text: &str) -> Properties {
        let after_open = text.find('\n').unwrap() + 1;
        let close = text[after_open..].find("---").unwrap() + after_open;
        let block_end = text[close..]
            .find('\n')
            .map(|i| close + i + 1)
            .unwrap_or(text.len());
        parse(text, after_open..close, 0..block_end)
    }

    // ------------------------------------------------------------ parsing

    #[test]
    fn parses_entries_in_file_order_with_spans() {
        let text = "---\ntitle: Hi\ncount: 3\n---\nbody\n";
        let p = split(text);
        assert_eq!(p.entries.len(), 2);
        assert_eq!(p.entries[0].key, "title");
        assert_eq!(p.entries[0].value, Value::Text("Hi".into()));
        assert_eq!(&text[p.entries[0].span.clone()], "title: Hi");
        assert_eq!(&text[p.entries[0].key_span.clone()], "title");
        assert_eq!(p.entries[1].value, Value::Number(3.0));
        assert_eq!(&text[p.entries[1].span.clone()], "count: 3");
    }

    #[test]
    fn parses_every_value_shape() {
        let text = concat!(
            "---\n",
            "plain: hello\n",
            "quoted: \"hello\"\n",
            "num: 42\n",
            "float: 3.5\n",
            "yes_str: yes\n",
            "flag: true\n",
            "nul: null\n",
            "empty:\n",
            "day: 2026-10-18\n",
            "flow: [a, b]\n",
            "block:\n  - a\n  - b\n",
            "nested:\n  k: v\n",
            "wrapped: |-\n  one\n  two\n",
            "---\n",
        );
        let p = split(text);
        let v = |k: &str| p.get(k).unwrap().value.clone();
        assert_eq!(v("plain"), Value::Text("hello".into()));
        assert_eq!(v("quoted"), Value::Text("hello".into()));
        assert_eq!(v("num"), Value::Number(42.0));
        assert_eq!(v("float"), Value::Number(3.5));
        // YAML 1.2 core: `yes` is a string, and Obsidian types it text.
        assert_eq!(v("yes_str"), Value::Text("yes".into()));
        assert_eq!(v("flag"), Value::Checkbox(true));
        assert_eq!(v("nul"), Value::Empty);
        assert_eq!(v("empty"), Value::Empty);
        assert_eq!(v("day"), Value::Text("2026-10-18".into()));
        assert_eq!(v("flow"), Value::List(vec!["a".into(), "b".into()]));
        assert_eq!(v("block"), Value::List(vec!["a".into(), "b".into()]));
        assert!(matches!(v("nested"), Value::Other(_)));
        assert_eq!(v("wrapped"), Value::Text("one\ntwo".into()));
        // A multi-line entry's span covers all of its lines and no newline.
        assert_eq!(
            &text[p.get("block").unwrap().span.clone()],
            "block:\n  - a\n  - b"
        );
    }

    #[test]
    fn keys_with_colons_quotes_and_spaces() {
        let text = "---\na:b: v\ntwo words: v\n\"odd: key\": v\nUnicode: v\n---\n";
        let p = split(text);
        let keys: Vec<&str> = p.entries.iter().map(|e| e.key.as_str()).collect();
        assert_eq!(keys, vec!["a:b", "two words", "odd: key", "Unicode"]);
        assert_eq!(
            &text[p.get("odd: key").unwrap().key_span.clone()],
            "\"odd: key\""
        );
    }

    #[test]
    fn comments_and_blank_lines_are_not_part_of_an_entry() {
        let text = "---\n# lead\na: 1\n\n# between\nb: 2\n---\n";
        let p = split(text);
        assert_eq!(p.entries.len(), 2);
        assert_eq!(&text[p.entries[0].span.clone()], "a: 1");
        assert_eq!(&text[p.entries[1].span.clone()], "b: 2");
    }

    #[test]
    fn duplicate_keys_are_kept_as_text() {
        // Obsidian's YAML parser rejects the whole block ("Map keys must be
        // unique"); Silica keeps the entries as unparsed text rather than
        // losing them.
        let text = "---\na: 1\na: 2\n---\n";
        let p = split(text);
        assert_eq!(p.entries.len(), 2);
        assert!(p.entries.iter().all(|e| e.key == "a"));
    }

    #[test]
    fn crlf_block_is_recognised() {
        let text = "---\r\ntitle: Hi\r\n---\r\nbody\r\n";
        let p = split(text);
        assert_eq!(p.newline, "\r\n");
        assert_eq!(p.entries[0].value, Value::Text("Hi".into()));
        assert_eq!(&text[p.entries[0].span.clone()], "title: Hi");
    }

    // ---------------------------------------------------------- inference

    #[test]
    fn inference_matches_obsidian_1_14_4() {
        let t = |v: Value| v.inferred_type();
        assert_eq!(t(Value::Text("hello".into())), PropertyType::Text);
        assert_eq!(t(Value::Number(42.0)), PropertyType::Number);
        assert_eq!(t(Value::Checkbox(true)), PropertyType::Checkbox);
        assert_eq!(t(Value::List(vec![])), PropertyType::MultiText);
        assert_eq!(t(Value::Empty), PropertyType::Text);
        // A bare date infers date; an ISO date-time does not infer datetime.
        assert_eq!(t(Value::Text("2026-10-18".into())), PropertyType::Date);
        assert_eq!(
            t(Value::Text("2026-10-18T09:30:00".into())),
            PropertyType::Text
        );
        assert_eq!(t(Value::Text("2026-13-99".into())), PropertyType::Date);
    }

    #[test]
    fn reserved_keys_beat_inference_and_types_json_beats_both() {
        let text = "---\ntags: [x]\naliases: [One]\ncssclasses: [c]\ncount: 3\n---\n";
        let p = split(text);
        let mut types = PropertyTypes::default();
        assert_eq!(p.type_of("tags", &types), PropertyType::Tags);
        assert_eq!(p.type_of("aliases", &types), PropertyType::Aliases);
        // cssclasses is a plain list, not a type of its own.
        assert_eq!(p.type_of("cssclasses", &types), PropertyType::MultiText);
        assert_eq!(p.type_of("count", &types), PropertyType::Number);
        types.types.insert("count".into(), "text".into());
        assert_eq!(p.type_of("count", &types), PropertyType::Text);
        // An unknown type name in types.json falls back to inference.
        types.types.insert("count".into(), "nonsense".into());
        assert_eq!(p.type_of("count", &types), PropertyType::Number);
    }

    // ---------------------------------------------------------- rendering

    #[test]
    fn renders_scalars_the_way_obsidian_writes_them() {
        let r = |s: &str| render_entry("k", &Value::Text(s.into()), "\n");
        assert_eq!(r("a: b"), "k: \"a: b\"");
        assert_eq!(r("a #b"), "k: \"a #b\"");
        assert_eq!(r("say \"hi\""), "k: say \"hi\"");
        assert_eq!(r("  padded  "), "k: \"  padded  \"");
        assert_eq!(r(""), "k: \"\"");
        assert_eq!(r("42"), "k: \"42\"");
        assert_eq!(r("[[Target]]"), "k: \"[[Target]]\"");
        assert_eq!(r("Ä ö — ✓"), "k: Ä ö — ✓");
        assert_eq!(r("https://example.com"), "k: https://example.com");
        assert_eq!(r(&"x".repeat(200)), format!("k: {}", "x".repeat(200)));
        assert_eq!(r("one\ntwo"), "k: |-\n  one\n  two");
        assert_eq!(r("true"), "k: \"true\"");
        // Obsidian writes a date bare, and YAML reads it back as a string.
        assert_eq!(r("2026-10-18"), "k: 2026-10-18");
    }

    #[test]
    fn renders_lists_numbers_checkboxes_and_null() {
        let e = |v: Value| render_entry("k", &v, "\n");
        assert_eq!(
            e(Value::List(vec!["a".into(), "b".into()])),
            "k:\n  - a\n  - b"
        );
        assert_eq!(e(Value::List(vec!["only".into()])), "k:\n  - only");
        assert_eq!(e(Value::List(vec![])), "k: []");
        assert_eq!(
            e(Value::List(vec!["#a".into(), "b/c".into()])),
            "k:\n  - \"#a\"\n  - b/c"
        );
        assert_eq!(e(Value::Number(42.0)), "k: 42");
        assert_eq!(e(Value::Number(3.5)), "k: 3.5");
        assert_eq!(e(Value::Number(0.0)), "k: 0");
        assert_eq!(e(Value::Checkbox(true)), "k: true");
        assert_eq!(e(Value::Checkbox(false)), "k: false");
        assert_eq!(e(Value::Empty), "k:");
    }

    #[test]
    fn renders_with_the_blocks_line_ending() {
        assert_eq!(
            render_entry("k", &Value::List(vec!["a".into()]), "\r\n"),
            "k:\r\n  - a"
        );
    }

    #[test]
    fn a_rendered_entry_parses_back_to_the_same_value() {
        for value in [
            Value::Text("a: b".into()),
            Value::Text("  padded  ".into()),
            Value::Text("42".into()),
            Value::Text("#tag".into()),
            Value::Text("one\ntwo".into()),
            Value::Text("".into()),
            Value::Number(3.5),
            Value::Checkbox(false),
            Value::List(vec!["#a".into(), "b".into()]),
            Value::List(vec![]),
            Value::Empty,
        ] {
            let text = format!("---\n{}\n---\n", render_entry("k", &value, "\n"));
            let back = split(&text).get("k").unwrap().value.clone();
            assert_eq!(back, value, "round trip of {value:?} via {text:?}");
        }
    }

    // -------------------------------------------------------------- edits

    fn applied(text: &str, changes: &[Change]) -> String {
        let props = split(text);
        match plan(text, &props, changes) {
            Some(edits) => apply(text, &edits),
            None => text.to_string(),
        }
    }

    #[test]
    fn sets_an_existing_value_and_leaves_the_rest_byte_for_byte() {
        let text = "---\n# a comment\ntitle: Kept\ntags: [one, two]\ncount: 3\n---\nbody\n";
        let out = applied(
            text,
            &[Change::Set {
                key: "count".into(),
                value: Value::Number(4.0),
            }],
        );
        // The comment survives and the flow list keeps its bytes: this is
        // where Obsidian would have rewritten the whole block.
        assert_eq!(
            out,
            "---\n# a comment\ntitle: Kept\ntags: [one, two]\ncount: 4\n---\nbody\n"
        );
    }

    #[test]
    fn setting_a_flow_list_to_its_own_value_keeps_the_flow_form() {
        // The corpus has notes written `tags: [a, b]`; rewriting the same
        // value must not reflow them, or every such note would churn.
        let text = "---\ntags: [edge, blocks]\n---\nbody\n";
        let props = split(text);
        assert!(plan(
            text,
            &props,
            &[Change::Set {
                key: "tags".into(),
                value: Value::List(vec!["edge".into(), "blocks".into()])
            }]
        )
        .is_none());
    }

    #[test]
    fn setting_a_quoted_scalar_to_its_own_value_keeps_the_quotes() {
        let text = "---\nk: \"plain\"\n---\n";
        let props = split(text);
        assert!(plan(
            text,
            &props,
            &[Change::Set {
                key: "k".into(),
                value: Value::Text("plain".into())
            }]
        )
        .is_none());
    }

    #[test]
    fn a_noop_set_plans_nothing() {
        let text = "---\ntitle: Hi\n---\nbody\n";
        let props = split(text);
        assert!(plan(
            text,
            &props,
            &[Change::Set {
                key: "title".into(),
                value: Value::Text("Hi".into())
            }]
        )
        .is_none());
    }

    #[test]
    fn adds_a_property_after_the_last_entry() {
        assert_eq!(
            applied(
                "---\ntitle: Hi\n---\nbody\n",
                &[Change::Set {
                    key: "added".into(),
                    value: Value::Text("value".into())
                }]
            ),
            "---\ntitle: Hi\nadded: value\n---\nbody\n"
        );
    }

    #[test]
    fn adds_two_properties_in_order() {
        assert_eq!(
            applied(
                "---\na: 1\n---\n",
                &[
                    Change::Set {
                        key: "b".into(),
                        value: Value::Number(2.0)
                    },
                    Change::Set {
                        key: "c".into(),
                        value: Value::Number(3.0)
                    },
                ]
            ),
            "---\na: 1\nb: 2\nc: 3\n---\n"
        );
    }

    #[test]
    fn adds_into_an_empty_block() {
        assert_eq!(
            applied(
                "---\n---\nbody\n",
                &[Change::Set {
                    key: "added".into(),
                    value: Value::Text("value".into())
                }]
            ),
            "---\nadded: value\n---\nbody\n"
        );
    }

    #[test]
    fn a_multi_line_value_replaces_every_line_of_the_entry() {
        let text = "---\ntags:\n  - a\n  - b\nnext: 1\n---\n";
        assert_eq!(
            applied(
                text,
                &[Change::Set {
                    key: "tags".into(),
                    value: Value::List(vec!["a".into(), "b".into(), "c".into()])
                }]
            ),
            "---\ntags:\n  - a\n  - b\n  - c\nnext: 1\n---\n"
        );
    }

    #[test]
    fn a_flow_list_becomes_a_block_list_only_when_it_changes() {
        let text = "---\ntags: [a, b]\nnext: 1\n---\n";
        // Untouched: Obsidian would have rewritten it on any edit.
        assert_eq!(
            applied(
                text,
                &[Change::Set {
                    key: "next".into(),
                    value: Value::Number(2.0)
                }]
            ),
            "---\ntags: [a, b]\nnext: 2\n---\n"
        );
        // Changed: written in Obsidian's canonical block form.
        assert_eq!(
            applied(
                text,
                &[Change::Set {
                    key: "tags".into(),
                    value: Value::List(vec!["a".into(), "b".into(), "c".into()])
                }]
            ),
            "---\ntags:\n  - a\n  - b\n  - c\nnext: 1\n---\n"
        );
    }

    #[test]
    fn renames_a_key_in_place() {
        // Obsidian's processFrontMatter moves a renamed key to the end;
        // Silica keeps the entry where it is.
        assert_eq!(
            applied(
                "---\nold: v\nkeep: k\n---\n",
                &[Change::Rename {
                    from: "old".into(),
                    to: "new".into()
                }]
            ),
            "---\nnew: v\nkeep: k\n---\n"
        );
    }

    #[test]
    fn renaming_to_a_key_needing_quotes_quotes_it() {
        assert_eq!(
            applied(
                "---\na: v\n---\n",
                &[Change::Rename {
                    from: "a".into(),
                    to: "odd: key".into()
                }]
            ),
            "---\n\"odd: key\": v\n---\n"
        );
    }

    #[test]
    fn removes_one_property_with_its_newline() {
        assert_eq!(
            applied(
                "---\na: 1\nb: 2\n---\nbody\n",
                &[Change::Remove { key: "a".into() }]
            ),
            "---\nb: 2\n---\nbody\n"
        );
    }

    #[test]
    fn removing_a_multi_line_property_removes_all_of_it() {
        assert_eq!(
            applied(
                "---\ntags:\n  - a\n  - b\nb: 2\n---\n",
                &[Change::Remove { key: "tags".into() }]
            ),
            "---\nb: 2\n---\n"
        );
    }

    #[test]
    fn removing_the_last_property_removes_the_block() {
        // Obsidian 1.14.4: `---\na: 1\n---\nbody\n` becomes `body\n`.
        assert_eq!(
            applied(
                "---\na: 1\n---\nbody\n",
                &[Change::Remove { key: "a".into() }]
            ),
            "body\n"
        );
        assert_eq!(
            applied(
                "---\na: 1\nb: 2\n---\nbody\n",
                &[
                    Change::Remove { key: "a".into() },
                    Change::Remove { key: "b".into() }
                ]
            ),
            "body\n"
        );
    }

    /// A note that is *only* frontmatter becomes a zero-byte file when every
    /// property goes, which is what Obsidian 1.14.5 writes for
    /// `markdown:clear-metadata-properties` (see
    /// [[Specification#Clearing every property leaves a blank line]]): the
    /// block is the whole file, so removing it leaves nothing at all rather
    /// than a bare pair of fences or a stray newline.
    #[test]
    fn a_note_that_is_only_frontmatter_clears_to_a_zero_byte_file() {
        // The probe's own note.
        assert_eq!(
            applied(
                "---\nalpha: one\n---\n",
                &[Change::Remove {
                    key: "alpha".into()
                }]
            ),
            ""
        );
        // Several properties, cleared in one plan, and a block whose closing
        // fence ends the file without a trailing newline.
        assert_eq!(
            applied(
                "---\nalpha: one\nbeta: two\n---\n",
                &[
                    Change::Remove {
                        key: "alpha".into()
                    },
                    Change::Remove { key: "beta".into() }
                ]
            ),
            ""
        );
        assert_eq!(
            applied(
                "---\nalpha: one\n---",
                &[Change::Remove {
                    key: "alpha".into()
                }]
            ),
            ""
        );
    }

    #[test]
    fn removing_everything_but_adding_one_keeps_the_block() {
        assert_eq!(
            applied(
                "---\na: 1\n---\nbody\n",
                &[
                    Change::Remove { key: "a".into() },
                    Change::Set {
                        key: "b".into(),
                        value: Value::Number(2.0)
                    }
                ]
            ),
            "---\nb: 2\n---\nbody\n"
        );
    }

    #[test]
    fn edits_keep_the_files_line_endings() {
        let text = "---\r\na: 1\r\nb: 2\r\n---\r\nbody\r\n";
        // Obsidian writes LF inside a CRLF block, leaving mixed endings;
        // Silica keeps the file's.
        assert_eq!(
            applied(
                text,
                &[Change::Set {
                    key: "c".into(),
                    value: Value::List(vec!["x".into()])
                }]
            ),
            "---\r\na: 1\r\nb: 2\r\nc:\r\n  - x\r\n---\r\nbody\r\n"
        );
        assert_eq!(
            applied(text, &[Change::Remove { key: "a".into() }]),
            "---\r\nb: 2\r\n---\r\nbody\r\n"
        );
    }

    #[test]
    fn an_unknown_key_is_not_removed_or_renamed() {
        let text = "---\na: 1\n---\n";
        let props = split(text);
        assert!(plan(text, &props, &[Change::Remove { key: "z".into() }]).is_none());
        assert!(plan(
            text,
            &props,
            &[Change::Rename {
                from: "z".into(),
                to: "y".into()
            }]
        )
        .is_none());
    }

    #[test]
    fn a_new_block_goes_above_the_body() {
        assert_eq!(
            new_block("added", &Value::Text("value".into()), "\n"),
            "---\nadded: value\n---\n"
        );
    }

    #[test]
    fn parse_as_reads_what_a_field_commits() {
        assert_eq!(
            Value::parse_as("3.5", PropertyType::Number),
            Value::Number(3.5)
        );
        assert_eq!(Value::parse_as("", PropertyType::Number), Value::Empty);
        // Not a number: kept as typed, as Obsidian keeps it and flags the row.
        assert_eq!(
            Value::parse_as("abc", PropertyType::Number),
            Value::Text("abc".into())
        );
        assert_eq!(
            Value::parse_as("true", PropertyType::Checkbox),
            Value::Checkbox(true)
        );
        assert_eq!(
            Value::parse_as("a, b , ,c", PropertyType::Tags),
            Value::List(vec!["a".into(), "b".into(), "c".into()])
        );
        assert_eq!(Value::parse_as("", PropertyType::Text), Value::Empty);
    }

    #[test]
    fn type_names_round_trip_through_types_json() {
        for ty in [
            PropertyType::Text,
            PropertyType::MultiText,
            PropertyType::Number,
            PropertyType::Checkbox,
            PropertyType::Date,
            PropertyType::DateTime,
            PropertyType::Tags,
            PropertyType::Aliases,
        ] {
            assert_eq!(PropertyType::parse(ty.as_str()), Some(ty));
        }
        // Obsidian 1.14.4 has no `cssclasses` type widget.
        assert_eq!(PropertyType::parse("cssclasses"), None);
    }
}
