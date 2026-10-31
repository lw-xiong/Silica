//! The `table` layout: a header row over virtualised rows.
//!
//! The shape is Obsidian's own, read off a running 1.14.5 over CDP
//! (`tools/obsidian-probe/results/2026-10-25-bases.json`; the prose is in
//! [[Specification#Observed Bases table view, Obsidian 1.14.5 (installer build 1.13.7)]]):
//!
//! - a **header row** of column display names, each header a fixed-width box
//!   (`columnSize` when the view carries one) with the last column taking the
//!   rest of the width. Observed on `Layout.base`: three headers at x 344, 436
//!   and 524, 92, 88 and 487 wide, 30 px tall, abutting with no gap, the first
//!   flush with the pane's left edge (ribbon 44 + sidebar 300 = 344), and the
//!   last one far wider than its text;
//! - the **rows** beneath it, each cell drawn by its `CellKind`, which is
//!   Obsidian's `data-property-type`: `text`, a `number`, or a **disabled
//!   checkbox** whose text is empty even when the value is `true`
//!   (`<div class="bases-table-cell bases-rendered-value"
//!   data-property-type="checkbox"><input disabled type="checkbox">`);
//! - the `file.name` column as an internal link, which is the one cell the run
//!   rendered as a `span.internal-link` rather than as plain text.
//!
//! The scroll body is virtualised: Obsidian's own table keeps `scrollEl` and
//! `theadEl` apart and calls `updateVirtualDisplay()` on resize, so the header
//! stays put while the rows scroll, and a few thousand rows cost only what is
//! on screen. The list is variable-height because a group header is not a row,
//! and a variable-height [`gpui::list`] must be **reset** when the data
//! changes — a splice lands at the end — so every rebuild goes through
//! [`BasesView::rebuild`](crate::bases::BasesView).
//!
//! The header is also where the table is **written through**: a right-click
//! on one opens the column's context menu and the right edge of one drags the
//! column wider, both of which live in [`edit`](crate::bases::edit) because
//! what they write is observed there. A cell typed into writes the note, not
//! the base, and that is in the same module.

use crate::bases::cells::{
    cell_text, checkbox, group_header, lines_for, renders_as_link, row_of, Line, CELL_PAD,
};
use crate::bases::edit::{editable_key, ColumnDrag};
use crate::bases::view::BasesView;
use crate::theme::{Theme, FONT_UI_SMALL};
use gpui::{
    div, list, prelude::*, px, AnyElement, Context, DragMoveEvent, ListAlignment, ListState,
    MouseButton, MouseDownEvent, SharedString, Window,
};
use serde_json::{json, Value as Json};
use silica_bases::{CellKind, Column, Row, Table, TableLayout, Value};

/// The height of a column header, observed: the right-click target of a
/// header on `Layout.base` is 30 px tall.
pub const HEADER_HEIGHT: f32 = 30.;

/// The height of a row at the default `rowHeight` (`""`, Short).
///
/// **Unprobed.** The table layout carries `getRowBaseHeight()` and
/// `getRowHeightMult()`, but both are Obsidian's own source and were left out
/// of the run, and no row's rect was measured. A Short row the height of the
/// header it sits under is the reading consistent with the rest of the shell.
pub const ROW_HEIGHT: f32 = 30.;

/// The narrowest a column is drawn at.
///
/// Read off the two unsized columns of `Layout.base`: "file name" came out
/// 92 px and "status" 88 px, four pixels apart over three characters of
/// difference, so 88 is a floor the shorter label did not clear rather than
/// its content width. The table layout's own `minColWidth` field agrees that
/// there is one.
pub const MIN_COLUMN_WIDTH: f32 = 88.;

/// The widest an unsized column is drawn at. **Unprobed** — `maxColWidth` is
/// a field of the layout, but no column ever reached it in the run.
pub const MAX_COLUMN_WIDTH: f32 = 400.;

/// How wide the grab area at a header's right edge is. **Unprobed**: that a
/// header drag resizes the column is observed, but the grip itself was never
/// measured. 6 px is the narrowest target the rest of the shell uses.
pub const GRIP_WIDTH: f32 = 6.;

/// A header drag in flight: the column whose right edge is moving and the
/// width it is at. Only the drawing uses it; `columnSize` is written once, at
/// the end of the drag, by
/// [`BasesView::finish_column_resize`](crate::bases::BasesView).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColumnResize {
    pub column: usize,
    pub width: f32,
}

/// What the table layout keeps between renders.
pub struct TableState {
    /// The scroll body, flattened: group headers interleaved with rows.
    pub lines: Vec<Line>,
    /// One width per column, in the order the table lists them.
    pub widths: Vec<f32>,
    /// The height of one row, from the view's `rowHeight`.
    pub row_height: f32,
    /// Whether the last column stretches to the rest of the width.
    ///
    /// Observed for an **unsized** last column: the third header of
    /// `Layout.base` came out 487 px, far wider than its text. A column the
    /// view's `columnSize` names is drawn at the width it names — "a resized
    /// column writes `columnSize` as a px map (`file.name: 232`), and the
    /// header then measures 232" — so a sized last column does not stretch.
    /// **Unprobed**: the fixture's sized column was its first.
    pub stretch_last: bool,
    /// A header drag in flight, which the widths already follow.
    pub resize: Option<ColumnResize>,
    pub list: ListState,
}

impl Default for TableState {
    fn default() -> TableState {
        TableState {
            lines: Vec::new(),
            widths: Vec::new(),
            row_height: ROW_HEIGHT,
            stretch_last: true,
            resize: None,
            list: ListState::new(0, ListAlignment::Top, px(400.)),
        }
    }
}

impl TableState {
    /// What the control socket reports for the table layout.
    pub fn state_json(&self) -> Json {
        json!({
            "lines": self.lines.len(),
            "widths": self.widths,
            "row_height": self.row_height,
            "stretch_last": self.stretch_last,
            "resize": self.resize.map(|r| json!({ "column": r.column, "width": r.width })),
        })
    }
}

impl BasesView {
    /// Rebuilds the scroll body and the measurements that depend on it.
    ///
    /// The list is **reset** rather than spliced: a splice on a
    /// variable-height list lands at the end, so rows would silently stop
    /// following the data. A base is redrawn only when the file or the vault
    /// changed, where Obsidian starts the view from its top too.
    pub(super) fn rebuild_table(&mut self) {
        let grouped = self.view().is_some_and(|v| v.group_by.is_some());
        let layout = match self.layout() {
            silica_bases::Layout::Table(table) => table.clone(),
            _ => TableLayout::default(),
        };
        let (lines, widths, stretch_last) = match self.table() {
            Some(table) => (
                lines_for(table, grouped),
                column_widths(&table.columns, &layout),
                stretches_last(&table.columns, &layout),
            ),
            None => (Vec::new(), Vec::new(), true),
        };
        self.table_state.lines = lines;
        self.table_state.widths = widths;
        self.table_state.stretch_last = stretch_last;
        self.table_state.row_height = ROW_HEIGHT * layout.row_height.mult();
        // A rebuild under a header drag — the watcher re-resolving the table
        // while a column is being pulled wider — must not snap the edge back
        // to the width the file still has.
        if let Some(resize) = self.table_state.resize {
            if let Some(slot) = self.table_state.widths.get_mut(resize.column) {
                *slot = resize.width;
            }
        }
        // An open cell field is kept only while the line it opened on still
        // stands for the same note: a rebuild re-flattens the body, so a row
        // can move or go away under it.
        if let Some(edit) = self.editing.cell.as_ref() {
            let still_there = self
                .table_state
                .lines
                .get(edit.line)
                .copied()
                .and_then(|line| self.table().and_then(|t| row_of(t, line)))
                .is_some_and(|row| row.file.path == edit.path);
            if !still_there {
                self.editing.cell = None;
            }
        }
        let count = self.table_state.lines.len();
        self.table_state.list.reset(count);
    }

    /// The header row over the virtualised body, or the empty state when the
    /// view resolved to no rows at all, and the column menu over both.
    ///
    /// Header and body share one box so a header drag keeps being followed
    /// once the pointer has left the 30 px header:
    /// [`gpui::InteractiveElement::on_drag_move`] reports every move while the
    /// drag that started on a grip is alive, and the box's own left edge is
    /// the table's, which is what the width is measured from.
    pub(super) fn render_table(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let Some(table) = self.table() else {
            return Vec::new();
        };
        let header = self.render_header(table, theme, cx);
        let body = if self.table_state.lines.is_empty() {
            self.render_empty(theme)
        } else {
            div()
                .flex_1()
                .min_h(px(0.))
                .w_full()
                .child(
                    list(
                        self.table_state.list.clone(),
                        cx.processor(Self::render_line),
                    )
                    .size_full(),
                )
                .into_any_element()
        };
        let mut body = vec![div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.))
            .w_full()
            .on_drag_move(
                cx.listener(|this, event: &DragMoveEvent<ColumnDrag>, _, cx| {
                    let column = event.drag(cx).column;
                    this.resize_column(column, event.event.position.x, event.bounds.left(), cx);
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.finish_column_resize(cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.finish_column_resize(cx)),
            )
            .child(header)
            .child(body)
            .into_any_element()];
        body.extend(self.render_header_menu(theme, cx));
        body
    }

    /// The header row: one box per column, abutting, an unsized last column
    /// taking the rest of the width, over a hairline.
    ///
    /// Each header is also its own target: a right-click opens the column's
    /// context menu and the 6 px at its right edge drag the column wider.
    fn render_header(&self, table: &Table, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let last = table.columns.len().saturating_sub(1);
        let stretch_last = self.table_state.stretch_last;
        div()
            .flex()
            .flex_row()
            .flex_shrink_0()
            .w_full()
            .h(px(HEADER_HEIGHT))
            .border_b_1()
            .border_color(theme.border)
            .children(table.columns.iter().enumerate().map(|(ix, col)| {
                let width = self
                    .table_state
                    .widths
                    .get(ix)
                    .copied()
                    .unwrap_or(MIN_COLUMN_WIDTH);
                let mut cell = div()
                    .id(("bases-header", ix))
                    .relative()
                    .flex()
                    .flex_row()
                    .items_center()
                    .h_full()
                    .px(px(CELL_PAD))
                    .overflow_hidden()
                    .text_size(px(FONT_UI_SMALL))
                    .text_color(theme.text_muted)
                    .when(ix > 0, |d| d.border_l_1().border_color(theme.border))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                            this.open_header_menu(ix, event.position, cx)
                        }),
                    );
                cell = if ix == last && stretch_last {
                    cell.flex_1().min_w(px(width))
                } else {
                    cell.flex_shrink_0().w(px(width))
                };
                cell.child(
                    div()
                        .min_w_0()
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(SharedString::from(col.display.clone())),
                )
                .child(
                    div()
                        .id(("bases-header-grip", ix))
                        .absolute()
                        .top(px(0.))
                        .right(px(0.))
                        .w(px(GRIP_WIDTH))
                        .h_full()
                        .cursor_col_resize()
                        .on_drag(ColumnDrag { column: ix }, |drag, _, _, cx| {
                            cx.stop_propagation();
                            cx.new(|_| drag.clone())
                        })
                        // The press that starts the drag is not a press on
                        // the header, which would open its menu.
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()),
                )
            }))
            .into_any_element()
    }

    /// One line of the scroll body.
    fn render_line(&mut self, ix: usize, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.global::<Theme>().clone();
        let Some(line) = self.table_state.lines.get(ix).copied() else {
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
            Line::Row { .. } => {
                let Some(row) = row_of(table, line).cloned() else {
                    return div().into_any_element();
                };
                self.render_row(ix, &row, &theme, cx)
            }
        }
    }

    /// One row: a cell per column.
    ///
    /// What a click on a cell does is this build's reading, and **unprobed**:
    /// a cell the frontmatter owns opens the field over it, and every other
    /// cell — a `file.*` the filesystem owns, a `formula.*` that is computed —
    /// activates the file, which is what the read-only first cut did for all
    /// of them. The cursor says which is which, so a cell that refuses is
    /// drawn as refusing rather than dropping the click.
    fn render_row(
        &self,
        ix: usize,
        row: &Row,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let last = row.cells.len().saturating_sub(1);
        let columns: Vec<Column> = self.table().map(|t| t.columns.clone()).unwrap_or_default();
        // The column of this row a field is open over, if any.
        let open = self
            .editing
            .cell
            .as_ref()
            .filter(|edit| edit.line == ix)
            .map(|edit| edit.column);
        let field = self.editing.field.clone();
        div()
            .id(("bases-row", ix))
            .flex()
            .flex_row()
            .w_full()
            .h(px(self.table_state.row_height))
            .border_b_1()
            .border_color(theme.border)
            .hover(|s| s.bg(theme.bg_alt))
            .children(row.cells.iter().enumerate().map(|(c, cell)| {
                let width = self
                    .table_state
                    .widths
                    .get(c)
                    .copied()
                    .unwrap_or(MIN_COLUMN_WIDTH);
                let column = columns.get(c);
                let link = column.is_some_and(|col| renders_as_link(col, cell));
                let editable = column.and_then(editable_key).is_some();
                let mut box_ = div()
                    .id(SharedString::from(format!("bases-cell-{ix}-{c}")))
                    .flex()
                    .flex_row()
                    .items_center()
                    .h_full()
                    .px(px(CELL_PAD))
                    .overflow_hidden()
                    .text_size(px(FONT_UI_SMALL))
                    .when(c > 0, |d| d.border_l_1().border_color(theme.border));
                box_ = if c == last && self.table_state.stretch_last {
                    box_.flex_1().min_w(px(width))
                } else {
                    box_.flex_shrink_0().w(px(width))
                };
                // A field is open over this very cell: it draws instead of the
                // value, and a press anywhere else commits it, the way the
                // property editor's own field closes.
                if open == Some(c) {
                    if let Some(field) = field.clone() {
                        return box_
                            .px(px(0.))
                            .on_mouse_down_out(
                                cx.listener(|this, _, _, cx| this.commit_cell_edit(cx)),
                            )
                            .child(field);
                    }
                }
                let path = row.file.path.clone();
                box_ = match (editable, cell.kind) {
                    // A boolean has no text to type, so the click flips it.
                    (true, CellKind::Checkbox) => box_.cursor_pointer().on_click(
                        cx.listener(move |this, _, _, cx| this.toggle_checkbox_cell(ix, c, cx)),
                    ),
                    (true, _) => {
                        box_.cursor_text()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.start_cell_edit(ix, c, window, cx)
                            }))
                    }
                    (false, _) => box_
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| this.open(path.clone(), cx))),
                };
                match cell.kind {
                    // A boolean is a disabled checkbox, so there is no text
                    // even when the value is `true`.
                    CellKind::Checkbox => {
                        box_.child(checkbox(matches!(cell.value, Value::Bool(true)), theme))
                    }
                    // **Unprobed**: no number column's alignment was measured;
                    // right is the convention a number column reads in.
                    CellKind::Number => box_.justify_end().text_color(theme.text).child(
                        div()
                            .min_w_0()
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .child(SharedString::from(cell_text(cell))),
                    ),
                    CellKind::Text => box_
                        .text_color(if link { theme.link } else { theme.text })
                        .child(
                            div()
                                .min_w_0()
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .text_ellipsis()
                                .child(SharedString::from(cell_text(cell))),
                        ),
                }
            }))
            .into_any_element()
    }
}

/// How wide each column is drawn.
///
/// `columnSize` is observed — a header drag wrote `columnSize:` /
/// `  file.name: 232` and the header then measured 232 px — and it is one of
/// the view's layout keys, so it arrives through
/// [`silica_bases::TableLayout`]. A column the map does not name is sized
/// from its label, floored at [`MIN_COLUMN_WIDTH`], which is the only reading
/// that fits both unsized columns of the fixture (92 for "file name", 88 for
/// "status"). The label's own width is **estimated** rather than measured: the
/// real table content-fits its header with the text it has already laid out,
/// and this element sizes its columns before the text exists.
pub fn column_widths(columns: &[Column], layout: &TableLayout) -> Vec<f32> {
    columns
        .iter()
        .map(|col| match layout.width_of(&col.id) {
            Some(w) => w,
            // ~7 px a glyph at 13 px of Inter, plus the cell's two pads
            // and the room Obsidian leaves for a header's type icon.
            None => (col.display.chars().count() as f32 * 7. + 2. * CELL_PAD + 24.)
                .clamp(MIN_COLUMN_WIDTH, MAX_COLUMN_WIDTH),
        })
        .collect()
}

/// Whether the last column takes the rest of the width.
///
/// An unsized last column does, observed: the third header of `Layout.base`
/// came out 487 px wide. A column `columnSize` names is drawn at the width it
/// names — the resized `file.name` measured exactly its 232 — so a last
/// column the map names keeps that width instead of stretching, which is what
/// makes a header drag on the last column do anything at all. **Unprobed**:
/// the fixture's one sized column was its first.
pub fn stretches_last(columns: &[Column], layout: &TableLayout) -> bool {
    match columns.last() {
        Some(col) => layout.width_of(&col.id).is_none(),
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use silica_bases::{Cell, ColumnSource, FileData, Group};
    use std::sync::Arc;

    fn file(path: &str) -> Arc<FileData> {
        Arc::new(FileData {
            path: path.to_string(),
            name: path.to_string(),
            basename: path.trim_end_matches(".md").to_string(),
            ..FileData::default()
        })
    }

    fn row(path: &str) -> Row {
        Row {
            file: file(path),
            cells: vec![Cell {
                value: Value::Str(path.to_string()),
                error: None,
                kind: CellKind::Text,
                text: path.to_string(),
            }],
            image: None,
        }
    }

    fn column(id: &str, display: &str, source: ColumnSource) -> Column {
        Column {
            id: id.to_string(),
            display: display.to_string(),
            source,
        }
    }

    fn table_of(rows: Vec<Row>, groups: Vec<Group>) -> Table {
        Table {
            columns: vec![column(
                "file.name",
                "file name",
                ColumnSource::File("name".into()),
            )],
            rows,
            groups,
            limited: false,
            unknown_kind: None,
        }
    }

    #[test]
    fn an_ungrouped_table_is_one_line_a_row() {
        let rows = vec![row("A.md"), row("B.md")];
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
    fn a_grouped_table_puts_a_header_before_each_group() {
        let a = row("A.md");
        let b = row("B.md");
        let table = table_of(
            vec![a.clone(), b.clone()],
            vec![
                Group {
                    key: None,
                    rows: vec![a.clone()],
                },
                Group {
                    key: Some("todo".to_string()),
                    rows: vec![b.clone()],
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
            ]
        );
        assert_eq!(
            row_of(
                &table,
                Line::Row {
                    group: Some(1),
                    ix: 0
                }
            ),
            Some(&b)
        );
        assert_eq!(row_of(&table, Line::Group(0)), None);
    }

    #[test]
    fn a_grouped_view_with_no_groups_falls_back_to_the_flat_rows() {
        let table = table_of(vec![row("A.md")], Vec::new());
        assert_eq!(
            lines_for(&table, true),
            vec![Line::Row { group: None, ix: 0 }]
        );
    }

    #[test]
    fn a_column_size_the_view_carries_wins_over_the_label() {
        let mut layout = TableLayout::default();
        layout.column_size.insert("file.name".to_string(), 232.);
        let widths = column_widths(
            &[column(
                "file.name",
                "file name",
                ColumnSource::File("name".into()),
            )],
            &layout,
        );
        assert_eq!(widths, vec![232.]);
    }

    #[test]
    fn only_an_unsized_last_column_stretches() {
        let columns = [
            column("file.name", "file name", ColumnSource::File("name".into())),
            column("status", "status", ColumnSource::Property("status".into())),
        ];
        let mut layout = TableLayout::default();
        assert!(stretches_last(&columns, &layout));
        layout.column_size.insert("file.name".to_string(), 232.);
        assert!(
            stretches_last(&columns, &layout),
            "a sized column that is not the last one changes nothing"
        );
        layout.column_size.insert("status".to_string(), 120.);
        assert!(!stretches_last(&columns, &layout));
        assert!(stretches_last(&[], &layout));
    }

    #[test]
    fn an_unsized_column_never_leaves_the_floor_and_ceiling() {
        let layout = TableLayout::default();
        let widths = column_widths(
            &[
                column("s", "s", ColumnSource::Property("s".into())),
                column(
                    "long",
                    &"x".repeat(200),
                    ColumnSource::Property("long".into()),
                ),
            ],
            &layout,
        );
        assert_eq!(widths[0], MIN_COLUMN_WIDTH);
        assert_eq!(widths[1], MAX_COLUMN_WIDTH);
    }
}
