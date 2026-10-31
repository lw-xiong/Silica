//! A view's **layout keys**: the per-layout settings that live beside
//! `type` and `name` in the view's own map.
//!
//! The published documentation calls these unpublished, so every key, widget,
//! range and default here is observed — read two ways that agreed, off each
//! registration's `options()` table (`plugin.getRegistrations()`) and off the
//! file after driving the real "Configure view" page. The table is in
//! [[Specification#Observed Bases write behavior, Obsidian 1.14.5 (installer build 1.13.7)]]:
//!
//! | Layout | Key | Control | Values / range | Default |
//! | --- | --- | --- | --- | --- |
//! | `table` | `rowHeight` | dropdown | `""` Short, `medium`, `tall`, `extra` | `""` (absent) |
//! | `table` | `columnSize` | a header drag | property id to integer px | absent |
//! | `cards` | `cardSize` | slider | 50 to 800 step 10 | 200 |
//! | `cards`, `kanban` | `image` | property picker | a qualified property id | absent |
//! | `cards`, `kanban` | `imageFit` | dropdown | `""` Cover, `contain` | `""` (absent) |
//! | `cards` | `imageAspectRatio` | slider | 0.25 to 2.5 step 0.05 | 1 |
//! | `kanban` | `columnWidth` | slider | 200 to 500 step 10 | 280 |
//! | `kanban` | `imageAspectRatio` | slider | 0.25 to 2.5 step 0.05 | 0.5 |
//! | `list` | `markers` | dropdown | `bullet`, `number`, `none` | `bullet` |
//! | `list` | `separator` | text | any string | `", "` |
//! | `list` | *(Indent properties)* | toggle | — | false |
//!
//! Two rules decide how this module behaves, and both are observed:
//!
//! - **A layout change keeps the old layout's keys.** Picking Kanban for a
//!   list view rewrote `type` and left `markers: number` and
//!   `separator: "; "` in place, so [`set_kind`] touches nothing but `kind`
//!   and reading a layout ignores every key that is not its own.
//! - **A layout key is a preserved key.** `View::preserved` is written last,
//!   in file order, which is why a layout key the UI wrote lands after a
//!   plugin's unknown key; inserting into that map is therefore all a setter
//!   has to do to match Obsidian's byte order.
//!
//! The reader never clamps and never rejects: a hand-written `cardSize: 4000`
//! is what the file says, and Obsidian's own slider is the only thing that
//! bounds one. The setters clamp to the slider's range, because that is the
//! control whose writes were observed.
//!
//! The key the list layout's "Indent properties" toggle writes is
//! **unprobed** — the toggle produced no write and the registration's source
//! names it only as a minified constant — so [`ListLayout::indent_properties`]
//! reads [`INDENT_PROPERTIES_KEY`], the plain spelling, and nothing writes it.

use indexmap::IndexMap;
use serde_yaml::{Mapping, Value};
use silica_formats::base::View;

/// The four layouts Obsidian registers, in the order the Configure view
/// page's Layout combobox offers them: the name, the label and the
/// `lucide-*` icon of each.
pub const LAYOUTS: &[(&str, &str, &str)] = &[
    ("table", "Table", "table"),
    ("cards", "Cards", "layout-grid"),
    ("list", "List", "list"),
    ("kanban", "Kanban", "kanban-square"),
];

/// The key the list layout's "Indent properties" toggle is assumed to write.
/// **Unprobed**, see the module note.
pub const INDENT_PROPERTIES_KEY: &str = "indentProperties";

/// Obsidian's label for a layout, when it is one of the four registered.
pub fn kind_label(kind: &str) -> Option<&'static str> {
    LAYOUTS
        .iter()
        .find(|(k, ..)| *k == kind)
        .map(|(_, l, _)| *l)
}

/// The `lucide-*` icon of a layout, without the `lucide-` prefix, as
/// [`crate::layout::LAYOUTS`] records it.
pub fn kind_icon(kind: &str) -> Option<&'static str> {
    LAYOUTS.iter().find(|(k, ..)| *k == kind).map(|(.., i)| *i)
}

// ---------------------------------------------------------------------------
// The layouts
// ---------------------------------------------------------------------------

/// A view's layout and the keys that layout reads.
///
/// `Unknown` is a `type` outside the four registered ones. The file still
/// parses and the type is kept; Obsidian draws "Unknown view type: x" and no
/// rows, which is [`silica_bases::Table::unknown_kind`](crate::Table).
#[derive(Debug, Clone, PartialEq)]
pub enum Layout {
    Table(TableLayout),
    Cards(CardsLayout),
    List(ListLayout),
    Kanban(KanbanLayout),
    Unknown(String),
}

impl Default for Layout {
    fn default() -> Layout {
        Layout::Table(TableLayout::default())
    }
}

impl Layout {
    /// Reads `view`'s layout off its `type` and its own keys.
    pub fn of(view: &View) -> Layout {
        match view.kind.as_str() {
            "table" => Layout::Table(TableLayout::of(view)),
            "cards" => Layout::Cards(CardsLayout::of(view)),
            "list" => Layout::List(ListLayout::of(view)),
            "kanban" => Layout::Kanban(KanbanLayout::of(view)),
            other => Layout::Unknown(other.to_string()),
        }
    }

    /// The `type` this layout was read from.
    pub fn kind(&self) -> &str {
        match self {
            Layout::Table(_) => "table",
            Layout::Cards(_) => "cards",
            Layout::List(_) => "list",
            Layout::Kanban(_) => "kanban",
            Layout::Unknown(k) => k,
        }
    }

    /// Whether this is one of the four layouts Obsidian registers.
    pub fn is_known(&self) -> bool {
        !matches!(self, Layout::Unknown(_))
    }
}

/// How a cell's image is fitted into its box: `""` Cover (the default, and
/// the key is then absent) or `contain`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ImageFit {
    #[default]
    Cover,
    Contain,
}

impl ImageFit {
    /// The string the key carries. Cover is `""`, which is also what an
    /// absent key means.
    pub fn key(self) -> &'static str {
        match self {
            ImageFit::Cover => "",
            ImageFit::Contain => "contain",
        }
    }

    fn of(s: Option<&str>) -> ImageFit {
        match s {
            Some("contain") => ImageFit::Contain,
            _ => ImageFit::Cover,
        }
    }
}

/// `rowHeight`: `""` Short (the default, and the key is then absent),
/// `medium`, `tall` or `extra` (Extra tall).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RowHeight {
    #[default]
    Short,
    Medium,
    Tall,
    Extra,
}

impl RowHeight {
    /// The string the key carries; Short is `""`.
    pub fn key(self) -> &'static str {
        match self {
            RowHeight::Short => "",
            RowHeight::Medium => "medium",
            RowHeight::Tall => "tall",
            RowHeight::Extra => "extra",
        }
    }

    /// The label the Row height dropdown shows.
    pub fn label(self) -> &'static str {
        match self {
            RowHeight::Short => "Short",
            RowHeight::Medium => "Medium",
            RowHeight::Tall => "Tall",
            RowHeight::Extra => "Extra tall",
        }
    }

    /// Every step, in the order the dropdown offers them.
    pub fn all() -> [RowHeight; 4] {
        [
            RowHeight::Short,
            RowHeight::Medium,
            RowHeight::Tall,
            RowHeight::Extra,
        ]
    }

    /// How many times a Short row's height this step is.
    ///
    /// **Unprobed**: `getRowBaseHeight()` and `getRowHeightMult()` are the
    /// functions that answer it and both are Obsidian's own source, which the
    /// run did not read. One step per name is the plain reading.
    pub fn mult(self) -> f32 {
        match self {
            RowHeight::Short => 1.,
            RowHeight::Medium => 2.,
            RowHeight::Tall => 3.,
            RowHeight::Extra => 4.,
        }
    }

    fn of(s: Option<&str>) -> RowHeight {
        match s {
            Some("medium") => RowHeight::Medium,
            Some("tall") => RowHeight::Tall,
            Some("extra") => RowHeight::Extra,
            _ => RowHeight::Short,
        }
    }
}

/// `markers`: the bullet a list layout's row carries.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Markers {
    #[default]
    Bullet,
    Number,
    None,
}

impl Markers {
    /// The string the key carries. Unlike the other dropdowns there is no
    /// empty value: the default `bullet` is spelled out.
    pub fn key(self) -> &'static str {
        match self {
            Markers::Bullet => "bullet",
            Markers::Number => "number",
            Markers::None => "none",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Markers::Bullet => "Bullet",
            Markers::Number => "Number",
            Markers::None => "None",
        }
    }

    pub fn all() -> [Markers; 3] {
        [Markers::Bullet, Markers::Number, Markers::None]
    }

    fn of(s: Option<&str>) -> Markers {
        match s {
            Some("number") => Markers::Number,
            Some("none") => Markers::None,
            _ => Markers::Bullet,
        }
    }
}

/// The `table` layout: `rowHeight` and `columnSize`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TableLayout {
    pub row_height: RowHeight,
    /// `columnSize`: a property id to its width in px, as a header drag
    /// writes it (`columnSize:` / `  file.name: 232`). The ids are the ones
    /// `order` carries, so a bare `status` and a qualified `note.status` are
    /// two different entries, exactly as the file has them.
    pub column_size: IndexMap<String, f64>,
}

impl TableLayout {
    pub fn of(view: &View) -> TableLayout {
        TableLayout {
            row_height: RowHeight::of(str_key(view, "rowHeight")),
            column_size: number_map(view, "columnSize"),
        }
    }

    /// The width `columnSize` gives `id`, when it names one above zero.
    pub fn width_of(&self, id: &str) -> Option<f32> {
        self.column_size
            .get(id)
            .copied()
            .filter(|w| *w > 0.)
            .map(|w| w as f32)
    }
}

/// The `cards` layout.
#[derive(Debug, Clone, PartialEq)]
pub struct CardsLayout {
    /// `cardSize`, 50 to 800 step 10, default 200.
    pub card_size: f64,
    /// `image`: the property whose value is the card's image, written
    /// qualified (`note.cover`, `formula.art`). Absent by default.
    pub image: Option<String>,
    pub image_fit: ImageFit,
    /// `imageAspectRatio`, 0.25 to 2.5 step 0.05, default 1.
    pub image_aspect_ratio: f64,
}

impl Default for CardsLayout {
    fn default() -> CardsLayout {
        CardsLayout {
            card_size: CARD_SIZE_DEFAULT,
            image: None,
            image_fit: ImageFit::default(),
            image_aspect_ratio: CARDS_ASPECT_DEFAULT,
        }
    }
}

impl CardsLayout {
    pub fn of(view: &View) -> CardsLayout {
        CardsLayout {
            card_size: number_key(view, "cardSize").unwrap_or(CARD_SIZE_DEFAULT),
            image: image_key(view),
            image_fit: ImageFit::of(str_key(view, "imageFit")),
            image_aspect_ratio: number_key(view, "imageAspectRatio")
                .unwrap_or(CARDS_ASPECT_DEFAULT),
        }
    }
}

/// The `list` layout.
#[derive(Debug, Clone, PartialEq)]
pub struct ListLayout {
    pub markers: Markers,
    /// `separator`: what joins a row's property values, default `", "`.
    pub separator: String,
    /// The "Indent properties" toggle. **Unprobed**, see the module note:
    /// this reads [`INDENT_PROPERTIES_KEY`] and nothing writes it.
    pub indent_properties: bool,
}

impl Default for ListLayout {
    fn default() -> ListLayout {
        ListLayout {
            markers: Markers::default(),
            separator: SEPARATOR_DEFAULT.to_string(),
            indent_properties: false,
        }
    }
}

impl ListLayout {
    pub fn of(view: &View) -> ListLayout {
        ListLayout {
            markers: Markers::of(str_key(view, "markers")),
            // A `separator: ""` is a separator: only an absent key — or one
            // whose value is not a string — falls back to the default.
            separator: str_key(view, "separator")
                .map(str::to_string)
                .unwrap_or_else(|| SEPARATOR_DEFAULT.to_string()),
            indent_properties: view
                .preserved
                .get(INDENT_PROPERTIES_KEY)
                .and_then(Value::as_bool)
                .unwrap_or(false),
        }
    }
}

/// The `kanban` layout: one column per `groupBy` value.
#[derive(Debug, Clone, PartialEq)]
pub struct KanbanLayout {
    /// `columnWidth`, 200 to 500 step 10, default 280.
    pub column_width: f64,
    pub image: Option<String>,
    pub image_fit: ImageFit,
    /// `imageAspectRatio`, 0.25 to 2.5 step 0.05, default **0.5** — the one
    /// key whose default differs from the cards layout's.
    pub image_aspect_ratio: f64,
}

impl Default for KanbanLayout {
    fn default() -> KanbanLayout {
        KanbanLayout {
            column_width: COLUMN_WIDTH_DEFAULT,
            image: None,
            image_fit: ImageFit::default(),
            image_aspect_ratio: KANBAN_ASPECT_DEFAULT,
        }
    }
}

impl KanbanLayout {
    pub fn of(view: &View) -> KanbanLayout {
        KanbanLayout {
            column_width: number_key(view, "columnWidth").unwrap_or(COLUMN_WIDTH_DEFAULT),
            image: image_key(view),
            image_fit: ImageFit::of(str_key(view, "imageFit")),
            image_aspect_ratio: number_key(view, "imageAspectRatio")
                .unwrap_or(KANBAN_ASPECT_DEFAULT),
        }
    }
}

// ---------------------------------------------------------------------------
// The sliders' own ranges
// ---------------------------------------------------------------------------

pub const CARD_SIZE_RANGE: (f64, f64, f64) = (50., 800., 10.);
pub const CARD_SIZE_DEFAULT: f64 = 200.;
pub const ASPECT_RANGE: (f64, f64, f64) = (0.25, 2.5, 0.05);
pub const CARDS_ASPECT_DEFAULT: f64 = 1.;
pub const KANBAN_ASPECT_DEFAULT: f64 = 0.5;
pub const COLUMN_WIDTH_RANGE: (f64, f64, f64) = (200., 500., 10.);
pub const COLUMN_WIDTH_DEFAULT: f64 = 280.;
pub const SEPARATOR_DEFAULT: &str = ", ";

/// Snaps `v` to a slider's `(min, max, step)`, which is what the control
/// writes: `cardSize: 630` and `imageAspectRatio: 2` are both on their step.
pub fn snap(v: f64, (min, max, step): (f64, f64, f64)) -> f64 {
    let clamped = v.clamp(min, max);
    let steps = ((clamped - min) / step).round();
    let snapped = min + steps * step;
    // 0.25 + 35 * 0.05 is 2.0000000000000004 in binary floating point, and
    // the key is written from this number, so the step's own precision is
    // what comes back.
    let places = if step.fract() == 0. { 0 } else { 2 };
    let scale = 10f64.powi(places);
    (snapped * scale).round() / scale
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

/// Changes a view's layout, which rewrites `type` and **keeps every key the
/// old layout wrote** — observed: a list view with `markers: number` and
/// `separator: "; "` became `type: kanban` with both keys still there.
pub fn set_kind(view: &mut View, kind: &str) {
    view.kind = kind.to_string();
}

/// `rowHeight`. Short is the default and its key is absent, so picking Short
/// removes the key rather than writing `rowHeight: ""`. **Unprobed**: the run
/// never drove the dropdown back to Short.
pub fn set_row_height(view: &mut View, h: RowHeight) {
    match h {
        RowHeight::Short => drop_key(view, "rowHeight"),
        other => set_str(view, "rowHeight", other.key()),
    }
}

/// `columnSize.<id>`, as a header drag writes it: an integer number of px.
/// The map keeps its own key order, and a new id is appended.
pub fn set_column_size(view: &mut View, id: &str, width: f64) {
    let width = width.round().max(1.);
    let mut map = match view.preserved.get("columnSize").and_then(Value::as_mapping) {
        Some(m) => m.clone(),
        None => Mapping::new(),
    };
    map.insert(Value::from(id.to_string()), number(width));
    view.preserved
        .insert("columnSize".to_string(), Value::Mapping(map));
}

/// Drops one id from `columnSize`, and the key with it when it was the last
/// one — an empty mapping is not a shape the UI ever wrote.
pub fn clear_column_size(view: &mut View, id: &str) {
    let Some(map) = view.preserved.get("columnSize").and_then(Value::as_mapping) else {
        return;
    };
    let mut map = map.clone();
    map.remove(Value::from(id.to_string()));
    if map.is_empty() {
        drop_key(view, "columnSize");
    } else {
        view.preserved
            .insert("columnSize".to_string(), Value::Mapping(map));
    }
}

/// `cardSize`, snapped to the slider's 50-to-800 step 10.
pub fn set_card_size(view: &mut View, size: f64) {
    set_number(view, "cardSize", snap(size, CARD_SIZE_RANGE));
}

/// `columnWidth`, snapped to the slider's 200-to-500 step 10.
pub fn set_column_width(view: &mut View, width: f64) {
    set_number(view, "columnWidth", snap(width, COLUMN_WIDTH_RANGE));
}

/// `imageAspectRatio`, snapped to the slider's 0.25-to-2.5 step 0.05. An
/// integral ratio is written bare (`imageAspectRatio: 2` for 2.00).
pub fn set_image_aspect_ratio(view: &mut View, ratio: f64) {
    set_number(view, "imageAspectRatio", snap(ratio, ASPECT_RANGE));
}

/// `image`: the property picker writes a **qualified** id, so a bare one is
/// qualified here the way the picker's own rows are. `None` drops the key,
/// which is its default state.
pub fn set_image(view: &mut View, property: Option<&str>) {
    match property {
        None => drop_key(view, "image"),
        Some(id) => set_str(view, "image", &qualify(id)),
    }
}

/// `imageFit`. Cover is the default and its key is absent.
pub fn set_image_fit(view: &mut View, fit: ImageFit) {
    match fit {
        ImageFit::Cover => drop_key(view, "imageFit"),
        ImageFit::Contain => set_str(view, "imageFit", fit.key()),
    }
}

/// `markers`. Every value is written, the default `bullet` included: unlike
/// the other dropdowns it has no empty value to fall back to.
pub fn set_markers(view: &mut View, markers: Markers) {
    set_str(view, "markers", markers.key());
}

/// `separator`, any string the text field holds.
pub fn set_separator(view: &mut View, separator: &str) {
    set_str(view, "separator", separator);
}

/// `limit`, which the results panel writes as a bare number; `None` drops it,
/// which is what *Show all* does.
pub fn set_limit(view: &mut View, limit: Option<u64>) {
    view.limit = limit;
}

/// A number as Obsidian writes one: integral values bare (`cardSize: 630`,
/// `imageAspectRatio: 2` for 2.00), the rest with their decimals.
pub fn number(v: f64) -> Value {
    if v.fract() == 0. && v.abs() < 9_007_199_254_740_992. {
        Value::from(v as i64)
    } else {
        Value::from(v)
    }
}

/// The property-id normalizer Obsidian runs `groupBy`, `order`, `sort` and
/// the keys of `summaries` through, and which the `image` picker writes with:
/// a bare id keeps its spelling (a hand-written `status` came back `status`),
/// and an id this build is writing for the first time is qualified `note.`
/// unless it already names a namespace.
pub fn qualify(id: &str) -> String {
    const NAMESPACES: &[&str] = &["file.", "note.", "formula."];
    if NAMESPACES.iter().any(|n| id.starts_with(n)) {
        id.to_string()
    } else {
        format!("note.{id}")
    }
}

fn set_str(view: &mut View, key: &str, value: &str) {
    view.preserved
        .insert(key.to_string(), Value::from(value.to_string()));
}

fn set_number(view: &mut View, key: &str, value: f64) {
    view.preserved.insert(key.to_string(), number(value));
}

fn drop_key(view: &mut View, key: &str) {
    view.preserved.shift_remove(key);
}

fn str_key<'v>(view: &'v View, key: &str) -> Option<&'v str> {
    view.preserved.get(key).and_then(Value::as_str)
}

fn number_key(view: &View, key: &str) -> Option<f64> {
    view.preserved.get(key).and_then(Value::as_f64)
}

/// `image` is a property id, and an empty string is no image rather than a
/// property with no name.
fn image_key(view: &View) -> Option<String> {
    str_key(view, "image")
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn number_map(view: &View, key: &str) -> IndexMap<String, f64> {
    let mut out = IndexMap::new();
    if let Some(map) = view.preserved.get(key).and_then(Value::as_mapping) {
        for (k, v) in map {
            if let (Some(k), Some(v)) = (k.as_str(), v.as_f64()) {
                out.insert(k.to_string(), v);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use silica_formats::base::Base;

    fn view_of(yaml: &str) -> View {
        Base::from_yaml(yaml).expect("parses").views.remove(0)
    }

    #[test]
    fn a_view_with_no_layout_keys_is_every_default() {
        let v = view_of("views:\n  - type: cards\n    name: V\n");
        let Layout::Cards(cards) = Layout::of(&v) else {
            panic!("cards");
        };
        assert_eq!(cards, CardsLayout::default());
        assert_eq!(cards.card_size, 200.);
        assert_eq!(cards.image_aspect_ratio, 1.);
        assert_eq!(cards.image, None);
        assert_eq!(cards.image_fit, ImageFit::Cover);
    }

    #[test]
    fn the_kanban_aspect_ratio_defaults_lower_than_the_cards_one() {
        let v = view_of("views:\n  - type: kanban\n    name: V\n");
        let Layout::Kanban(k) = Layout::of(&v) else {
            panic!("kanban");
        };
        assert_eq!(k.image_aspect_ratio, 0.5);
        assert_eq!(k.column_width, 280.);
    }

    #[test]
    fn the_keys_are_read_off_the_views_own_map() {
        let v = view_of(
            "views:\n  - type: list\n    name: V\n    markers: number\n    separator: \"; \"\n",
        );
        let Layout::List(l) = Layout::of(&v) else {
            panic!("list");
        };
        assert_eq!(l.markers, Markers::Number);
        assert_eq!(l.separator, "; ");
        assert!(!l.indent_properties);
    }

    #[test]
    fn an_unknown_type_is_kept_as_it_was_written() {
        let v = view_of("views:\n  - type: nonsense\n    name: V\n");
        assert_eq!(Layout::of(&v), Layout::Unknown("nonsense".to_string()));
        assert!(!Layout::of(&v).is_known());
    }

    #[test]
    fn a_layout_reads_none_of_another_layouts_keys() {
        // Observed: picking Kanban for a list view rewrote `type` and left
        // the list keys in place, so a kanban view may carry `markers`.
        let mut v = view_of(
            "views:\n  - type: list\n    name: V\n    markers: number\n    separator: \"; \"\n",
        );
        set_kind(&mut v, "kanban");
        assert_eq!(Layout::of(&v), Layout::Kanban(KanbanLayout::default()));
        assert_eq!(
            v.preserved.get("markers").and_then(Value::as_str),
            Some("number")
        );
    }

    #[test]
    fn a_column_size_is_a_px_map_keyed_by_the_id_the_file_uses() {
        let v = view_of(
            "views:\n  - type: table\n    name: V\n    columnSize:\n      file.name: 232\n",
        );
        let Layout::Table(t) = Layout::of(&v) else {
            panic!("table");
        };
        assert_eq!(t.width_of("file.name"), Some(232.));
        assert_eq!(t.width_of("status"), None);
        assert_eq!(t.row_height, RowHeight::Short);
    }

    #[test]
    fn a_row_height_step_is_the_key_obsidian_writes() {
        let v = view_of("views:\n  - type: table\n    name: V\n    rowHeight: tall\n");
        let Layout::Table(t) = Layout::of(&v) else {
            panic!("table");
        };
        assert_eq!(t.row_height, RowHeight::Tall);
        assert_eq!(t.row_height.mult(), 3.);
        assert_eq!(RowHeight::Short.key(), "");
    }

    #[test]
    fn a_slider_snaps_to_its_own_step() {
        assert_eq!(snap(634., CARD_SIZE_RANGE), 630.);
        assert_eq!(snap(4000., CARD_SIZE_RANGE), 800.);
        assert_eq!(snap(0., CARD_SIZE_RANGE), 50.);
        assert_eq!(snap(2.0, ASPECT_RANGE), 2.);
        assert_eq!(snap(1.02, ASPECT_RANGE), 1.);
        assert_eq!(snap(3., ASPECT_RANGE), 2.5);
        assert_eq!(snap(201., COLUMN_WIDTH_RANGE), 200.);
    }

    #[test]
    fn an_integral_number_is_written_bare() {
        let mut v = view_of("views:\n  - type: cards\n    name: V\n");
        set_card_size(&mut v, 630.);
        set_image_aspect_ratio(&mut v, 2.0);
        let base = Base {
            views: vec![v],
            ..Base::default()
        };
        let yaml = base.to_yaml();
        assert!(yaml.contains("cardSize: 630\n"), "{yaml}");
        assert!(yaml.contains("imageAspectRatio: 2\n"), "{yaml}");
    }

    #[test]
    fn a_fractional_ratio_keeps_its_decimals() {
        let mut v = view_of("views:\n  - type: kanban\n    name: V\n");
        set_image_aspect_ratio(&mut v, 0.75);
        assert_eq!(
            v.preserved.get("imageAspectRatio").and_then(Value::as_f64),
            Some(0.75)
        );
    }

    #[test]
    fn a_default_dropdown_value_drops_its_key() {
        let mut v = view_of(
            "views:\n  - type: table\n    name: V\n    rowHeight: tall\n    imageFit: contain\n",
        );
        set_row_height(&mut v, RowHeight::Short);
        set_image_fit(&mut v, ImageFit::Cover);
        assert!(!v.preserved.contains_key("rowHeight"));
        assert!(!v.preserved.contains_key("imageFit"));
    }

    #[test]
    fn markers_writes_every_value_including_its_default() {
        let mut v = view_of("views:\n  - type: list\n    name: V\n    markers: number\n");
        set_markers(&mut v, Markers::Bullet);
        assert_eq!(
            v.preserved.get("markers").and_then(Value::as_str),
            Some("bullet")
        );
    }

    #[test]
    fn the_image_picker_writes_a_qualified_id() {
        let mut v = view_of("views:\n  - type: cards\n    name: V\n");
        set_image(&mut v, Some("cover"));
        assert_eq!(
            v.preserved.get("image").and_then(Value::as_str),
            Some("note.cover")
        );
        set_image(&mut v, Some("formula.art"));
        assert_eq!(
            v.preserved.get("image").and_then(Value::as_str),
            Some("formula.art")
        );
        set_image(&mut v, None);
        assert!(!v.preserved.contains_key("image"));
    }

    #[test]
    fn a_layout_key_is_written_after_every_other_key() {
        // `preserved` is written last and in file order, so a key the UI
        // writes lands after a plugin's unknown key.
        let mut v = view_of(
            "views:\n  - type: table\n    name: V\n    pluginKey: x\n    order:\n      - file.name\n",
        );
        set_row_height(&mut v, RowHeight::Medium);
        let keys = v.key_order();
        assert_eq!(
            keys,
            vec!["type", "name", "order", "pluginKey", "rowHeight"]
                .into_iter()
                .map(str::to_string)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_column_size_drops_its_key_with_its_last_entry() {
        let mut v = view_of(
            "views:\n  - type: table\n    name: V\n    columnSize:\n      file.name: 232\n      status: 90\n",
        );
        clear_column_size(&mut v, "file.name");
        assert!(v.preserved.contains_key("columnSize"));
        clear_column_size(&mut v, "status");
        assert!(!v.preserved.contains_key("columnSize"));
    }

    #[test]
    fn a_hand_written_value_is_never_clamped_on_the_way_in() {
        let v = view_of("views:\n  - type: cards\n    name: V\n    cardSize: 4000\n");
        let Layout::Cards(c) = Layout::of(&v) else {
            panic!("cards");
        };
        assert_eq!(c.card_size, 4000.);
    }

    #[test]
    fn the_four_registered_layouts_have_obsidians_labels_and_icons() {
        assert_eq!(kind_label("kanban"), Some("Kanban"));
        assert_eq!(kind_icon("kanban"), Some("kanban-square"));
        assert_eq!(kind_icon("cards"), Some("layout-grid"));
        assert_eq!(kind_label("map"), None);
        assert_eq!(LAYOUTS.len(), 4);
    }
}
