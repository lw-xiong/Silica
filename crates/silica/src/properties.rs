//! The properties block as both surfaces show it.
//!
//! `silica-formats::properties` owns the model, the serializer and the edit
//! planner; this module turns a parsed block into display rows (type icon,
//! key, value) and back into [`Change`]s when a field commits. The reading
//! view draws the rows read-only; Live Preview draws the same rows as an
//! interactive widget.
//!
//! A value is not markdown: Obsidian linkifies a wikilink and a tag inside a
//! property value but does not render emphasis or code there, so [`value_text`]
//! looks for those two constructs only.

use crate::reading::model::{InlineStyle, LinkTarget, RichText};
use silica_formats::properties::{Change, PropertyType, Value};
use std::ops::Range;

/// Fold id of the properties block, in the same space as the callout title
/// ids the views keep fold state under. Those are small counters from the
/// model builder, so a sentinel at the top of the range cannot collide.
pub const PROPERTIES_FOLD_ID: usize = usize::MAX;

/// First `RichText` id a property value may use. Property rows are built at
/// render time, after the model's own ids, so they start past any note's
/// counter instead of colliding with a paragraph's runs.
pub const PROPERTY_ID_BASE: usize = 1 << 40;

/// The glyph that stands for a property type. Obsidian uses Lucide icons
/// (`lucide-text`, `lucide-binary`, ...); Silica is clean room, so these are
/// the Unicode glyphs the rest of the app draws its affordances with.
pub fn type_glyph(ty: PropertyType) -> &'static str {
    match ty {
        PropertyType::Text => "T",
        PropertyType::MultiText => "☰",
        PropertyType::Number => "#",
        PropertyType::Checkbox => "✓",
        PropertyType::Date => "▦",
        PropertyType::DateTime => "◷",
        PropertyType::Tags => "⌗",
        PropertyType::Aliases => "⇄",
    }
}

/// One row of the properties table.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// Index of the row in the block, for GPUI element ids.
    pub id: usize,
    pub key: String,
    pub ty: PropertyType,
    pub value: Display,
    /// The value as editable text, which is what a scalar field shows and
    /// commits. A list joins with `, ` here only to be *shown*; a list is
    /// edited through [`Row::item_field`], one item at a time.
    pub edit_text: String,
    /// The items behind [`Display::Pills`], as they are stored and in the
    /// same order. A pill may show something else — a wikilink shows its
    /// label — so the raw item is what an item field edits.
    pub items: Vec<String>,
    /// The value does not fit its type (a `number` holding `abc`): Obsidian
    /// keeps what was typed and marks the row.
    pub mismatched: bool,
}

/// How a row draws its value.
#[derive(Debug, Clone, PartialEq)]
pub enum Display {
    /// Text, number, date or date-time: one line, links and tags clickable.
    Scalar(RichText),
    /// A list, tags or aliases: one pill per item.
    Pills(Vec<RichText>),
    Checkbox(bool),
    /// No value yet; the field shows its placeholder.
    Empty,
}

/// Builds the display rows for a parsed block. `type_of` gives each key its
/// type, so the caller decides how `types.json` and inference combine.
pub fn rows(
    entries: &[silica_formats::properties::Property],
    mut type_of: impl FnMut(&str) -> PropertyType,
    next_id: &mut usize,
) -> Vec<Row> {
    entries
        .iter()
        .enumerate()
        .map(|(ix, p)| row(ix, &p.key, &p.value, type_of(&p.key), next_id))
        .collect()
}

fn row(id: usize, key: &str, value: &Value, ty: PropertyType, next_id: &mut usize) -> Row {
    let edit_text = value.display();
    let mismatched = is_mismatched(value, ty);
    let mut items = Vec::new();
    let display = match (ty, value) {
        (_, Value::Empty) => Display::Empty,
        (PropertyType::Checkbox, Value::Checkbox(b)) => Display::Checkbox(*b),
        (ty, Value::List(stored)) if ty.is_list() || !stored.is_empty() => {
            items = stored.clone();
            Display::Pills(
                items
                    .iter()
                    .map(|item| value_text(item, ty, next_id))
                    .collect(),
            )
        }
        // A list type holding a scalar shows the scalar as one pill, which is
        // how `tags: a` reads.
        (ty, Value::Text(s)) if ty.is_list() => {
            items = vec![s.clone()];
            Display::Pills(vec![value_text(s, ty, next_id)])
        }
        (ty, value) => Display::Scalar(value_text(&value.display(), ty, next_id)),
    };
    Row {
        id,
        key: key.to_string(),
        ty,
        value: display,
        edit_text,
        items,
        mismatched,
    }
}

/// Whether the stored value disagrees with the row's type.
fn is_mismatched(value: &Value, ty: PropertyType) -> bool {
    match (ty, value) {
        (_, Value::Empty) => false,
        (PropertyType::Number, Value::Number(_)) => false,
        (PropertyType::Number, _) => true,
        (PropertyType::Checkbox, Value::Checkbox(_)) => false,
        (PropertyType::Checkbox, _) => true,
        (PropertyType::Date, Value::Text(s)) => !is_date_text(s),
        (PropertyType::DateTime, Value::Text(_)) => false,
        (ty, Value::List(_)) if ty.is_list() => false,
        (ty, _) if ty.is_list() => false,
        (_, Value::Other(_)) => true,
        _ => false,
    }
}

fn is_date_text(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter().filter(|c| c.is_ascii_digit()).count() == 8
}

/// A property value as rich text: a whole-value wikilink or `#tag` becomes
/// clickable, and a `tags` or `aliases` item is styled as a tag. Anything else
/// is plain — property values are not markdown.
pub fn value_text(text: &str, ty: PropertyType, next_id: &mut usize) -> RichText {
    let id = *next_id;
    *next_id += 1;
    let mut runs: Vec<(Range<usize>, InlineStyle)> = Vec::new();
    let mut links: Vec<(Range<usize>, LinkTarget)> = Vec::new();

    if let Some(body) = wikilink_body(text) {
        // `[[A]]` and `[[A|alias]]` render as the link, not as the brackets,
        // the way Obsidian shows a frontmatter link: the alias when there is
        // one, else the target without its subpath.
        let shown = wikilink_label(body);
        let whole = 0..shown.len();
        runs.push((
            whole.clone(),
            InlineStyle {
                link: true,
                ..Default::default()
            },
        ));
        links.push((whole, LinkTarget::Internal(body.to_string())));
        return RichText {
            id,
            text: shown,
            runs,
            links,
        };
    }
    if ty == PropertyType::Tags {
        // Every item of `tags` is a tag whether or not it is written with `#`.
        let whole = 0..text.len();
        runs.push((
            whole.clone(),
            InlineStyle {
                tag: true,
                ..Default::default()
            },
        ));
        links.push((
            whole,
            LinkTarget::Tag(text.trim_start_matches('#').to_string()),
        ));
    } else if let Some(tag) = text.strip_prefix('#') {
        let whole = 0..text.len();
        runs.push((
            whole.clone(),
            InlineStyle {
                tag: true,
                ..Default::default()
            },
        ));
        links.push((whole, LinkTarget::Tag(tag.to_string())));
    }
    RichText {
        id,
        text: text.to_string(),
        runs,
        links,
    }
}

/// What a wikilink value shows: its alias, or the target without a subpath.
fn wikilink_label(body: &str) -> String {
    if let Some((_, alias)) = body.split_once('|') {
        return alias.to_string();
    }
    body.split_once('#')
        .map(|(target, _)| target)
        .filter(|target| !target.is_empty())
        .unwrap_or(body)
        .to_string()
}

/// The link body of a value that is exactly one wikilink.
fn wikilink_body(text: &str) -> Option<&str> {
    let inner = text.strip_prefix("[[")?.strip_suffix("]]")?;
    if inner.is_empty() || inner.contains("[[") || inner.contains("]]") {
        return None;
    }
    Some(inner)
}

// --------------------------------------------------------------- editing

/// Which half of a row a field edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    Key,
    Value,
}

/// The row a field belongs to: one already in the block, or the empty row
/// `markdown:add-metadata-property` opens. Obsidian writes nothing for a new
/// row until it is named, which is why `New` carries no key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Existing(String),
    New,
}

/// The list a value field is editing one item of.
///
/// Obsidian edits a list property as pills, one editable item each, so a
/// field holds a single item and the rest of the list rides along with it. A
/// commit then puts the typed text back in its place instead of re-splitting
/// a joined string, which is the only way an item that contains a comma
/// survives the round trip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListEdit {
    /// The list as stored, before this edit.
    pub items: Vec<String>,
    /// Which item the field holds. `items.len()` is a fresh item being
    /// appended, which is what a click past the last pill opens.
    pub index: usize,
}

impl ListEdit {
    /// The list with `text` back in the place of the item being edited. An
    /// item typed empty is dropped, the way a pill's remove control drops it;
    /// removing the last item leaves the key with an empty value rather than
    /// a list, which is what Obsidian writes for the gesture (see
    /// [[Specification#Property pills, four open gestures]]):
    /// `tags:\n  - one\n` emptied a pill at a time ends as `tags:\n`, not
    /// `tags: []`, and the key is not deleted. The flow `[]` stays the form
    /// [`render_entry`] gives a value that *is* an empty list, which is how a
    /// vault's own `k: []` survives an edit to another row.
    fn value(&self, text: &str) -> Value {
        let mut items = self.items.clone();
        let typed = text.trim();
        if typed.is_empty() {
            if self.index < items.len() {
                items.remove(self.index);
            }
        } else if let Some(slot) = items.get_mut(self.index) {
            *slot = typed.to_string();
        } else {
            items.push(typed.to_string());
        }
        if items.is_empty() {
            return Value::Empty;
        }
        Value::List(items)
    }
}

/// One properties field being typed into. The editor owns the text while a
/// field is open and turns it into a [`Change`] on commit, so the document
/// only changes once per field rather than on every keystroke.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub target: Target,
    pub part: Part,
    pub text: String,
    /// Byte offset of the caret in `text`.
    pub cursor: usize,
    /// The text the field opened with. A value nobody typed into writes
    /// nothing: re-reading it would serialize the value again, and
    /// `aliases: [One, "Two, with comma"]` does not survive that round trip.
    was: String,
    /// Set when the field holds one item of a list-shaped value: `text` is
    /// then that item alone, never the whole value.
    pub list: Option<ListEdit>,
}

impl Field {
    pub fn new(target: Target, part: Part, text: impl Into<String>) -> Field {
        let text = text.into();
        let cursor = text.len();
        Field {
            target,
            part,
            was: text.clone(),
            text,
            cursor,
            list: None,
        }
    }

    pub fn key(&self) -> Option<&str> {
        match &self.target {
            Target::Existing(k) => Some(k),
            Target::New => None,
        }
    }

    pub fn insert(&mut self, s: &str) {
        // A newline would break the row; property fields are single line.
        let clean: String = s.chars().filter(|c| *c != '\n' && *c != '\r').collect();
        self.text.insert_str(self.cursor, &clean);
        self.cursor += clean.len();
    }

    /// Backspace. In the trailing input of a list value this is not a
    /// character delete at all but the first or second half of Obsidian's
    /// two-press pill gesture, so the press reports what it meant and the
    /// caller — which owns the selected pill and can write the file — acts
    /// on it. Everywhere else it takes the character before the caret and
    /// reports [`PillStep::Text`]. `selected` is the pill an earlier press
    /// selected, if any.
    ///
    /// See [`pill_step`] for the gesture itself.
    #[must_use]
    pub fn backspace(&mut self, selected: Option<usize>) -> PillStep {
        match pill_step(self, selected) {
            PillStep::Text => {
                if let Some(prev) = self.prev_boundary() {
                    self.text.replace_range(prev..self.cursor, "");
                    self.cursor = prev;
                }
                PillStep::Text
            }
            step => step,
        }
    }

    pub fn delete(&mut self) {
        if let Some(next) = self.next_boundary() {
            self.text.replace_range(self.cursor..next, "");
        }
    }

    pub fn left(&mut self) {
        if let Some(prev) = self.prev_boundary() {
            self.cursor = prev;
        }
    }

    pub fn right(&mut self) {
        if let Some(next) = self.next_boundary() {
            self.cursor = next;
        }
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.text.len();
    }

    fn prev_boundary(&self) -> Option<usize> {
        if self.cursor == 0 {
            return None;
        }
        Some(
            self.text[..self.cursor]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0),
        )
    }

    fn next_boundary(&self) -> Option<usize> {
        if self.cursor >= self.text.len() {
            return None;
        }
        let rest = &self.text[self.cursor..];
        rest.char_indices()
            .nth(1)
            .map(|(i, _)| self.cursor + i)
            .or(Some(self.text.len()))
    }
}

// ------------------------------------------------------------ list items
//
// Obsidian edits a list property one pill at a time, which is the only way an
// item containing a comma survives: a value click lands in the empty input
// after the pills ([`value_field`]) and what is typed there becomes one more
// item, while a click on a pill opens that item alone and its own remove
// control ([`remove_item`]) drops it.

impl Field {
    /// A field on one item of a list-shaped value. An `index` past the last
    /// item opens an empty field that appends.
    pub fn item(key: impl Into<String>, items: Vec<String>, index: usize) -> Field {
        let text = items.get(index).cloned().unwrap_or_default();
        let cursor = text.len();
        Field {
            target: Target::Existing(key.into()),
            part: Part::Value,
            was: text.clone(),
            text,
            cursor,
            list: Some(ListEdit { items, index }),
        }
    }

    /// Which item of a list-shaped value this field holds, if it holds one.
    pub fn item_index(&self) -> Option<usize> {
        self.list.as_ref().map(|list| list.index)
    }
}

impl Field {
    /// The item of `row`'s list this field is typing into, if it is typing
    /// into one of them. The value cell draws the field in that item's
    /// place, so a field on another row, on a key, or on a whole value
    /// belongs nowhere among these pills.
    pub fn item_of(&self, row: &Row) -> Option<usize> {
        if self.part != Part::Value || self.key() != Some(row.key.as_str()) {
            return None;
        }
        self.item_index()
    }
}

impl Row {
    /// Whether the value is edited one item at a time: a list type, or any
    /// type whose stored value is a sequence.
    pub fn is_list_valued(&self) -> bool {
        self.ty.is_list() || matches!(self.value, Display::Pills(_))
    }

    /// Whether the value cell has nothing of its own to draw. An explicitly
    /// empty list (`aliases: []`) is the same state as a key with no value
    /// at all, and the cell draws the one placeholder for both: a cell
    /// holding nothing is a cell with no height, and a cell with no height
    /// takes no click, so the first item could never be typed.
    pub fn is_blank(&self) -> bool {
        match &self.value {
            Display::Empty => true,
            Display::Pills(items) => items.is_empty(),
            _ => false,
        }
    }

    /// A field on the item at `index`, or on a fresh item at the end when
    /// `index` is past the last one.
    pub fn item_field(&self, index: usize) -> Field {
        Field::item(self.key.clone(), self.items.clone(), index)
    }

    /// A field on a fresh item at the end, which is where a click on the
    /// value of a list row lands: Obsidian keeps an empty input after the
    /// pills and focuses that one.
    pub fn append_field(&self) -> Field {
        self.item_field(self.items.len())
    }
}

/// The field a click on a row's value opens: the empty input after the pills
/// for a list-shaped row, the whole value for a scalar. Obsidian puts the
/// caret in that input, and what is typed there is one item however many
/// commas it holds.
pub fn value_field(row: &Row) -> Field {
    if row.is_list_valued() {
        row.append_field()
    } else {
        Field::new(
            Target::Existing(row.key.clone()),
            Part::Value,
            row.edit_text.clone(),
        )
    }
}

/// What a Backspace in a list value means.
///
/// Obsidian takes **two** presses to remove a pill: with the trailing input
/// focused and empty the first moves focus to the last pill and changes
/// nothing, and the second removes that pill and writes the file (see
/// [[Specification#The property pills]]). Repeating walks backwards a pill at
/// a time, and once the list is empty focus is back in the input and further
/// presses do nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PillStep {
    /// Not the gesture: the press belongs to the field's own text, and
    /// [`Field::backspace`] has taken the character before the caret.
    Text,
    /// Focus moves to the pill at this index. Nothing is written — this is
    /// the first of the two presses.
    Select(usize),
    /// The selected pill goes, through the same [`remove_item`] change its
    /// own remove control makes. This is the second press.
    Remove(usize),
    /// Nothing left to select: the list is empty and the input has focus, so
    /// the press does nothing at all.
    Nothing,
}

/// What a Backspace means for `field`, given the pill an earlier press
/// selected. The gesture belongs to the trailing input alone: a caret with
/// text before it, and a field opened on an existing pill rather than on the
/// input after them, both take the character.
pub fn pill_step(field: &Field, selected: Option<usize>) -> PillStep {
    let Some(list) = field.list.as_ref() else {
        return PillStep::Text;
    };
    // The trailing input is the item past the last one, and it has to be
    // empty: a half-typed item's Backspace is a character delete, and so is
    // one inside a pill the reader opened to edit.
    if field.cursor > 0 || !field.text.is_empty() || list.index < list.items.len() {
        return PillStep::Text;
    }
    match selected {
        // A pill is already selected, so this is the second press.
        Some(ix) if ix < list.items.len() => PillStep::Remove(ix),
        // The first press selects the last pill, if there is one.
        _ => match list.items.len().checked_sub(1) {
            Some(last) => PillStep::Select(last),
            None => PillStep::Nothing,
        },
    }
}

/// The change a pill's remove control makes: the row's list without the item
/// at `index`. `None` when the row has no such item.
pub fn remove_item(row: &Row, index: usize) -> Option<Change> {
    (index < row.items.len()).then(|| Change::Set {
        key: row.key.clone(),
        value: ListEdit {
            items: row.items.clone(),
            index,
        }
        .value(""),
    })
}

/// The change a committed field makes, or `None` when it makes none.
///
/// An existing key typed empty removes the property, which is how Obsidian's
/// key field behaves; a new row typed empty is simply abandoned, because
/// nothing was written for it in the first place.
pub fn commit(field: &Field, ty: PropertyType) -> Option<Change> {
    match (&field.target, field.part) {
        (Target::New, _) => {
            let key = field.text.trim();
            (!key.is_empty()).then(|| Change::Set {
                key: key.to_string(),
                value: Value::Empty,
            })
        }
        (Target::Existing(key), Part::Key) => {
            let to = field.text.trim();
            if to.is_empty() {
                Some(Change::Remove { key: key.clone() })
            } else if to == key {
                None
            } else {
                Some(Change::Rename {
                    from: key.clone(),
                    to: to.to_string(),
                })
            }
        }
        // Opening a value and leaving it is not an edit. The round trip
        // through `display`/`parse_as` is lossy for a list -- it splits an
        // item that contains a comma in two -- so an untouched field must not
        // take it.
        (Target::Existing(_), Part::Value) if field.text == field.was => None,
        (Target::Existing(key), Part::Value) => match &field.list {
            // A fresh item left untyped writes nothing, so that opening the
            // value of `tags: a` and clicking away does not rewrite it as a
            // sequence.
            Some(list) if list.index >= list.items.len() && field.text.trim().is_empty() => None,
            // The text is one item, so it goes back whole. Reading it as `ty`
            // would split an item that contains a comma in two.
            Some(list) => Some(Change::Set {
                key: key.clone(),
                value: list.value(&field.text),
            }),
            None => Some(Change::Set {
                key: key.clone(),
                value: Value::parse_as(&field.text, ty),
            }),
        },
    }
}

/// Changing a row's type keeps the text the user already wrote and reads it
/// as the new type, so `3` typed as text becomes the number 3 and a list
/// becomes `a, b` as text.
pub fn retype(row: &Row, ty: PropertyType) -> Change {
    // Between two list types the items carry over as they are; reading the
    // joined text back would split an item that contains a comma.
    let value = if ty.is_list() && !row.items.is_empty() {
        Value::List(row.items.clone())
    } else {
        Value::parse_as(&row.edit_text, ty)
    };
    Change::Set {
        key: row.key.clone(),
        value,
    }
}

/// Every property of the block removed, for `markdown:clear-metadata-properties`.
/// [`plan`](silica_formats::properties::plan) turns a full set of removals into
/// one edit that takes the fences with it.
pub fn clear_all(entries: &[silica_formats::properties::Property]) -> Vec<Change> {
    entries
        .iter()
        .map(|p| Change::Remove { key: p.key.clone() })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use silica_formats::config::PropertyTypes;
    use silica_formats::properties::{type_of, Property};

    fn prop(key: &str, value: Value) -> Property {
        Property {
            key: key.to_string(),
            value,
            span: 0..0,
            key_span: 0..0,
        }
    }

    #[test]
    fn a_scalar_row_carries_its_edit_text() {
        let mut id = 0;
        let rows = rows(
            &[prop("title", Value::Text("Hi".into()))],
            |_| PropertyType::Text,
            &mut id,
        );
        assert_eq!(rows[0].key, "title");
        assert_eq!(rows[0].edit_text, "Hi");
        assert!(!rows[0].mismatched);
        assert!(matches!(&rows[0].value, Display::Scalar(t) if t.text == "Hi"));
    }

    #[test]
    fn a_list_row_becomes_one_pill_per_item() {
        let mut id = 0;
        let rows = rows(
            &[prop("tags", Value::List(vec!["one".into(), "two".into()]))],
            |_| PropertyType::Tags,
            &mut id,
        );
        let Display::Pills(pills) = &rows[0].value else {
            panic!("expected pills, got {:?}", rows[0].value);
        };
        assert_eq!(pills.len(), 2);
        assert_eq!(pills[0].text, "one");
        // Every tag item is clickable even written without `#`.
        assert_eq!(pills[0].links[0].1, LinkTarget::Tag("one".into()));
        // The items ride along as stored, which is what an item field edits.
        assert_eq!(rows[0].items, ["one", "two"]);
        // The edit field joins them the way a field shows a list.
        assert_eq!(rows[0].edit_text, "one, two");
    }

    #[test]
    fn a_hash_in_a_tag_item_is_not_doubled() {
        let mut id = 0;
        let t = value_text("#a/b", PropertyType::Tags, &mut id);
        assert_eq!(t.links[0].1, LinkTarget::Tag("a/b".into()));
    }

    #[test]
    fn a_wikilink_value_shows_the_link_not_the_brackets() {
        let mut id = 0;
        let t = value_text("[[Target]]", PropertyType::Text, &mut id);
        assert_eq!(t.text, "Target");
        assert_eq!(t.links.len(), 1);
        assert_eq!(t.links[0].1, LinkTarget::Internal("Target".into()));
        assert_eq!(t.links[0].0, 0..t.text.len());
        assert!(t.runs[0].1.link);
    }

    #[test]
    fn a_wikilink_shows_its_alias_and_drops_a_subpath() {
        let mut id = 0;
        let aliased = value_text("[[Target|shown]]", PropertyType::Text, &mut id);
        assert_eq!(aliased.text, "shown");
        // The whole body still resolves the link, subpath included.
        assert_eq!(
            aliased.links[0].1,
            LinkTarget::Internal("Target|shown".into())
        );
        let heading = value_text("[[Target#Section]]", PropertyType::Text, &mut id);
        assert_eq!(heading.text, "Target");
        assert_eq!(
            heading.links[0].1,
            LinkTarget::Internal("Target#Section".into())
        );
        // A self link to a heading keeps its text, having no target.
        let own = value_text("[[#Section]]", PropertyType::Text, &mut id);
        assert_eq!(own.text, "#Section");
    }

    #[test]
    fn a_plain_value_has_no_runs() {
        let mut id = 0;
        // Emphasis is not rendered in a property value.
        let t = value_text("*not italic* and `not code`", PropertyType::Text, &mut id);
        assert!(t.runs.is_empty());
        assert!(t.links.is_empty());
    }

    #[test]
    fn a_tag_in_a_text_value_is_still_clickable() {
        let mut id = 0;
        let t = value_text("#project", PropertyType::Text, &mut id);
        assert_eq!(t.links[0].1, LinkTarget::Tag("project".into()));
    }

    #[test]
    fn a_checkbox_row_draws_a_box_and_an_empty_value_draws_nothing() {
        let mut id = 0;
        let rows = rows(
            &[
                prop("done", Value::Checkbox(true)),
                prop("blank", Value::Empty),
            ],
            |k| {
                if k == "done" {
                    PropertyType::Checkbox
                } else {
                    PropertyType::Text
                }
            },
            &mut id,
        );
        assert_eq!(rows[0].value, Display::Checkbox(true));
        assert_eq!(rows[1].value, Display::Empty);
        assert_eq!(rows[1].edit_text, "");
    }

    #[test]
    fn a_value_that_does_not_fit_its_type_is_marked() {
        let mut id = 0;
        let rows = rows(
            &[prop("count", Value::Text("abc".into()))],
            |_| PropertyType::Number,
            &mut id,
        );
        assert!(rows[0].mismatched);
        // The text is kept as typed rather than thrown away.
        assert_eq!(rows[0].edit_text, "abc");
    }

    #[test]
    fn a_nested_mapping_is_shown_but_marked() {
        let mut id = 0;
        let rows = rows(
            &[prop("nested", Value::Other("nested:\n  k: v".into()))],
            |_| PropertyType::Text,
            &mut id,
        );
        assert!(rows[0].mismatched);
    }

    #[test]
    fn every_rich_text_gets_a_distinct_id() {
        let mut id = 0;
        let rows = rows(
            &[
                prop("a", Value::Text("x".into())),
                prop("b", Value::List(vec!["p".into(), "q".into()])),
            ],
            |k| {
                if k == "b" {
                    PropertyType::MultiText
                } else {
                    PropertyType::Text
                }
            },
            &mut id,
        );
        let mut seen = Vec::new();
        for r in &rows {
            match &r.value {
                Display::Scalar(t) => seen.push(t.id),
                Display::Pills(ps) => seen.extend(ps.iter().map(|p| p.id)),
                _ => {}
            }
        }
        let mut sorted = seen.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), seen.len(), "ids repeated: {seen:?}");
    }

    // ------------------------------------------------------------ editing

    #[test]
    fn a_field_types_moves_and_deletes_by_character() {
        let mut f = Field::new(Target::Existing("k".into()), Part::Value, "ab");
        assert_eq!(f.cursor, 2);
        f.insert("c");
        assert_eq!((f.text.as_str(), f.cursor), ("abc", 3));
        f.left();
        f.insert("X");
        assert_eq!((f.text.as_str(), f.cursor), ("abXc", 3));
        assert_eq!(f.backspace(None), PillStep::Text);
        assert_eq!((f.text.as_str(), f.cursor), ("abc", 2));
        f.delete();
        assert_eq!((f.text.as_str(), f.cursor), ("ab", 2));
        f.home();
        assert_eq!(f.cursor, 0);
        // A scalar value has no pills behind it, so there is nothing for the
        // caret to step out into: the press is still a character delete, and
        // with nothing before the caret it takes nothing.
        assert_eq!(f.backspace(None), PillStep::Text);
        assert_eq!(
            f.text, "ab",
            "backspace at the start of a scalar field does nothing"
        );
        f.end();
        f.delete();
        assert_eq!(f.text, "ab", "delete at the end does nothing");
    }

    #[test]
    fn a_field_steps_over_whole_characters() {
        let mut f = Field::new(Target::New, Part::Key, "Ä✓");
        f.left();
        assert_eq!(f.cursor, 2, "caret sits before the ✓, not inside it");
        assert_eq!(f.backspace(None), PillStep::Text);
        assert_eq!(f.text, "✓");
    }

    #[test]
    fn a_field_never_takes_a_newline() {
        let mut f = Field::new(Target::New, Part::Key, "");
        f.insert("a\nb\r\nc");
        assert_eq!(f.text, "abc");
    }

    #[test]
    fn committing_a_value_reads_it_as_the_rows_type() {
        // Each field is typed into: a value nobody changed commits nothing.
        let mut f = Field::new(Target::Existing("n".into()), Part::Value, "4");
        f.insert("2");
        assert_eq!(
            commit(&f, PropertyType::Number),
            Some(Change::Set {
                key: "n".into(),
                value: Value::Number(42.0)
            })
        );
        assert_eq!(
            commit(&f, PropertyType::Text),
            Some(Change::Set {
                key: "n".into(),
                value: Value::Text("42".into())
            })
        );
        let mut list = Field::new(Target::Existing("tags".into()), Part::Value, "a,");
        list.insert(" b");
        assert_eq!(
            commit(&list, PropertyType::Tags),
            Some(Change::Set {
                key: "tags".into(),
                value: Value::List(vec!["a".into(), "b".into()])
            })
        );
    }

    #[test]
    fn committing_a_key_renames_and_an_empty_key_removes() {
        let same = Field::new(Target::Existing("k".into()), Part::Key, "k");
        assert_eq!(commit(&same, PropertyType::Text), None);
        let renamed = Field::new(Target::Existing("k".into()), Part::Key, " new ");
        assert_eq!(
            commit(&renamed, PropertyType::Text),
            Some(Change::Rename {
                from: "k".into(),
                to: "new".into()
            })
        );
        let cleared = Field::new(Target::Existing("k".into()), Part::Key, "   ");
        assert_eq!(
            commit(&cleared, PropertyType::Text),
            Some(Change::Remove { key: "k".into() })
        );
    }

    #[test]
    fn a_new_row_writes_nothing_until_it_is_named() {
        let blank = Field::new(Target::New, Part::Key, "");
        assert_eq!(commit(&blank, PropertyType::Text), None);
        let named = Field::new(Target::New, Part::Key, "fresh");
        assert_eq!(
            commit(&named, PropertyType::Text),
            Some(Change::Set {
                key: "fresh".into(),
                value: Value::Empty
            })
        );
    }

    #[test]
    fn retyping_rereads_the_text_the_row_already_shows() {
        let mut id = 0;
        let scalar = rows(
            &[prop("n", Value::Text("3".into()))],
            |_| PropertyType::Text,
            &mut id,
        );
        assert_eq!(
            retype(&scalar[0], PropertyType::Number),
            Change::Set {
                key: "n".into(),
                value: Value::Number(3.0)
            }
        );
        let mut id = 0;
        let listed = rows(
            &[prop("l", Value::List(vec!["a".into(), "b".into()]))],
            |_| PropertyType::MultiText,
            &mut id,
        );
        assert_eq!(
            retype(&listed[0], PropertyType::Text),
            Change::Set {
                key: "l".into(),
                value: Value::Text("a, b".into())
            }
        );
    }

    #[test]
    fn clearing_removes_every_property() {
        let entries = [
            prop("a", Value::Number(1.0)),
            prop("b", Value::Text("x".into())),
        ];
        assert_eq!(
            clear_all(&entries),
            vec![
                Change::Remove { key: "a".into() },
                Change::Remove { key: "b".into() }
            ]
        );
    }

    // -------------------------------------------------------- list items

    /// The ranges `silica_markdown::extract_frontmatter` would report for a
    /// note that starts with a properties block.
    fn block_ranges(text: &str) -> (std::ops::Range<usize>, std::ops::Range<usize>) {
        let close = text.find("\n---\n").expect("closing fence");
        (4..close + 1, 0..close + 5)
    }

    fn parse_block(text: &str) -> silica_formats::properties::Properties {
        let (yaml, block) = block_ranges(text);
        silica_formats::properties::parse(text, yaml, block)
    }

    fn list_row(text: &str, ty: PropertyType) -> Row {
        let mut id = 0;
        rows(&parse_block(text).entries, |_| ty, &mut id)
            .pop()
            .expect("one row")
    }

    #[test]
    fn an_item_field_holds_one_item_and_puts_it_back_whole() {
        let row = list_row(
            "---\naliases:\n  - One\n  - Smith, John\n---\n\nbody\n",
            PropertyType::Aliases,
        );
        assert!(row.is_list_valued());
        let mut f = row.item_field(1);
        assert_eq!(f.text, "Smith, John", "the field holds the item alone");
        assert_eq!(f.item_index(), Some(1));
        f.insert(" Jr.");
        assert_eq!(
            commit(&f, PropertyType::Aliases),
            Some(Change::Set {
                key: "aliases".into(),
                // The comma stays inside the item instead of splitting it.
                value: Value::List(vec!["One".into(), "Smith, John Jr.".into()])
            })
        );
    }

    #[test]
    fn an_item_field_belongs_to_the_value_cell_of_its_own_row() {
        let row = list_row(
            "---\naliases:\n  - One\n  - Two\ntags:\n  - a\n---\n\nbody\n",
            PropertyType::Aliases,
        );
        assert_eq!(row.key, "tags", "the last row of the block");
        // The pill the field stands in for, and the empty input after the
        // last pill, are the two places the value cell draws it.
        assert_eq!(row.item_field(0).item_of(&row), Some(0));
        assert_eq!(row.append_field().item_of(&row), Some(1));
        // A field on another row, on a key, or on the whole value belongs
        // nowhere among these pills.
        let other = Field::item("aliases", vec!["One".into()], 0);
        assert_eq!(other.item_of(&row), None);
        let key = Field::new(Target::Existing("tags".into()), Part::Key, "tags");
        assert_eq!(key.item_of(&row), None);
        let whole = Field::new(Target::Existing("tags".into()), Part::Value, "a");
        assert_eq!(whole.item_of(&row), None);
        assert_eq!(Field::new(Target::New, Part::Key, "").item_of(&row), None);
    }

    #[test]
    fn a_typed_comma_survives_the_write_and_reads_back_as_one_item() {
        let text = "---\naliases:\n  - One\n---\n\nbody\n";
        let props = parse_block(text);
        let mut id = 0;
        let row = rows(&props.entries, |_| PropertyType::Aliases, &mut id)
            .pop()
            .unwrap();
        let mut f = row.append_field();
        f.insert("Smith, John");
        let change = commit(&f, PropertyType::Aliases).expect("a change");
        let edits = silica_formats::properties::plan(text, &props, &[change]).expect("edits");
        let out = silica_formats::properties::apply(text, &edits);
        assert_eq!(
            out,
            "---\naliases:\n  - One\n  - Smith, John\n---\n\nbody\n"
        );
        // What the writer wrote reads back as the one item that was typed.
        assert_eq!(
            parse_block(&out).entries[0].value,
            Value::List(vec!["One".into(), "Smith, John".into()])
        );
    }

    #[test]
    fn an_item_typed_empty_is_dropped_and_an_untyped_one_writes_nothing() {
        let row = list_row(
            "---\ntags:\n  - one\n  - two\n---\n\nbody\n",
            PropertyType::Tags,
        );
        let mut f = row.item_field(0);
        f.home();
        for _ in 0..3 {
            f.delete();
        }
        assert_eq!(
            commit(&f, PropertyType::Tags),
            Some(Change::Set {
                key: "tags".into(),
                value: Value::List(vec!["two".into()])
            })
        );
        // Opening the empty input after the pills and leaving it is not an
        // edit, so a scalar `tags: a` is not rewritten as a sequence.
        let scalar = list_row("---\ntags: a\n---\n\nbody\n", PropertyType::Tags);
        assert_eq!(scalar.items, ["a"]);
        assert_eq!(commit(&scalar.append_field(), PropertyType::Tags), None);
    }

    #[test]
    fn a_pills_remove_control_drops_that_item_only() {
        let row = list_row(
            "---\ntags:\n  - one\n  - Smith, John\n  - three\n---\n\nbody\n",
            PropertyType::Tags,
        );
        assert_eq!(
            remove_item(&row, 1),
            Some(Change::Set {
                key: "tags".into(),
                value: Value::List(vec!["one".into(), "three".into()])
            })
        );
        // The last item removed leaves the key with an empty value, which is
        // what Obsidian writes for this gesture -- `tags:`, not `tags: []`.
        let single = list_row("---\ntags:\n  - one\n---\n\nbody\n", PropertyType::Tags);
        assert_eq!(
            remove_item(&single, 0),
            Some(Change::Set {
                key: "tags".into(),
                value: Value::Empty
            })
        );
        assert_eq!(remove_item(&single, 1), None);
    }

    /// `changes` planned against the note and applied to its bytes, which is
    /// what the editor does with a committed field or a removed pill.
    fn apply(text: &mut String, changes: &[Change]) {
        let block = parse_block(text);
        let mut edits = silica_formats::properties::plan(text, &block, changes).expect("a plan");
        edits.sort_by_key(|e| e.start);
        for e in edits.into_iter().rev() {
            text.replace_range(e.start..e.end, &e.text);
        }
    }

    /// One Backspace in the trailing input of `ty`'s list, driven the way the
    /// editor drives it: the step decides, a removal goes through
    /// [`remove_item`] and the planner, and the input is rebuilt from the row
    /// as it then stands — the old field's snapshot of the items is stale
    /// and committing it would write the removed item back.
    fn press_backspace(
        text: &mut String,
        ty: PropertyType,
        selected: &mut Option<usize>,
    ) -> PillStep {
        let row = list_row(text, ty);
        let mut field = row.append_field();
        let step = field.backspace(*selected);
        match step {
            PillStep::Select(ix) => *selected = Some(ix),
            PillStep::Remove(ix) => {
                *selected = None;
                let change = remove_item(&row, ix).expect("the selected pill is in the list");
                apply(text, &[change]);
            }
            PillStep::Text | PillStep::Nothing => *selected = None,
        }
        step
    }

    /// Obsidian's two-press Backspace: the first press selects the last pill
    /// and writes nothing, the second removes it, and repeating walks
    /// backwards a pill at a time.
    #[test]
    fn a_backspace_in_the_input_selects_a_pill_before_it_removes_it() {
        let start = "---\ntags:\n  - one\n  - two\n---\n\nbody\n";
        let mut text = start.to_string();
        let mut selected = None;

        assert_eq!(
            press_backspace(&mut text, PropertyType::Tags, &mut selected),
            PillStep::Select(1),
            "the first press takes the last pill"
        );
        assert_eq!(selected, Some(1));
        assert_eq!(text, start, "and changes nothing in the file");

        assert_eq!(
            press_backspace(&mut text, PropertyType::Tags, &mut selected),
            PillStep::Remove(1),
            "the second press removes the selected pill"
        );
        assert_eq!(text, "---\ntags:\n  - one\n---\n\nbody\n");
        assert_eq!(selected, None, "focus is back in the input");

        // Walking backwards: the same two presses take the pill before it.
        assert_eq!(
            press_backspace(&mut text, PropertyType::Tags, &mut selected),
            PillStep::Select(0)
        );
        assert_eq!(text, "---\ntags:\n  - one\n---\n\nbody\n");
        assert_eq!(
            press_backspace(&mut text, PropertyType::Tags, &mut selected),
            PillStep::Remove(0)
        );
    }

    /// The end of the walk: the key stays, with an empty value — `tags:`,
    /// not `tags: []` and not a removed key — and further presses do
    /// nothing at all.
    #[test]
    fn the_last_pill_removed_leaves_the_key_with_an_empty_value() {
        let mut text = "---\ntags:\n  - one\n---\n\nbody\n".to_string();
        let mut selected = None;
        assert_eq!(
            press_backspace(&mut text, PropertyType::Tags, &mut selected),
            PillStep::Select(0)
        );
        assert_eq!(
            press_backspace(&mut text, PropertyType::Tags, &mut selected),
            PillStep::Remove(0)
        );
        assert_eq!(text, "---\ntags:\n---\n\nbody\n");
        let block = parse_block(&text);
        assert_eq!(block.entries.len(), 1, "the key is not deleted");
        assert_eq!(block.entries[0].key, "tags");
        assert_eq!(block.entries[0].value, Value::Empty);

        // The list is empty, so the input keeps focus and the press does
        // nothing however often it is pressed.
        for _ in 0..3 {
            assert_eq!(
                press_backspace(&mut text, PropertyType::Tags, &mut selected),
                PillStep::Nothing
            );
            assert_eq!(selected, None);
            assert_eq!(text, "---\ntags:\n---\n\nbody\n");
        }
    }

    /// The gesture belongs to the empty trailing input alone: with something
    /// typed into it, and inside a pill opened for editing, Backspace is the
    /// character delete it always was.
    #[test]
    fn a_backspace_with_text_in_hand_is_still_a_character_delete() {
        let row = list_row("---\ntags:\n  - one\n---\n\nbody\n", PropertyType::Tags);

        let mut typed = row.append_field();
        typed.insert("two");
        assert_eq!(typed.backspace(None), PillStep::Text);
        assert_eq!(typed.text, "tw");
        assert_eq!(pill_step(&typed, None), PillStep::Text);

        // A pill the reader opened holds its item, and emptying it is not
        // the input's gesture: the spec settles the input only.
        let mut pill = row.item_field(0);
        for _ in 0..3 {
            assert_eq!(pill.backspace(None), PillStep::Text);
        }
        assert_eq!(pill.text, "");
        assert_eq!(pill.backspace(None), PillStep::Text);
    }

    #[test]
    fn a_whole_value_field_still_reads_a_list_as_typed() {
        // Without an item, a value field is the scalar path it always was.
        let mut f = Field::new(Target::Existing("tags".into()), Part::Value, "a");
        assert_eq!(f.item_index(), None);
        f.insert(", b");
        assert_eq!(
            commit(&f, PropertyType::Tags),
            Some(Change::Set {
                key: "tags".into(),
                value: Value::List(vec!["a".into(), "b".into()])
            })
        );
    }

    #[test]
    fn a_value_click_on_a_list_opens_the_input_after_the_pills() {
        let row = list_row(
            "---\naliases:\n  - One\n  - Two\n---\n\nbody\n",
            PropertyType::Aliases,
        );
        let mut f = value_field(&row);
        assert_eq!(
            f.text, "",
            "the caret lands in the empty input, not in the pills"
        );
        assert_eq!(f.item_index(), Some(2));
        f.insert("Smith, John");
        assert_eq!(
            commit(&f, PropertyType::Aliases),
            Some(Change::Set {
                key: "aliases".into(),
                value: Value::List(vec!["One".into(), "Two".into(), "Smith, John".into()])
            })
        );
        // A scalar row opens on its whole value, as it always did.
        let scalar = list_row("---\ntitle: Hi\n---\n\nbody\n", PropertyType::Text);
        let s = value_field(&scalar);
        assert_eq!(s.text, "Hi");
        assert_eq!(s.item_index(), None);
    }

    /// A list row with nothing in it yet is still edited one item at a
    /// time: the cell has no pills to click, so Live Preview opens the
    /// input that follows them and the first thing typed becomes item 0,
    /// commas and all.
    #[test]
    fn an_empty_list_row_opens_an_item_field_not_a_whole_value() {
        let row = list_row("---\ntags:\n---\n\nbody\n", PropertyType::Tags);
        assert!(row.items.is_empty());
        assert!(matches!(row.value, Display::Empty));
        assert!(
            row.is_list_valued(),
            "a list type with no items is still edited as a list"
        );
        let mut f = value_field(&row);
        assert_eq!(f.item_index(), Some(0));
        f.insert("Smith, John");
        assert_eq!(
            commit(&f, PropertyType::Tags),
            Some(Change::Set {
                key: "tags".into(),
                value: Value::List(vec!["Smith, John".into()])
            })
        );
    }

    /// `k: []` is the form Obsidian writes for a list with no items, and it
    /// reads as blank: the cell draws the placeholder a missing value draws,
    /// which is what gives it the height a click needs, and the click opens
    /// the field for item 0.
    #[test]
    fn an_explicitly_empty_list_is_blank_and_takes_a_first_item() {
        for text in [
            "---\naliases: []\n---\n\nbody\n",
            "---\naliases:\n---\n\nbody\n",
        ] {
            let row = list_row(text, PropertyType::Aliases);
            assert!(row.items.is_empty());
            assert!(row.is_blank(), "{text:?} draws no value of its own");
            assert!(row.is_list_valued());
            let mut f = value_field(&row);
            assert_eq!(f.item_index(), Some(0));
            f.insert("Zed");
            assert_eq!(
                commit(&f, PropertyType::Aliases),
                Some(Change::Set {
                    key: "aliases".into(),
                    value: Value::List(vec!["Zed".into()])
                })
            );
        }
        // A key with no type of its own infers `MultiText` from `[]`, so it
        // is the same blank list rather than an empty scalar.
        let mut id = 0;
        let entries = parse_block("---\nstuff: []\n---\n\nbody\n").entries;
        let row = rows(
            &entries,
            |k| type_of(k, &entries, &PropertyTypes::default()),
            &mut id,
        )
        .pop()
        .expect("one row");
        assert_eq!(row.ty, PropertyType::MultiText);
        assert!(row.is_blank());
    }

    /// A list that does hold something is not blank, however little that is:
    /// its pills are the click targets, and the cell draws them.
    #[test]
    fn a_list_holding_an_empty_item_is_not_blank() {
        for text in [
            "---\naliases: [\"\"]\n---\n\nbody\n",
            "---\naliases:\n  - One\n---\n\nbody\n",
        ] {
            let row = list_row(text, PropertyType::Aliases);
            assert_eq!(row.items.len(), 1, "{text:?}");
            assert!(!row.is_blank(), "{text:?} has a pill to draw");
        }
        let scalar = list_row("---\ntitle: Hi\n---\n\nbody\n", PropertyType::Text);
        assert!(!scalar.is_blank());
    }

    #[test]
    fn a_value_field_nobody_typed_into_writes_nothing() {
        // Opening this value and closing it used to rewrite the file as
        // `- One` / `- Two` / `- with comma`.
        let row = list_row(
            "---\naliases:\n  - One\n  - \"Two, with comma\"\n---\n\nbody\n",
            PropertyType::Aliases,
        );
        assert_eq!(row.items, ["One", "Two, with comma"]);
        assert_eq!(row.edit_text, "One, Two, with comma");
        let whole = Field::new(
            Target::Existing(row.key.clone()),
            Part::Value,
            row.edit_text.clone(),
        );
        assert_eq!(commit(&whole, PropertyType::Aliases), None);
        // Nor does opening one pill, or the input after them, and leaving.
        assert_eq!(commit(&row.item_field(1), PropertyType::Aliases), None);
        assert_eq!(commit(&value_field(&row), PropertyType::Aliases), None);
        // A scalar whose stored form is not its display form is just as
        // untouchable: the vault is the source of truth.
        let nested = {
            let mut id = 0;
            rows(
                &[prop("nested", Value::Other("nested:\n  k: v".into()))],
                |_| PropertyType::Text,
                &mut id,
            )
            .pop()
            .unwrap()
        };
        assert_eq!(commit(&value_field(&nested), PropertyType::Text), None);
    }

    #[test]
    fn retyping_between_two_list_types_keeps_an_item_whole() {
        let row = list_row(
            "---\ntags:\n  - Smith, John\n---\n\nbody\n",
            PropertyType::Tags,
        );
        assert_eq!(
            retype(&row, PropertyType::MultiText),
            Change::Set {
                key: "tags".into(),
                value: Value::List(vec!["Smith, John".into()])
            }
        );
        // To a scalar it is still the joined text the row shows.
        assert_eq!(
            retype(&row, PropertyType::Text),
            Change::Set {
                key: "tags".into(),
                value: Value::Text("Smith, John".into())
            }
        );
    }

    #[test]
    fn every_type_has_a_glyph() {
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
            assert!(!type_glyph(ty).is_empty());
        }
    }
}
