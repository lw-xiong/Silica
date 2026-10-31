//! The `cards` layout: a grid of cards, one per row.
//!
//! Its keys are observed (`plugin.getRegistrations()` and the real Configure
//! view page; see [`silica_bases::layout`] and
//! [[Specification#Observed Bases write behavior, Obsidian 1.14.5 (installer build 1.13.7)]]):
//! `cardSize` (50 to 800 step 10, default 200), `image` (a property id),
//! `imageFit` (`""` Cover or `contain`) and `imageAspectRatio` (0.25 to 2.5
//! step 0.05, default 1). The rows, the columns and the grouping are the
//! table's — every layout draws the same resolved
//! [`Table`](silica_bases::Table) — so a card shows the same cells a row
//! would, in a box of its own.
//!
//! **Unprobed**: no cards view was ever rendered in the run, so the card's
//! own box is not observed at all. The keys and their ranges are, and
//! everything the keys do not say is a reading, marked as one below:
//!
//! - a card is [`CardsLayout::card_size`] wide and its image box
//!   `card_size / imageAspectRatio` tall, because the key is a *size* and a
//!   ratio of width to height — [`metrics`];
//! - the **title** is the row's first cell, which is `order`'s first id and
//!   so usually `file.name`, and clicking the card opens the row's file
//!   — [`title_of`];
//! - the **body** is the rest of the row's cells, one per line, with the
//!   cells that rendered nothing left out — [`body_of`];
//! - a **grouped** view draws the same group header the table draws, above
//!   the cards of that group, in `Table::groups` order (the no-value group
//!   leads under an ascending `groupBy`); an ungrouped view is one section
//!   holding every row — [`sections`].
//!
//! [`title_of`], [`body_of`], [`renders_title_as_link`] and [`group_header`]
//! are what the [`list`](crate::bases::list) layout reads a line's title,
//! body and group header from as well: the two layouts differ only in their
//! box. They live here rather than in [`cells`](crate::bases::cells), which
//! is where shared drawing belongs, only because this build grew the two
//! layouts at once.
//!
//! Obsidian's own grid is virtualised; this one wraps inside a scroll box,
//! which cannot be, so it draws at most [`MAX_CARDS`] cards and reports
//! `capped` in its state rather than laying out a 20 000-note vault.

use std::path::PathBuf;

use crate::bases::cells::{
    body_of, group_header, image_ref, renders_title_as_link, title_of, ImageRef, CELL_PAD,
};
use crate::bases::view::BasesView;
use crate::theme::{Theme, FONT_UI_SMALL, FONT_UI_SMALLER};
use gpui::{div, img, prelude::*, px, AnyElement, Context, ObjectFit, ScrollHandle, SharedString};
use serde_json::{json, Value as Json};
use silica_bases::{CardsLayout, Column, ImageFit, Row, Table};

/// The gap between two cards, and the padding around the grid.
///
/// **Unprobed**: no cards view was measured. 8 px is the shell's own gap
/// (`CELL_PAD`, a dock tab header's padding) and 16 px the padding the
/// reading view gives a block.
pub const CARD_GAP: f32 = 8.;
pub const GRID_PAD: f32 = 16.;

/// How many cards the grid lays out before it stops.
///
/// Not a behavior: a limit this build has and Obsidian does not, because
/// Obsidian virtualises its grid and a wrapping flex box cannot be
/// virtualised without knowing the pane's width before layout. The view's own
/// `limit` key is the real bound; this is the backstop that keeps a 20 000-row
/// base from laying out 20 000 cards a frame.
pub const MAX_CARDS: usize = 500;

/// How many card titles [`CardsState::state_json`] lists.
const STATE_CARDS: usize = 20;

/// The measurements a card is drawn at, from the view's own keys.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CardMetrics {
    /// `cardSize`: the width of one card.
    pub width: f32,
    /// `cardSize / imageAspectRatio`: the height of the image box, the ratio
    /// being width to height.
    pub image_height: f32,
}

impl Default for CardMetrics {
    fn default() -> CardMetrics {
        metrics(&CardsLayout::default())
    }
}

/// A `groupBy` header above a section of cards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupHeader {
    /// The group's rendered key; `None` is Obsidian's no-value group.
    pub key: Option<String>,
    pub rows: usize,
}

/// One run of cards under at most one group header.
#[derive(Debug, Clone, PartialEq)]
pub struct Section<'a> {
    /// `None` when the view does not group, which is one section of
    /// everything.
    pub header: Option<GroupHeader>,
    pub rows: &'a [Row],
}

/// One card, as the rebuild resolved it: nothing here needs the table again,
/// so a frame draws cards without touching the query.
#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    /// The file the card opens.
    pub path: String,
    pub title: String,
    /// Whether the title draws in the link colour, which is what
    /// [`renders_as_link`] says of the first column.
    pub link: bool,
    pub body: Vec<String>,
    /// The image file the view's `image` property named, already resolved
    /// against the vault.
    pub image: Option<PathBuf>,
    /// The `image` property named a URL. gpui would have to fetch it, so
    /// nothing is drawn and the count is reported instead.
    pub remote: bool,
}

/// One section of the drawn grid.
#[derive(Debug, Clone, PartialEq)]
pub struct CardSection {
    pub header: Option<GroupHeader>,
    pub cards: Vec<Card>,
    /// The flat index of this section's first card, so a card's element id is
    /// stable across sections.
    pub first: usize,
}

/// What the cards layout keeps between renders.
#[derive(Default)]
pub struct CardsState {
    /// The grid scrolls as one box; cards are laid out by wrapping rather
    /// than virtualised, which is why this is a scroll handle and not a
    /// [`gpui::ListState`].
    pub scroll: ScrollHandle,
    /// How many cards the grid holds, which is the table's row count once
    /// the limit has been applied.
    pub cards: usize,
    /// The grid, in draw order.
    pub sections: Vec<CardSection>,
    pub metrics: CardMetrics,
    pub fit: ImageFit,
    /// Whether the view names an `image` property at all; with none, a card
    /// has no image box rather than an empty one.
    pub has_image: bool,
    /// How many drawn cards resolved to a file in the vault.
    pub images: usize,
    /// How many named a URL, which this build does not load.
    pub remote: usize,
    /// Whether [`MAX_CARDS`] cut the grid short.
    pub capped: bool,
}

impl CardsState {
    pub fn state_json(&self) -> Json {
        let sections: Vec<Json> = self
            .sections
            .iter()
            .map(|s| {
                json!({
                    "key": s.header.as_ref().and_then(|h| h.key.clone()),
                    "cards": s.cards.len(),
                })
            })
            .collect();
        let titles: Vec<&str> = self
            .sections
            .iter()
            .flat_map(|s| s.cards.iter())
            .take(STATE_CARDS)
            .map(|c| c.title.as_str())
            .collect();
        json!({
            "cards": self.cards,
            "drawn": self.drawn(),
            "capped": self.capped,
            "card_size": self.metrics.width,
            "image_height": self.metrics.image_height,
            "image_fit": match self.fit {
                ImageFit::Cover => "cover",
                ImageFit::Contain => "contain",
            },
            "has_image": self.has_image,
            "images": self.images,
            "remote_images": self.remote,
            "sections": sections,
            "titles": titles,
        })
    }

    /// How many cards the grid actually laid out, which is the row count
    /// unless [`MAX_CARDS`] cut it short.
    pub fn drawn(&self) -> usize {
        self.sections.iter().map(|s| s.cards.len()).sum()
    }
}

impl BasesView {
    pub(super) fn rebuild_cards(&mut self) {
        let layout = match self.layout() {
            silica_bases::Layout::Cards(cards) => cards.clone(),
            _ => CardsLayout::default(),
        };
        let grouped = self.view().is_some_and(|v| v.group_by.is_some());
        let vault = self.vault().cloned();
        let mut built: Vec<CardSection> = Vec::new();
        let mut rows = 0;
        let mut capped = false;
        if let Some(table) = self.table() {
            rows = table.rows.len();
            let columns = &table.columns;
            let mut drawn = 0;
            for section in sections(table, grouped) {
                if drawn >= MAX_CARDS {
                    capped = true;
                    break;
                }
                let room = MAX_CARDS - drawn;
                let take = section.rows.len().min(room);
                capped |= take < section.rows.len();
                let cards: Vec<Card> = section.rows[..take]
                    .iter()
                    .map(|row| {
                        let mut card = card_of(row, columns);
                        // Only a view that names an `image` property has one;
                        // `Row::image` is `None` otherwise.
                        if let (Some(cell), Some(vault)) = (row.image.as_ref(), vault.as_deref()) {
                            match image_ref(cell, &row.file.path, vault) {
                                Some(ImageRef::File(path)) => card.image = Some(path),
                                Some(ImageRef::Url(_)) => card.remote = true,
                                None => {}
                            }
                        }
                        card
                    })
                    .collect();
                let first = drawn;
                drawn += cards.len();
                built.push(CardSection {
                    header: section.header,
                    cards,
                    first,
                });
            }
        }
        self.cards.images = built
            .iter()
            .flat_map(|s| s.cards.iter())
            .filter(|c| c.image.is_some())
            .count();
        self.cards.remote = built
            .iter()
            .flat_map(|s| s.cards.iter())
            .filter(|c| c.remote)
            .count();
        self.cards.sections = built;
        self.cards.cards = rows;
        self.cards.capped = capped;
        self.cards.metrics = metrics(&layout);
        self.cards.fit = layout.image_fit;
        self.cards.has_image = layout.image.is_some();
    }

    /// Draws the grid: one section per group, each a wrapping run of cards,
    /// all inside one scroll box.
    pub(super) fn render_cards(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        if self.table().is_none() {
            return Vec::new();
        }
        if self.cards.cards == 0 {
            return vec![self.render_empty(theme)];
        }
        let mut body = div()
            .id("bases-cards")
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.))
            .w_full()
            .overflow_y_scroll()
            .track_scroll(&self.cards.scroll);
        for (s, section) in self.cards.sections.iter().enumerate() {
            if let Some(header) = section.header.as_ref() {
                body = body.child(group_header(s, header.key.as_deref(), header.rows, theme));
            }
            let first = section.first;
            body = body.child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .w_full()
                    .gap(px(CARD_GAP))
                    .p(px(GRID_PAD))
                    .children(
                        section
                            .cards
                            .iter()
                            .enumerate()
                            .map(|(c, card)| self.render_card(first + c, card, theme, cx)),
                    ),
            );
        }
        vec![body.into_any_element()]
    }

    /// One card: its image box when the view names an `image` property, its
    /// title, and the rest of the row's cells beneath it.
    fn render_card(
        &self,
        ix: usize,
        card: &Card,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let path = card.path.clone();
        let metrics = self.cards.metrics;
        let mut box_ = div()
            .id(("bases-card", ix))
            .flex()
            .flex_col()
            .flex_shrink_0()
            .w(px(metrics.width))
            .overflow_hidden()
            .rounded(px(6.))
            .border_1()
            .border_color(theme.border)
            .bg(theme.bg_secondary)
            .cursor_pointer()
            .hover(|s| s.bg(theme.bg_alt))
            .on_click(cx.listener(move |this, _, _, cx| this.open(path.clone(), cx)));
        if self.cards.has_image {
            // The box is drawn whether or not the property answered, so a
            // row with no image does not shuffle the grid out of line.
            let mut frame = div()
                .w_full()
                .h(px(metrics.image_height))
                .flex_shrink_0()
                .overflow_hidden()
                .bg(theme.bg_alt);
            if let Some(file) = card.image.clone() {
                frame = frame.child(img(file).size_full().object_fit(match self.cards.fit {
                    ImageFit::Cover => ObjectFit::Cover,
                    ImageFit::Contain => ObjectFit::Contain,
                }));
            }
            box_ = box_.child(frame);
        }
        box_.child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .gap(px(2.))
                .p(px(CELL_PAD))
                .child(
                    div()
                        .min_w_0()
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .text_ellipsis()
                        .text_size(px(FONT_UI_SMALL))
                        .text_color(if card.link { theme.link } else { theme.text })
                        .child(SharedString::from(card.title.clone())),
                )
                .children(card.body.iter().map(|value| {
                    div()
                        .min_w_0()
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .text_ellipsis()
                        .text_size(px(FONT_UI_SMALLER))
                        .text_color(theme.text_muted)
                        .child(SharedString::from(value.clone()))
                })),
        )
        .into_any_element()
    }
}

/// The measurements a card is drawn at.
///
/// **Unprobed**, and the plain reading of two observed sliders: `cardSize` is
/// the card's width and `imageAspectRatio` its image box's width to height,
/// so a ratio of 0.5 — the kanban default — is an image box twice as tall as
/// the card is wide. Both are clamped to their own slider's range, because
/// that is the only range the control can write; the reader itself never
/// clamps, so a hand-written `cardSize: 4000` survives a round trip and is
/// merely drawn at 800.
pub fn metrics(layout: &CardsLayout) -> CardMetrics {
    let (min_size, max_size, _) = silica_bases::layout::CARD_SIZE_RANGE;
    let (min_ratio, max_ratio, _) = silica_bases::layout::ASPECT_RANGE;
    let width = layout.card_size.clamp(min_size, max_size) as f32;
    let ratio = layout.image_aspect_ratio.clamp(min_ratio, max_ratio) as f32;
    CardMetrics {
        width,
        image_height: (width / ratio).round(),
    }
}

/// The sections of a resolved table: one per group when the view groups, and
/// one holding every row when it does not.
///
/// A view with no `groupBy` still has one group holding the same rows, so the
/// caller says whether the view groups rather than this guessing from the
/// shape; a grouped view whose groups came back empty falls back to the flat
/// rows so nothing disappears. Both are the table layout's own rules, which
/// is the point: every layout draws the same resolved table.
pub fn sections(table: &Table, grouped: bool) -> Vec<Section<'_>> {
    if grouped && !table.groups.is_empty() {
        table
            .groups
            .iter()
            .map(|group| Section {
                header: Some(GroupHeader {
                    key: group.key.clone(),
                    rows: group.rows.len(),
                }),
                rows: &group.rows,
            })
            .collect()
    } else {
        vec![Section {
            header: None,
            rows: &table.rows,
        }]
    }
}

/// One card from one row.
pub fn card_of(row: &Row, columns: &[Column]) -> Card {
    Card {
        path: row.file.path.clone(),
        title: title_of(row),
        link: renders_title_as_link(columns, row),
        body: body_of(row),
        image: None,
        remote: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    fn cell(text: &str) -> Cell {
        Cell {
            value: Value::Str(text.to_string()),
            error: None,
            kind: CellKind::Text,
            text: text.to_string(),
        }
    }

    fn row(path: &str, cells: &[&str]) -> Row {
        Row {
            file: file(path),
            cells: cells.iter().map(|t| cell(t)).collect(),
            image: None,
        }
    }

    fn columns(ids: &[&str]) -> Vec<Column> {
        ids.iter()
            .map(|id| Column {
                id: id.to_string(),
                display: id.to_string(),
                source: match id.strip_prefix("file.") {
                    Some(f) => ColumnSource::File(f.to_string()),
                    None => ColumnSource::Property(id.to_string()),
                },
            })
            .collect()
    }

    fn table_of(rows: Vec<Row>, groups: Vec<Group>, ids: &[&str]) -> Table {
        Table {
            columns: columns(ids),
            rows,
            groups,
            limited: false,
            unknown_kind: None,
        }
    }

    #[test]
    fn a_card_takes_its_title_from_the_first_column() {
        let r = row("A.md", &["Alpha", "todo"]);
        let card = card_of(&r, &columns(&["file.name", "status"]));
        assert_eq!(card.title, "Alpha");
        assert_eq!(card.path, "A.md");
        assert!(card.link, "a file.name column draws as a link");
    }

    #[test]
    fn a_card_whose_first_cell_rendered_nothing_falls_back_to_the_file() {
        let r = row("A.md", &["", "todo"]);
        assert_eq!(title_of(&r), "A.md");
        let bare = Row {
            file: file("B.md"),
            cells: Vec::new(),
            image: None,
        };
        assert_eq!(title_of(&bare), "B.md");
        assert!(renders_title_as_link(&[], &bare), "a file name is a link");
    }

    #[test]
    fn a_cards_body_is_the_rest_of_the_row_without_its_empty_cells() {
        let r = row("A.md", &["Alpha", "todo", "", "0.02"]);
        assert_eq!(body_of(&r), vec!["todo".to_string(), "0.02".to_string()]);
    }

    #[test]
    fn a_title_that_is_not_a_file_column_is_not_a_link() {
        let r = row("A.md", &["todo", "Alpha"]);
        assert!(!renders_title_as_link(
            &columns(&["status", "file.name"]),
            &r
        ));
    }

    #[test]
    fn an_image_box_is_as_tall_as_the_aspect_ratio_makes_it() {
        let square = metrics(&CardsLayout::default());
        assert_eq!(square.width, 200.);
        assert_eq!(square.image_height, 200.);
        let tall = metrics(&CardsLayout {
            image_aspect_ratio: 0.5,
            ..CardsLayout::default()
        });
        assert_eq!(tall.image_height, 400.);
        let wide = metrics(&CardsLayout {
            card_size: 300.,
            image_aspect_ratio: 2.,
            ..CardsLayout::default()
        });
        assert_eq!(wide.width, 300.);
        assert_eq!(wide.image_height, 150.);
    }

    #[test]
    fn a_card_size_outside_the_sliders_range_is_drawn_at_its_bound() {
        // The reader keeps `cardSize: 4000` as the file wrote it; only the
        // draw bounds it, at the slider's own maximum.
        let huge = metrics(&CardsLayout {
            card_size: 4000.,
            ..CardsLayout::default()
        });
        assert_eq!(huge.width, 800.);
        let tiny = metrics(&CardsLayout {
            card_size: 1.,
            image_aspect_ratio: 0.,
            ..CardsLayout::default()
        });
        assert_eq!(tiny.width, 50.);
        assert_eq!(tiny.image_height, 200., "a zero ratio is the slider's 0.25");
    }

    #[test]
    fn an_ungrouped_view_is_one_section_of_every_row() {
        let rows = vec![row("A.md", &["A"]), row("B.md", &["B"])];
        let table = table_of(
            rows.clone(),
            vec![Group {
                key: None,
                rows: rows.clone(),
            }],
            &["file.name"],
        );
        let got = sections(&table, false);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].header, None);
        assert_eq!(got[0].rows.len(), 2);
    }

    #[test]
    fn a_grouped_view_is_one_section_a_group_in_the_tables_own_order() {
        let a = row("A.md", &["A"]);
        let b = row("B.md", &["B"]);
        let table = table_of(
            vec![a.clone(), b.clone()],
            vec![
                // The no-value group leads under an ascending `groupBy`,
                // which is observed; this only keeps the table's order.
                Group {
                    key: None,
                    rows: vec![a.clone()],
                },
                Group {
                    key: Some("todo".to_string()),
                    rows: vec![b.clone()],
                },
            ],
            &["file.name"],
        );
        let got = sections(&table, true);
        assert_eq!(got.len(), 2);
        assert_eq!(
            got[0].header,
            Some(GroupHeader { key: None, rows: 1 }),
            "the no-value group leads"
        );
        assert_eq!(
            got[1].header,
            Some(GroupHeader {
                key: Some("todo".to_string()),
                rows: 1
            })
        );
        assert_eq!(got[1].rows, &[b]);
    }

    #[test]
    fn a_grouped_view_with_no_groups_falls_back_to_the_flat_rows() {
        let table = table_of(vec![row("A.md", &["A"])], Vec::new(), &["file.name"]);
        let got = sections(&table, true);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].header, None);
        assert_eq!(got[0].rows.len(), 1);
    }
}
