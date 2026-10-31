//! The `kanban` layout: the engine's groups side by side, cards inside them.
//!
//! A kanban board **is** the grouping. One column per `groupBy` value, in the
//! order `groupOrder` and the `groupBy` direction put them in, with the
//! no-value group leading under an ascending `groupBy` — all of which
//! [`Table::groups`] already answers, so this layout never groups anything
//! itself. A view with no `groupBy` has one group holding every row, so such a
//! board is one column of the whole table. Each column holds one card per row
//! of its group, showing the same cells the table's row would show, with the
//! view's `limit` already applied by the engine.
//!
//! Its keys are observed, read two ways that agreed — off
//! `plugin.getRegistrations()` and off the file after driving the real
//! "Configure view" page (see [`silica_bases::layout`] and
//! [[Specification#Observed Bases write behavior, Obsidian 1.14.5 (installer build 1.13.7)]]):
//! `columnWidth` (200 to 500 step 10, default 280), `image` (a property id,
//! which the engine evaluates per row into [`Row::image`]), `imageFit` (`""`
//! Cover or `contain`) and `imageAspectRatio` (0.25 to 2.5 step 0.05, default
//! **0.5** — the one key whose default differs from the cards layout's).
//!
//! The board scrolls sideways through its columns and each column scrolls its
//! own cards, which is what makes a column a [`gpui::uniform_list`]: every
//! card of a board draws the same cells over the same image box, so they are
//! all one height ([`card_height`]) and a column with ten thousand cards costs
//! only the ones on screen. The gap is drawn inside the item, because a
//! uniform list measures one item and lays the rest out on that measurement.
//!
//! **Unprobed.** No kanban view was ever rendered in either probe run, so
//! everything below the keys is a reading rather than an observation, and each
//! is marked where it is made: the column header's own text and where its row
//! count sits, what a null group key reads as, which cell is a card's title,
//! that a click on a card opens its file, the card's own box, and the two
//! scroll directions above.
//!
//! **Dragging a card between columns is deliberately not implemented.** The
//! spec's unprobed list says nothing is known about whether a kanban drag
//! exists, let alone what it writes — presumably the dragged note's `groupBy`
//! property, but which property id, which value spelling, and whether
//! `groupOrder` moves with it are all unobserved. A wrong guess would write a
//! user's note, so the board is read-only until a probe says otherwise.

use crate::bases::cells::{cell_text, checkbox, image_ref, renders_as_link, ImageRef, CELL_PAD};
use crate::bases::view::BasesView;
use crate::theme::{Theme, FONT_UI_SMALL, FONT_UI_SMALLER};
use gpui::{
    div, img, prelude::*, px, uniform_list, AnyElement, Context, ObjectFit, ScrollHandle,
    SharedString, UniformListScrollHandle, Window,
};
use serde_json::{json, Value as Json};
use silica_bases::{CellKind, Column, ImageFit, KanbanLayout, Layout, Row, Table, Value};
use std::ops::Range;

/// The height of a column's header.
///
/// **Unprobed**: the kanban's own header was never rendered. The table's
/// header row measured 30 px
/// ([`HEADER_HEIGHT`](crate::bases::table::HEADER_HEIGHT)), and a group header
/// at the same step is what the rest of the shell reads in.
pub const COLUMN_HEADER_HEIGHT: f32 = 30.;

/// What a column with no key reads as when the view groups: Obsidian's
/// "no value for this property" group.
///
/// **Unprobed**: the word Obsidian uses was never read off a running app, in
/// the table layout either. "Empty" is what the property editor writes for a
/// blank value, and it is the same word
/// [`render_group`](crate::bases::table) draws for one, so the two layouts
/// agree.
pub const EMPTY_GROUP: &str = "Empty";

/// The narrowest a column is drawn, whatever the file asks for.
///
/// Not a clamp of `columnWidth`: the reader never clamps, and the slider's own
/// floor is 200. This is only a guard, so a hand-written `columnWidth: 0`
/// draws a narrow column rather than none at all, and 88 px is the floor the
/// table's columns were observed to have.
pub const MIN_COLUMN_WIDTH: f32 = 88.;

/// Padding inside a column, around its cards.
pub const COLUMN_PAD: f32 = 8.;

/// The gap between two columns, and between two cards.
pub const CARD_GAP: f32 = 8.;

/// The line a card's title takes, and the line each of its other cells takes.
/// **Unprobed**, like the rest of the card's box.
pub const CARD_TITLE_HEIGHT: f32 = 20.;
pub const CARD_LINE_HEIGHT: f32 = 18.;

/// The hairline around a card, which counts towards its height.
pub const CARD_BORDER: f32 = 1.;

/// One column of the board: one group of the resolved table.
#[derive(Debug, Clone, Default)]
pub struct BoardColumn {
    /// The group's key, as [`silica_bases::Group::key`] has it; `None` is the
    /// no-value group — or the single column of a view that does not group.
    pub key: Option<String>,
    /// How many cards the column holds.
    pub rows: usize,
    /// The column scrolls its own cards.
    pub cards: UniformListScrollHandle,
}

/// What the kanban layout keeps between renders.
pub struct KanbanState {
    /// The board scrolls sideways through its columns.
    pub scroll: ScrollHandle,
    /// One per group, in the order the engine grouped them.
    pub columns: Vec<BoardColumn>,
    /// Whether the showing view has a `groupBy`, which is what decides
    /// whether a `None` key is the no-value group or just the one column of an
    /// ungrouped board.
    pub grouped: bool,
    /// How wide a column is drawn, from `columnWidth`.
    pub column_width: f32,
    /// How wide a card is drawn, which is the column less its padding.
    pub card_width: f32,
    /// How tall a card's image box is, or `None` when the view names no
    /// `image` property and a card therefore has none.
    pub image_height: Option<f32>,
    pub image_fit: ImageFit,
    /// How tall every card of this board is; see [`card_height`].
    pub card_height: f32,
}

impl Default for KanbanState {
    fn default() -> KanbanState {
        let layout = KanbanLayout::default();
        let column_width = column_width(&layout);
        let card_width = card_width(column_width);
        KanbanState {
            scroll: ScrollHandle::default(),
            columns: Vec::new(),
            grouped: false,
            column_width,
            card_width,
            image_height: image_height(card_width, &layout),
            image_fit: ImageFit::default(),
            card_height: card_height(0, None),
        }
    }
}

impl KanbanState {
    /// What the control socket reports for the kanban layout: the board's
    /// columns with their keys and row counts, and the measurements the keys
    /// decided.
    pub fn state_json(&self) -> Json {
        json!({
            "columns": self
                .columns
                .iter()
                .map(|c| json!({ "key": c.key, "rows": c.rows }))
                .collect::<Vec<_>>(),
            "grouped": self.grouped,
            "column_width": self.column_width,
            "card_width": self.card_width,
            "card_height": self.card_height,
            "image_height": self.image_height,
        })
    }
}

impl BasesView {
    /// Recomputes the board: its columns, and the sizes the layout keys give a
    /// card.
    ///
    /// The scroll handles are new ones, so a redraw starts the board at its
    /// first column and each column at its first card — the same choice the
    /// table layout makes when it resets its list, and for the same reason: a
    /// base is redrawn only when the file or the vault changed.
    pub(super) fn rebuild_kanban(&mut self) {
        let grouped = self.view().is_some_and(|v| v.group_by.is_some());
        let layout = match self.layout() {
            Layout::Kanban(kanban) => kanban.clone(),
            // A kanban board of another layout's keys is every default; this
            // is only reachable through the rebuild dispatch, which already
            // matched the layout.
            _ => KanbanLayout::default(),
        };
        let columns = self.table().map(board_columns).unwrap_or_default();
        let cells = self.table().map_or(0, |t| t.columns.len());
        let column_width = column_width(&layout);
        let card_width = card_width(column_width);
        let image_height = image_height(card_width, &layout);
        self.kanban = KanbanState {
            scroll: ScrollHandle::default(),
            columns,
            grouped,
            column_width,
            card_width,
            image_height,
            image_fit: layout.image_fit,
            card_height: card_height(cells, image_height),
        };
    }

    /// Draws the board: the columns side by side, scrolling sideways.
    pub(super) fn render_kanban(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        if self.table().is_none() {
            return Vec::new();
        }
        // No column holding a card is the same nothing the table draws for a
        // view that resolved to no rows — an ungrouped view still has its one
        // group, so the count is what says the board is empty.
        if self.kanban.columns.iter().all(|column| column.rows == 0) {
            return vec![self.render_empty(theme)];
        }
        let grouped = self.kanban.grouped;
        let width = self.kanban.column_width;
        let columns: Vec<AnyElement> = self
            .kanban
            .columns
            .iter()
            .enumerate()
            .map(|(g, column)| {
                let cards = uniform_list(
                    ("bases-kanban-cards", g),
                    column.rows,
                    cx.processor(move |this, range: Range<usize>, window, cx| {
                        this.render_kanban_cards(g, range, window, cx)
                    }),
                )
                .track_scroll(&column.cards)
                .size_full();
                div()
                    .id(("bases-kanban-column", g))
                    .flex()
                    .flex_col()
                    .flex_shrink_0()
                    .w(px(width))
                    .h_full()
                    .rounded_md()
                    .bg(theme.bg_secondary)
                    .child(column_header(
                        column_title(column.key.as_deref(), grouped),
                        column.rows,
                        theme,
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_h(px(0.))
                            .w_full()
                            .px(px(COLUMN_PAD))
                            .child(cards),
                    )
                    .into_any_element()
            })
            .collect();
        vec![div()
            .id("bases-kanban")
            .flex_1()
            .min_h(px(0.))
            .w_full()
            .overflow_x_scroll()
            .track_scroll(&self.kanban.scroll)
            .flex()
            .flex_row()
            .items_stretch()
            .gap(px(CARD_GAP))
            .p(px(COLUMN_PAD))
            .children(columns)
            .into_any_element()]
    }

    /// The cards of column `g` the list asked for.
    fn render_kanban_cards(
        &mut self,
        g: usize,
        range: Range<usize>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let theme = cx.global::<Theme>().clone();
        let Some(table) = self.table() else {
            return Vec::new();
        };
        let columns = table.columns.clone();
        let rows = rows_of(table, g);
        let end = range.end.min(rows.len());
        let rows: Vec<Row> = rows
            .get(range.start.min(end)..end)
            .unwrap_or_default()
            .to_vec();
        rows.iter()
            .enumerate()
            .map(|(at, row)| self.render_kanban_card(range.start + at, row, &columns, &theme, cx))
            .collect()
    }

    /// One card: the row's image over its cells, opening the row's file.
    ///
    /// **Unprobed.** The whole box is a reading: the image full-bleed across
    /// the card's top at [`KanbanState::image_height`], the **first** cell as
    /// the card's title (the plain reading of "which cell names a card", and
    /// the one cell the table was observed to draw as a link), the rest one
    /// faint line each, and a click opening the file — which is what a table
    /// row does and the only thing a card could usefully do while the board is
    /// read-only. A line is drawn for every cell even when the cell is empty,
    /// because a card the same height as every other card is what lets the
    /// column virtualise.
    fn render_kanban_card(
        &self,
        ix: usize,
        row: &Row,
        columns: &[Column],
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let path = row.file.path.clone();
        let title = row
            .cells
            .first()
            .map(cell_text)
            // A view whose columns came out empty has nothing to name a card
            // with; the file's own base name is the only thing left, and is
            // itself **unprobed**.
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| row.file.basename.clone());
        let title_is_link = match (columns.first(), row.cells.first()) {
            (Some(column), Some(cell)) => renders_as_link(column, cell),
            _ => false,
        };
        let mut card = div()
            .id(("bases-kanban-card", ix))
            .flex()
            .flex_col()
            .w_full()
            .h(px(self.kanban.card_height))
            .overflow_hidden()
            .rounded_md()
            .border_1()
            .border_color(theme.border)
            .bg(theme.bg)
            .cursor_pointer()
            .hover(|s| s.bg(theme.bg_alt))
            .on_click(cx.listener(move |this, _, _, cx| this.open(path.clone(), cx)));
        if let Some(height) = self.kanban.image_height {
            card = card.child(self.render_kanban_image(row, height, theme));
        }
        card = card.child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.))
                .w_full()
                .p(px(CELL_PAD))
                .child(
                    div()
                        .flex_shrink_0()
                        .h(px(CARD_TITLE_HEIGHT))
                        .w_full()
                        .flex()
                        .items_center()
                        .overflow_hidden()
                        .text_size(px(FONT_UI_SMALL))
                        .text_color(if title_is_link {
                            theme.link
                        } else {
                            theme.text
                        })
                        .child(
                            div()
                                .min_w_0()
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .text_ellipsis()
                                .child(SharedString::from(title)),
                        ),
                )
                .children(
                    row.cells
                        .iter()
                        .enumerate()
                        .skip(1)
                        .map(|(c, cell)| {
                            let line = div()
                                .flex_shrink_0()
                                .h(px(CARD_LINE_HEIGHT))
                                .w_full()
                                .flex()
                                .flex_row()
                                .items_center()
                                .overflow_hidden()
                                .text_size(px(FONT_UI_SMALLER));
                            match cell.kind {
                                // A boolean is a disabled checkbox, so there
                                // is no text even when the value is `true`.
                                CellKind::Checkbox => line.child(checkbox(
                                    matches!(cell.value, Value::Bool(true)),
                                    theme,
                                )),
                                CellKind::Number | CellKind::Text => {
                                    let link = columns
                                        .get(c)
                                        .is_some_and(|column| renders_as_link(column, cell));
                                    line.text_color(if link {
                                        theme.link
                                    } else {
                                        theme.text_muted
                                    })
                                    .child(
                                        div()
                                            .min_w_0()
                                            .whitespace_nowrap()
                                            .overflow_hidden()
                                            .text_ellipsis()
                                            .child(SharedString::from(cell_text(cell))),
                                    )
                                }
                            }
                        })
                        .collect::<Vec<_>>(),
                ),
        );
        // The gap lives inside the item: a uniform list measures one item and
        // lays the rest out on that measurement, so a margin between items is
        // not something it can see.
        div()
            .w_full()
            .h(px(self.kanban.card_height + CARD_GAP))
            .child(card)
            .into_any_element()
    }

    /// A card's image box.
    ///
    /// The box is drawn whether or not the row resolved one, so every card
    /// stays the same height. A URL is **not** drawn: [`gpui::img`] would have
    /// to fetch it over the network, which an offline app does not do, so an
    /// `image` property naming `https://…` leaves the box empty.
    fn render_kanban_image(&self, row: &Row, height: f32, theme: &Theme) -> AnyElement {
        let file = match (row.image.as_ref(), self.vault()) {
            (Some(cell), Some(vault)) => match image_ref(cell, &row.file.path, vault.as_ref()) {
                Some(ImageRef::File(path)) => Some(path),
                Some(ImageRef::Url(_)) | None => None,
            },
            _ => None,
        };
        div()
            .flex_shrink_0()
            .w_full()
            .h(px(height))
            .overflow_hidden()
            .bg(theme.bg_secondary)
            .children(file.map(|path| {
                img(path)
                    .size_full()
                    .object_fit(match self.kanban.image_fit {
                        ImageFit::Cover => ObjectFit::Cover,
                        ImageFit::Contain => ObjectFit::Contain,
                    })
                    .into_any_element()
            }))
            .into_any_element()
    }
}

/// A column's header: what the group is called, and how many cards it holds.
///
/// **Unprobed**: neither the text nor the count's place was ever read off a
/// running board. The shape is the table layout's own group header — the key
/// in 13 px with the count as a faint flair beside it — so a board and a
/// grouped table name a group the same way.
fn column_header(title: Option<&str>, rows: usize, theme: &Theme) -> AnyElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .flex_shrink_0()
        .w_full()
        .h(px(COLUMN_HEADER_HEIGHT))
        .px(px(CELL_PAD))
        .gap(px(8.))
        .children(title.map(|title| {
            div()
                .min_w_0()
                .whitespace_nowrap()
                .overflow_hidden()
                .text_ellipsis()
                .text_size(px(FONT_UI_SMALL))
                .text_color(theme.text)
                .child(SharedString::from(title.to_string()))
        }))
        .child(
            div()
                .flex_shrink_0()
                .text_size(px(FONT_UI_SMALLER))
                .text_color(theme.text_faint)
                .child(SharedString::from(rows.to_string())),
        )
        .into_any_element()
}

/// The board's columns: one per group, in the engine's own group order.
///
/// Nothing here orders or groups anything. `groupBy`, `groupOrder` and the
/// direction are all [`silica_bases::table::build`]'s, and a view with no
/// `groupBy` arrives as one group of every row — so an ungrouped board is one
/// column, with the same cards a table would list. A grouped table whose
/// groups somehow came back empty falls back to one column of the flat rows,
/// the way the table's own body does, so nothing disappears.
pub fn board_columns(table: &Table) -> Vec<BoardColumn> {
    if table.groups.is_empty() {
        if table.rows.is_empty() {
            return Vec::new();
        }
        return vec![BoardColumn {
            key: None,
            rows: table.rows.len(),
            cards: UniformListScrollHandle::new(),
        }];
    }
    table
        .groups
        .iter()
        .map(|group| BoardColumn {
            key: group.key.clone(),
            rows: group.rows.len(),
            cards: UniformListScrollHandle::new(),
        })
        .collect()
}

/// The rows column `g` holds, through its group — or the flat rows under the
/// fallback [`board_columns`] makes for a table with no groups at all.
pub fn rows_of(table: &Table, g: usize) -> &[Row] {
    match table.groups.get(g) {
        Some(group) => &group.rows,
        None if g == 0 => &table.rows,
        None => &[],
    }
}

/// What a column's header says the group is.
///
/// A key is its own text. A `None` key is the no-value group when the view
/// groups — [`EMPTY_GROUP`] — and is nothing at all when it does not: the one
/// column of an ungrouped board is the whole table and has no group to name.
pub fn column_title(key: Option<&str>, grouped: bool) -> Option<&str> {
    match (key, grouped) {
        (Some(key), _) => Some(key),
        (None, true) => Some(EMPTY_GROUP),
        (None, false) => None,
    }
}

/// How wide a column is drawn, from `columnWidth`.
///
/// The key is taken as written — a hand-written value is never clamped, the
/// way the table takes `columnSize` as written — and only floored at
/// [`MIN_COLUMN_WIDTH`] so a zero or a negative draws something.
pub fn column_width(layout: &KanbanLayout) -> f32 {
    (layout.column_width as f32).max(MIN_COLUMN_WIDTH)
}

/// How wide a card is drawn: the column less the padding around its cards.
pub fn card_width(column_width: f32) -> f32 {
    (column_width - 2. * COLUMN_PAD).max(1.)
}

/// How tall a card's image box is, or `None` when the view names no `image`
/// property — a card then has no image box at all rather than an empty one.
///
/// **Unprobed**: `imageAspectRatio` is observed as a key, a range and a
/// default, but no view carrying one was ever rendered, so what the number
/// means is a reading. It is read as a ratio of **width to height**, which is
/// what the word means in CSS and what the [`cards`](crate::bases::cards)
/// layout reads the same key as, so the two layouts agree: the box is
/// `card_width / imageAspectRatio` tall, and the kanban default of 0.5 is
/// therefore a portrait box twice as tall as a card is wide — which is the
/// shape a narrow, tall column has room for, and the reason its default
/// differs from the cards layout's 1. A ratio of zero or less would have no
/// height at all and is floored.
pub fn image_height(card_width: f32, layout: &KanbanLayout) -> Option<f32> {
    layout.image.as_ref().map(|_| {
        let ratio = layout.image_aspect_ratio as f32;
        if ratio > 0. {
            (card_width / ratio).max(1.)
        } else {
            card_width
        }
    })
}

/// How tall every card of a board is.
///
/// One height for the whole board is what lets a column be a uniform list,
/// and it falls out of the card drawing the same cells for every row: the
/// image box when the view names one, then the title's line, then a line per
/// remaining cell, inside the card's padding and its hairline border — gpui
/// sizes a box the way CSS's `border-box` does, so both are part of the
/// height. A view with no columns still has a title line, which is where the
/// file's own name goes.
pub fn card_height(cells: usize, image_height: Option<f32>) -> f32 {
    let lines = cells.max(1);
    image_height.unwrap_or(0.)
        + 2. * CARD_BORDER
        + 2. * CELL_PAD
        + CARD_TITLE_HEIGHT
        + (lines - 1) as f32 * CARD_LINE_HEIGHT
}

#[cfg(test)]
mod tests {
    use super::*;
    use silica_bases::{Cell, ColumnSource, FileData, Group};
    use std::sync::Arc;

    fn row(path: &str) -> Row {
        Row {
            file: Arc::new(FileData {
                path: path.to_string(),
                name: path.to_string(),
                basename: path.trim_end_matches(".md").to_string(),
                ..FileData::default()
            }),
            cells: vec![Cell {
                value: Value::Str(path.to_string()),
                error: None,
                kind: CellKind::Text,
                text: path.to_string(),
            }],
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

    fn group(key: Option<&str>, rows: Vec<Row>) -> Group {
        Group {
            key: key.map(str::to_string),
            rows,
        }
    }

    #[test]
    fn a_board_is_one_column_a_group() {
        let (a, b, c) = (row("A.md"), row("B.md"), row("C.md"));
        // The order is the engine's: the no-value group leads under an
        // ascending `groupBy`, and this layout never reorders it.
        let table = table_of(
            vec![a.clone(), b.clone(), c.clone()],
            vec![
                group(None, vec![a.clone()]),
                group(Some("todo"), vec![b.clone(), c.clone()]),
            ],
        );
        let columns = board_columns(&table);
        assert_eq!(columns.len(), 2);
        assert_eq!(columns[0].key, None);
        assert_eq!(columns[0].rows, 1);
        assert_eq!(columns[1].key.as_deref(), Some("todo"));
        assert_eq!(columns[1].rows, 2);
        assert_eq!(rows_of(&table, 1).to_vec(), vec![b, c]);
    }

    #[test]
    fn a_view_with_no_group_by_is_one_column_of_every_row() {
        let rows = vec![row("A.md"), row("B.md")];
        // What the engine hands over for an ungrouped view: one group, no key.
        let table = table_of(rows.clone(), vec![group(None, rows.clone())]);
        let columns = board_columns(&table);
        assert_eq!(columns.len(), 1);
        assert_eq!(columns[0].rows, 2);
        assert_eq!(rows_of(&table, 0), rows.as_slice());
    }

    #[test]
    fn a_table_with_no_groups_still_puts_its_rows_in_a_column() {
        let rows = vec![row("A.md")];
        let table = table_of(rows.clone(), Vec::new());
        let columns = board_columns(&table);
        assert_eq!(columns.len(), 1);
        assert_eq!(columns[0].rows, 1);
        assert_eq!(rows_of(&table, 0), rows.as_slice());
        assert!(rows_of(&table, 1).is_empty());
    }

    #[test]
    fn a_view_that_resolved_to_nothing_has_no_columns() {
        let table = table_of(Vec::new(), Vec::new());
        assert!(board_columns(&table).is_empty());
        // And the empty group an ungrouped view hands over is still a column,
        // which is what the render reads the row count for.
        let table = table_of(Vec::new(), vec![group(None, Vec::new())]);
        assert_eq!(board_columns(&table).len(), 1);
        assert_eq!(board_columns(&table)[0].rows, 0);
    }

    #[test]
    fn a_null_group_reads_as_empty_only_when_the_view_groups() {
        assert_eq!(column_title(Some("todo"), true), Some("todo"));
        assert_eq!(column_title(None, true), Some(EMPTY_GROUP));
        assert_eq!(column_title(None, false), None);
        // An ungrouped board's one column is still named when the key is
        // somehow there, which only a hand-built table can do.
        assert_eq!(column_title(Some("todo"), false), Some("todo"));
    }

    #[test]
    fn a_column_is_as_wide_as_the_view_asks() {
        assert_eq!(column_width(&KanbanLayout::default()), 280.);
        let wide = KanbanLayout {
            column_width: 440.,
            ..KanbanLayout::default()
        };
        assert_eq!(column_width(&wide), 440.);
        // Taken as written past the slider's own 500, floored above zero.
        let silly = KanbanLayout {
            column_width: 4000.,
            ..KanbanLayout::default()
        };
        assert_eq!(column_width(&silly), 4000.);
        let zero = KanbanLayout {
            column_width: 0.,
            ..KanbanLayout::default()
        };
        assert_eq!(column_width(&zero), MIN_COLUMN_WIDTH);
        assert_eq!(card_width(280.), 280. - 2. * COLUMN_PAD);
    }

    #[test]
    fn a_card_reserves_an_image_box_only_when_the_view_names_one() {
        let bare = KanbanLayout::default();
        assert_eq!(image_height(264., &bare), None);
        let with_image = KanbanLayout {
            image: Some("note.cover".to_string()),
            ..KanbanLayout::default()
        };
        // The default ratio is 0.5, read as width over height, so the box is
        // twice as tall as the card is wide.
        assert_eq!(image_height(264., &with_image), Some(528.));
        let square = KanbanLayout {
            image: Some("note.cover".to_string()),
            image_aspect_ratio: 1.,
            ..KanbanLayout::default()
        };
        assert_eq!(image_height(264., &square), Some(264.));
    }

    #[test]
    fn every_card_of_a_board_is_the_same_height() {
        // One column is the title's line and nothing else, inside the card's
        // padding and its border.
        assert_eq!(
            card_height(1, None),
            2. * CARD_BORDER + 2. * CELL_PAD + CARD_TITLE_HEIGHT
        );
        // A view with no columns still has the title's line.
        assert_eq!(card_height(0, None), card_height(1, None));
        // Every further column is one more line, whether its cell has text or
        // not — which is what keeps the cards one height.
        assert_eq!(
            card_height(3, None),
            card_height(1, None) + 2. * CARD_LINE_HEIGHT
        );
        assert_eq!(card_height(1, Some(528.)), card_height(1, None) + 528.);
    }

    #[test]
    fn a_boards_state_reports_its_columns_with_their_keys_and_counts() {
        let (a, b) = (row("A.md"), row("B.md"));
        let table = table_of(
            vec![a.clone(), b.clone()],
            vec![group(None, vec![a]), group(Some("todo"), vec![b])],
        );
        let state = KanbanState {
            columns: board_columns(&table),
            grouped: true,
            ..KanbanState::default()
        };
        let json = state.state_json();
        assert_eq!(json["columns"][0]["key"], Json::Null);
        assert_eq!(json["columns"][0]["rows"], 1);
        assert_eq!(json["columns"][1]["key"], "todo");
        assert_eq!(json["columns"][1]["rows"], 1);
        assert_eq!(json["column_width"], 280.);
        assert_eq!(json["image_height"], Json::Null);
    }
}
