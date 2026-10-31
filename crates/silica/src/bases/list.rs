//! The `list` layout: one line per row, its values joined.
//!
//! Its keys are observed (see [`silica_bases::layout`] and
//! [[Specification#Observed Bases write behavior, Obsidian 1.14.5 (installer build 1.13.7)]]):
//! `markers` (`bullet`, `number` or `none`, default `bullet`), `separator`
//! (any string, default `", "`) and the "Indent properties" toggle, whose own
//! key is **unprobed** — the toggle produced no write and the registration
//! names it only as a minified constant, so
//! [`ListLayout::indent_properties`](silica_bases::ListLayout) reads the
//! plain spelling and nothing writes it.
//!
//! The rows, the columns and the grouping are the table's: every layout draws
//! the same resolved [`Table`](silica_bases::Table), so a line shows the same
//! cells a row would, joined by `separator`. A line's title, its body and the
//! group header above it are [`cards`](crate::bases::cards)'s
//! [`title_of`], [`body_of`], [`renders_title_as_link`] and [`group_header`],
//! which the two layouts share because they differ only in their box.
//!
//! **Unprobed**: no list view was ever rendered in the run, so nothing of the
//! line's own box is observed. The readings are:
//!
//! - the **title** is the row's first cell — `order`'s first id, usually
//!   `file.name` — drawn in the link colour, and clicking the line opens the
//!   row's file;
//! - the rest of the row's values follow it on the same line, joined by
//!   `separator` and led by one, because the key's own description is what
//!   joins *a row's values* and the title is the first of them
//!   — [`line_text`];
//! - "Indent properties" puts the remaining values on their own lines,
//!   indented under the title, instead of after it — [`INDENT`];
//! - `markers: number` counts from 1 **inside each group**, which is what a
//!   markdown ordered list does per list; `bullet` draws `•`, the glyph the
//!   reading view draws for an unordered item — [`marker`];
//! - a **grouped** view draws the table's own group header between the runs
//!   of lines (`Table::groups`, the no-value group leading under an ascending
//!   `groupBy`), and an ungrouped view is one run of every row
//!   — [`lines_for`].
//!
//! The body is a virtualised [`gpui::list`], as the table's is: a group
//! header is not a line of values, so the list is variable-height, and a
//! variable-height list must be **reset** when the data changes — a splice
//! lands at the end — which [`BasesView::rebuild_list`] does.

use crate::bases::cells::{
    body_of, group_header, lines_for, renders_title_as_link, row_of, title_of, Line, CELL_PAD,
};
use crate::bases::view::BasesView;
use crate::theme::{Theme, FONT_UI_SMALL};
use gpui::{
    div, list, prelude::*, px, AnyElement, Context, ListAlignment, ListState, SharedString, Window,
};
use serde_json::{json, Value as Json};
use silica_bases::{Column, ListLayout, Markers, Row};

/// The vertical padding of one line, above and below.
///
/// **Unprobed**: 3 px over a 13 px line is the ~19 px a markdown list item
/// takes in the reading view, which is the nearest thing a list of values has
/// to a measured height.
pub const LINE_PAD: f32 = 3.;

/// How wide the marker column is, so every title starts at the same x
/// whatever the marker. **Unprobed**: wide enough for `•` and for two digits
/// and a stop at 13 px.
pub const MARKER_WIDTH: f32 = 20.;

/// How far "Indent properties" indents a value under its title.
/// **Unprobed**: the toggle was never driven, and this is the marker column's
/// own width, so an indented value lines up with the title above it.
pub const INDENT: f32 = MARKER_WIDTH;

/// The bullet `markers: bullet` draws. **Unprobed** as a glyph; `•` is what
/// the reading view draws for an unordered list item.
pub const BULLET: &str = "•";

/// How many lines [`ListLayoutState::state_json`] spells out.
const STATE_LINES: usize = 20;

/// What the list layout keeps between renders.
pub struct ListLayoutState {
    /// The scroll body, flattened: group headers interleaved with lines.
    pub lines: Vec<Line>,
    pub markers: Markers,
    /// `separator`: what joins a row's values, default `", "`. An empty
    /// string is a separator, so this is never defaulted on the way in.
    pub separator: String,
    pub indent: bool,
    /// The whole text of the body's first [`STATE_LINES`] lines, which is
    /// how the control socket sees that `separator` reached the draw.
    pub texts: Vec<String>,
    pub list: ListState,
}

impl Default for ListLayoutState {
    fn default() -> ListLayoutState {
        let layout = ListLayout::default();
        ListLayoutState {
            lines: Vec::new(),
            markers: layout.markers,
            separator: layout.separator,
            indent: layout.indent_properties,
            texts: Vec::new(),
            list: ListState::new(0, ListAlignment::Top, px(400.)),
        }
    }
}

impl ListLayoutState {
    pub fn state_json(&self) -> Json {
        json!({
            "lines": self.lines.len(),
            "rows": self.rows(),
            "groups": self
                .lines
                .iter()
                .filter(|l| matches!(l, Line::Group(_)))
                .count(),
            "markers": self.markers.key(),
            "separator": self.separator,
            "indent_properties": self.indent,
            "text": self.texts,
            "marks": self
                .lines
                .iter()
                .filter_map(|line| match line {
                    Line::Group(_) => None,
                    Line::Row { ix, .. } => Some(marker(self.markers, ix + 1)),
                })
                .take(STATE_LINES)
                .collect::<Vec<_>>(),
        })
    }

    /// How many of the body's lines are rows rather than group headers.
    pub fn rows(&self) -> usize {
        self.lines
            .iter()
            .filter(|l| matches!(l, Line::Row { .. }))
            .count()
    }
}

impl BasesView {
    pub(super) fn rebuild_list(&mut self) {
        let layout = match self.layout() {
            silica_bases::Layout::List(list) => list.clone(),
            _ => ListLayout::default(),
        };
        let grouped = self.view().is_some_and(|v| v.group_by.is_some());
        let (lines, texts) = match self.table() {
            Some(table) => {
                let lines = lines_for(table, grouped);
                let texts = lines
                    .iter()
                    .filter_map(|line| row_of(table, *line))
                    .take(STATE_LINES)
                    .map(|row| line_text(row, &layout.separator))
                    .collect();
                (lines, texts)
            }
            None => (Vec::new(), Vec::new()),
        };
        self.list.lines = lines;
        self.list.texts = texts;
        self.list.markers = layout.markers;
        self.list.separator = layout.separator;
        self.list.indent = layout.indent_properties;
        // Reset rather than splice: a splice on a variable-height list lands
        // at the end, so lines would silently stop following the data.
        let count = self.list.lines.len();
        self.list.list.reset(count);
    }

    /// Draws the lines, or the empty state when the view resolved to none.
    pub(super) fn render_list(&mut self, theme: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        if self.table().is_none() {
            return Vec::new();
        }
        if self.list.lines.is_empty() {
            return vec![self.render_empty(theme)];
        }
        vec![div()
            .flex_1()
            .min_h(px(0.))
            .w_full()
            .child(list(self.list.list.clone(), cx.processor(Self::render_list_line)).size_full())
            .into_any_element()]
    }

    /// One line of the scroll body: a group header, or a row's values.
    fn render_list_line(
        &mut self,
        ix: usize,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.global::<Theme>().clone();
        let Some(line) = self.list.lines.get(ix).copied() else {
            return div().into_any_element();
        };
        let Some(table) = self.table() else {
            return div().into_any_element();
        };
        match line {
            Line::Group(g) => {
                let Some(group) = table.groups.get(g) else {
                    return div().into_any_element();
                };
                let (key, rows) = (group.key.clone(), group.rows.len());
                group_header(ix, key.as_deref(), rows, &theme)
            }
            Line::Row { ix: n, .. } => {
                let Some(row) = row_of(table, line).cloned() else {
                    return div().into_any_element();
                };
                let columns = table.columns.clone();
                self.render_list_row(ix, n + 1, &row, &columns, &theme, cx)
            }
        }
    }

    /// One row as a line: its marker, its title, and the rest of its values
    /// after a `separator` — or beneath it, indented, when the view's
    /// "Indent properties" is on.
    fn render_list_row(
        &self,
        id: usize,
        number: usize,
        row: &Row,
        columns: &[Column],
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let path = row.file.path.clone();
        let separator = self.list.separator.clone();
        let link = renders_title_as_link(columns, row);
        let body = body_of(row);
        let title = div()
            .flex_shrink_0()
            .whitespace_nowrap()
            .text_color(if link { theme.link } else { theme.text })
            .child(SharedString::from(title_of(row)));
        let values = if self.list.indent {
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .child(title)
                .children(body.iter().map(|value| {
                    div()
                        .pl(px(INDENT))
                        .min_w_0()
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .text_ellipsis()
                        .text_color(theme.text_muted)
                        .child(SharedString::from(value.clone()))
                }))
        } else {
            div()
                .flex()
                .flex_row()
                .items_center()
                .min_w_0()
                .child(title)
                .when_some(trailing(&body, &separator), |d, rest| {
                    d.child(
                        div()
                            .min_w_0()
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .text_ellipsis()
                            .text_color(theme.text_muted)
                            .child(SharedString::from(rest)),
                    )
                })
        };
        let mut line = div()
            .id(("bases-line", id))
            .flex()
            .flex_row()
            .items_start()
            .w_full()
            .px(px(CELL_PAD))
            .py(px(LINE_PAD))
            .overflow_hidden()
            .text_size(px(FONT_UI_SMALL))
            .cursor_pointer()
            .hover(|s| s.bg(theme.bg_alt))
            .on_click(cx.listener(move |this, _, _, cx| this.open(path.clone(), cx)));
        if let Some(mark) = marker(self.list.markers, number) {
            line = line.child(
                div()
                    .flex_shrink_0()
                    .w(px(MARKER_WIDTH))
                    .text_color(theme.text_muted)
                    .child(SharedString::from(mark)),
            );
        }
        line.child(values).into_any_element()
    }
}

/// The marker a line carries, `number` counting from 1.
///
/// **Unprobed**: `markers` is observed and so are its three values, but no
/// list view was rendered, so the glyph `bullet` draws, the shape `number`
/// writes, and that `number` restarts inside each group are all readings — a
/// markdown ordered list numbers per list, and a group is a list.
pub fn marker(markers: Markers, number: usize) -> Option<String> {
    match markers {
        Markers::Bullet => Some(BULLET.to_string()),
        Markers::Number => Some(format!("{number}.")),
        Markers::None => None,
    }
}

/// The whole text of one line: every value of the row joined by `separator`,
/// the title first.
///
/// This is what the line draws, in one string — the drawing itself splits it
/// so the title can take the link colour, which is why the separator is
/// emitted before the body rather than between title and body as a special
/// case. **Unprobed**: that the title is one of the joined values at all, and
/// that a cell which rendered nothing is left out instead of leaving an empty
/// slot between two separators.
pub fn line_text(row: &Row, separator: &str) -> String {
    let mut out = title_of(row);
    if let Some(rest) = trailing(&body_of(row), separator) {
        out.push_str(&rest);
    }
    out
}

/// What follows the title on a line: a leading `separator` and then the rest
/// of the row's values joined by one. `None` when the row has nothing but a
/// title, so no separator is left dangling.
pub fn trailing(body: &[String], separator: &str) -> Option<String> {
    (!body.is_empty()).then(|| format!("{separator}{}", body.join(separator)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use silica_bases::Table;
    use silica_bases::{Cell, CellKind, ColumnSource, FileData, Group, Value};
    use std::sync::Arc;

    fn file(path: &str) -> Arc<FileData> {
        Arc::new(FileData {
            path: path.to_string(),
            name: path.to_string(),
            basename: path.trim_end_matches(".md").to_string(),
            ..FileData::default()
        })
    }

    fn row(path: &str, cells: &[&str]) -> Row {
        Row {
            file: file(path),
            cells: cells
                .iter()
                .map(|text| Cell {
                    value: Value::Str(text.to_string()),
                    error: None,
                    kind: CellKind::Text,
                    text: text.to_string(),
                })
                .collect(),
            image: None,
        }
    }

    fn table_of(rows: Vec<Row>, groups: Vec<Group>) -> Table {
        Table {
            columns: vec![Column {
                id: "file.name".to_string(),
                display: "file name".to_string(),
                source: ColumnSource::File("name".to_string()),
            }],
            rows,
            groups,
            limited: false,
            unknown_kind: None,
        }
    }

    #[test]
    fn a_line_joins_its_values_with_the_separator() {
        let r = row("A.md", &["Alpha", "todo", "0.02"]);
        assert_eq!(line_text(&r, ", "), "Alpha, todo, 0.02");
        assert_eq!(line_text(&r, "; "), "Alpha; todo; 0.02");
        // An empty separator is a separator, which is what the reader keeps.
        assert_eq!(line_text(&r, ""), "Alphatodo0.02");
    }

    #[test]
    fn a_line_takes_its_title_from_the_first_column() {
        let r = row("A.md", &["Alpha", "todo"]);
        assert_eq!(title_of(&r), "Alpha");
        assert_eq!(body_of(&r), vec!["todo".to_string()]);
    }

    #[test]
    fn a_cell_that_rendered_nothing_is_left_out_of_the_line() {
        let r = row("A.md", &["Alpha", "", "todo"]);
        assert_eq!(line_text(&r, ", "), "Alpha, todo");
    }

    #[test]
    fn a_row_of_one_value_is_a_line_with_no_separator_at_all() {
        let r = row("A.md", &["Alpha"]);
        assert_eq!(line_text(&r, ", "), "Alpha");
        assert_eq!(trailing(&[], ", "), None);
        assert_eq!(
            trailing(&["todo".to_string()], ", "),
            Some(", todo".to_string())
        );
    }

    #[test]
    fn a_marker_is_the_bullet_a_number_or_nothing() {
        assert_eq!(marker(Markers::Bullet, 7), Some(BULLET.to_string()));
        assert_eq!(marker(Markers::Number, 1), Some("1.".to_string()));
        assert_eq!(marker(Markers::Number, 12), Some("12.".to_string()));
        assert_eq!(marker(Markers::None, 1), None);
    }

    #[test]
    fn an_ungrouped_view_is_one_line_a_row() {
        let rows = vec![row("A.md", &["A"]), row("B.md", &["B"])];
        let table = table_of(
            rows.clone(),
            vec![Group {
                key: None,
                rows: rows.clone(),
            }],
        );
        assert_eq!(
            lines_for(&table, false),
            vec![
                Line::Row { group: None, ix: 0 },
                Line::Row { group: None, ix: 1 },
            ]
        );
        assert_eq!(
            row_of(&table, Line::Row { group: None, ix: 1 }),
            Some(&rows[1])
        );
    }

    #[test]
    fn a_grouped_view_puts_a_header_before_each_run_of_lines() {
        let a = row("A.md", &["A"]);
        let b = row("B.md", &["B"]);
        let c = row("C.md", &["C"]);
        let table = table_of(
            vec![a.clone(), b.clone(), c.clone()],
            vec![
                // The no-value group leads under an ascending `groupBy`.
                Group {
                    key: None,
                    rows: vec![a.clone()],
                },
                Group {
                    key: Some("todo".to_string()),
                    rows: vec![b.clone(), c.clone()],
                },
            ],
        );
        assert_eq!(
            lines_for(&table, true),
            vec![
                Line::Group(0),
                Line::Row {
                    group: Some(0),
                    ix: 0
                },
                Line::Group(1),
                Line::Row {
                    group: Some(1),
                    ix: 0
                },
                Line::Row {
                    group: Some(1),
                    ix: 1
                },
            ]
        );
        assert_eq!(row_of(&table, Line::Group(0)), None);
        assert_eq!(
            row_of(
                &table,
                Line::Row {
                    group: Some(1),
                    ix: 1
                }
            ),
            Some(&c)
        );
    }

    #[test]
    fn a_numbered_marker_counts_from_one_inside_each_group() {
        let a = row("A.md", &["A"]);
        let b = row("B.md", &["B"]);
        let table = table_of(
            vec![a.clone(), b.clone()],
            vec![
                Group {
                    key: Some("one".to_string()),
                    rows: vec![a.clone()],
                },
                Group {
                    key: Some("two".to_string()),
                    rows: vec![b.clone()],
                },
            ],
        );
        let numbers: Vec<Option<String>> = lines_for(&table, true)
            .into_iter()
            .filter_map(|line| match line {
                Line::Group(_) => None,
                Line::Row { ix, .. } => Some(marker(Markers::Number, ix + 1)),
            })
            .collect();
        assert_eq!(
            numbers,
            vec![Some("1.".to_string()), Some("1.".to_string())],
            "the second group's first line is 1. again"
        );
    }

    #[test]
    fn a_grouped_view_with_no_groups_falls_back_to_the_flat_rows() {
        let table = table_of(vec![row("A.md", &["A"])], Vec::new());
        assert_eq!(
            lines_for(&table, true),
            vec![Line::Row { group: None, ix: 0 }]
        );
    }

    #[test]
    fn the_defaults_are_the_observed_ones() {
        let state = ListLayoutState::default();
        assert_eq!(state.markers, Markers::Bullet);
        assert_eq!(state.separator, ", ");
        assert!(!state.indent);
    }
}
