//! An embedded base: `![[x.base]]` inside a note.
//!
//! The reading view resolves the embed through [`resolve`] and draws what it
//! answers, which is the same [`Resolved`](crate::bases::rows::Resolved) the
//! view itself draws — one engine, one set of rules, whether a base is open
//! in a leaf or embedded in a note.
//!
//! What the **rows** look like is observed, because it is the table view's own
//! observation (`tools/obsidian-probe/results/2026-10-25-bases.json`; the
//! prose is in
//! [[Specification#Observed Bases table view, Obsidian 1.14.5 (installer build 1.13.7)]]),
//! and an embed draws them exactly as [`table`](crate::bases::table) does:
//! the 30 px header row of display names, a row per row, a cell per
//! [`CellKind`] — `text`, a right-reading `number`, and a **disabled
//! checkbox** whose text is empty even when the value is `true` — and the
//! `file.name` column in the link colour. The two error shapes are observed
//! too, and an embed must not swallow them: a file that does not parse draws
//! [`PARSE_ERROR_BANNER`] and the YAML library's own message, and a view whose
//! `type` is outside the four registered layouts draws "Unknown view type: x"
//! and no rows. A view that resolved to no rows keeps its header row, with no
//! banner, the way `BadFilter.base` does in the real view.
//!
//! **Unprobed**: an embedded base was never captured in the run — it is named
//! in the write-behavior section's own unprobed list, along with what
//! `createAndEmbedBase` writes — so everything around those rows is this
//! build's reading rather than an observation:
//!
//! - the **box** is the shell's own embed box, the one a note embed draws,
//!   with the base's view name as its title and a click on the title opening
//!   the `.base`;
//! - **no toolbar**: the open view's chrome configures the view and writes the
//!   file, and an embed has no view state of its own to configure (it always
//!   shows the file's *first* view, because a leaf remembers which view it was
//!   on and an embed has nowhere to remember one);
//! - the **height** is the rows', capped at [`EMBED_ROWS`] with a line
//!   counting what is left: an embed is not scrolled and not virtualised, the
//!   note's own scroller being the only one;
//! - a **cards, list or kanban** view draws the same rows a table does, under
//!   its own name. Those layouts are the open view's; an embed says nothing
//!   about which one it is rather than claiming to be a table.

use crate::bases::cells::{cell_text, checkbox, renders_as_link, CELL_PAD};
use crate::bases::rows;
use crate::bases::table::{
    column_widths, stretches_last, HEADER_HEIGHT, MIN_COLUMN_WIDTH, ROW_HEIGHT,
};
use crate::reading::model::{EmbedKind, RBlock, Segment};
use crate::theme::{Theme, FONT_UI_SMALL};
use crate::workspace::VaultState;
use gpui::{div, prelude::*, px, AnyElement, App, SharedString, Window};
use serde_json::{json, Value as Json};
use silica_bases::{CellKind, Column, Layout, Row, Table, TableLayout, Value};
use silica_formats::base::PARSE_ERROR_BANNER;
use std::rc::Rc;

/// How many rows an embedded base draws.
///
/// **This build's choice.** An embed is not scrolled and not virtualised, so a
/// base over a 20 000-note vault would otherwise lay out 20 000 rows inside a
/// note's paragraph flow. 50 is a screenful and a half at [`ROW_HEIGHT`], and
/// what is left is counted on one line under the last row. Obsidian's own
/// embedded base was never captured, so neither the cap nor the line is
/// observed.
pub const EMBED_ROWS: usize = 50;

/// What a click inside an embedded base opens, by vault path: the base itself
/// for its title, a row's own file for a row. The reading view hands this
/// over as an internal link, which is how every other embed opens its target.
pub type OpenPath = Rc<dyn Fn(&str, &mut Window, &mut App)>;

/// A `.base` resolved for an embed: what the first view of the file shows.
#[derive(Debug, Clone, PartialEq)]
pub struct EmbeddedBase {
    /// The base's own vault path, which is what a click opens.
    pub path: String,
    /// The showing view's name, which is the title an embed draws.
    pub view_name: Option<String>,
    pub layout: Layout,
    pub table: Option<Table>,
    /// The parse failure, if the file has one: an embedded broken base says
    /// the same thing the view does.
    pub error: Option<String>,
}

/// Resolves the base at `path` for an embed, on its **first** view — a leaf
/// remembers which view it was on, an embed has nowhere to remember one.
pub fn resolve(state: &VaultState, path: &str) -> Option<EmbeddedBase> {
    let text = std::fs::read_to_string(state.vault().absolute(path)).ok()?;
    let resolved = rows::resolve(Some(state), Some(path), &text, 0);
    let view = resolved.base.as_ref().and_then(|b| b.views.first());
    Some(EmbeddedBase {
        path: path.to_string(),
        view_name: view.map(|v| v.name.clone()),
        layout: view.map(Layout::of).unwrap_or_default(),
        table: resolved.table,
        error: resolved.error,
    })
}

impl EmbeddedBase {
    /// The rows this embed draws, and how many it leaves undrawn.
    ///
    /// The whole count is the table's: a view's own `limit` has already
    /// truncated it (`Table::limited`), and [`EMBED_ROWS`] is this build's
    /// second cap on top of that.
    pub fn drawn_rows(&self) -> (&[Row], usize) {
        let rows: &[Row] = self.table.as_ref().map_or(&[], |t| &t.rows);
        let shown = rows.len().min(EMBED_ROWS);
        (&rows[..shown], rows.len() - shown)
    }

    /// The table layout's own keys, which are what sizes the columns — a
    /// `columnSize` the file carries is honoured in an embed too. A view of
    /// another layout has none, and its rows are drawn at the default widths.
    fn table_layout(&self) -> TableLayout {
        match &self.layout {
            Layout::Table(table) => table.clone(),
            _ => TableLayout::default(),
        }
    }
}

/// Draws an embedded base inside a note.
///
/// `open` is what a click does: the reading view follows an internal link,
/// which is how every other embed's title opens its target. The title opens
/// the `.base` and a row opens the file that row came from, as a row of the
/// open view does.
///
/// **Unprobed**, as the module note says: everything but the rows themselves
/// is this build's reading.
pub fn render(base: &EmbeddedBase, theme: &Theme, open: OpenPath) -> AnyElement {
    let title = base.view_name.clone().unwrap_or_else(|| base.path.clone());
    let body: Vec<AnyElement> = match (&base.error, &base.table) {
        // A file that does not parse says what the view says, banner and all.
        (Some(message), _) => vec![error_block(true, message, theme)],
        (None, Some(table)) => match &table.unknown_kind {
            // The file parsed; only the layout is one nothing can draw.
            Some(kind) => vec![error_block(
                false,
                &format!("Unknown view type: {kind}"),
                theme,
            )],
            None => table_body(base, table, theme, open.clone()),
        },
        // No table and no error is a base that was never resolved.
        (None, None) => Vec::new(),
    };
    let path = base.path.clone();
    div()
        .id(SharedString::from(format!("bases-embed-{}", base.path)))
        .my_2()
        .border_1()
        .border_color(theme.border)
        .rounded_md()
        .px_4()
        .py_3()
        .flex()
        .flex_col()
        .child(
            div()
                .id("bases-embed-title")
                .mb_2()
                .text_size(px(theme.base_font_size * 0.75))
                .text_color(theme.text_muted)
                .cursor_pointer()
                .hover(|s| s.text_color(theme.link))
                .on_click({
                    let open = open.clone();
                    move |_, window, cx| open(&path, window, cx)
                })
                .child(SharedString::from(format!("\u{2197} {title}"))),
        )
        .children(body)
        .into_any_element()
}

/// The header row, the rows under it, and the line counting the rows the cap
/// left out.
fn table_body(
    base: &EmbeddedBase,
    table: &Table,
    theme: &Theme,
    open: OpenPath,
) -> Vec<AnyElement> {
    let layout = base.table_layout();
    let widths = column_widths(&table.columns, &layout);
    // The same column metrics the open view draws: a `columnSize` the file
    // carries is honoured, an unsized last column takes the rest of the width,
    // and one the map names keeps the width it names.
    let stretch = stretches_last(&table.columns, &layout);
    let height = ROW_HEIGHT * layout.row_height.mult();
    let (rows, more) = base.drawn_rows();
    let mut out = vec![header_row(&table.columns, &widths, stretch, theme)];
    if rows.is_empty() {
        // Observed on `BadFilter.base`: a view that empties keeps its header
        // row and shows no banner, only the count.
        out.push(empty_row(theme));
        return out;
    }
    out.extend(rows.iter().enumerate().map(|(ix, row)| {
        row_element(
            ix,
            row,
            &table.columns,
            &widths,
            Metrics { height, stretch },
            theme,
            open.clone(),
        )
    }));
    if more > 0 {
        out.push(more_rows(more, theme));
    }
    out
}

/// The header row: one box per column, abutting, the last one taking the rest
/// of the width, over a hairline — the table layout's own header, 30 px tall.
fn header_row(columns: &[Column], widths: &[f32], stretch: bool, theme: &Theme) -> AnyElement {
    let last = columns.len().saturating_sub(1);
    div()
        .flex()
        .flex_row()
        .flex_shrink_0()
        .w_full()
        .h(px(HEADER_HEIGHT))
        .border_b_1()
        .border_color(theme.border)
        .children(columns.iter().enumerate().map(|(ix, col)| {
            let width = widths.get(ix).copied().unwrap_or(MIN_COLUMN_WIDTH);
            cell_box(ix, stretch && ix == last, width, theme)
                .text_color(theme.text_muted)
                .child(
                    div()
                        .min_w_0()
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(SharedString::from(col.display.clone())),
                )
        }))
        .into_any_element()
}

/// How tall a row is drawn and whether its last cell stretches: the two
/// measurements every row of one table shares.
#[derive(Debug, Clone, Copy)]
struct Metrics {
    height: f32,
    stretch: bool,
}

/// One row: a cell per column, opening the file it came from.
fn row_element(
    ix: usize,
    row: &Row,
    columns: &[Column],
    widths: &[f32],
    metrics: Metrics,
    theme: &Theme,
    open: OpenPath,
) -> AnyElement {
    let path = row.file.path.clone();
    let last = row.cells.len().saturating_sub(1);
    div()
        .id(("bases-embed-row", ix))
        .flex()
        .flex_row()
        .w_full()
        .h(px(metrics.height))
        .border_b_1()
        .border_color(theme.border)
        .cursor_pointer()
        .hover(|s| s.bg(theme.bg_alt))
        .on_click(move |_, window, cx| open(&path, window, cx))
        .children(row.cells.iter().enumerate().map(|(c, cell)| {
            let width = widths.get(c).copied().unwrap_or(MIN_COLUMN_WIDTH);
            let link = columns.get(c).is_some_and(|col| renders_as_link(col, cell));
            let box_ = cell_box(c, metrics.stretch && c == last, width, theme);
            match cell.kind {
                // A boolean is a disabled checkbox, so there is no text even
                // when the value is `true`.
                CellKind::Checkbox => {
                    box_.child(checkbox(matches!(cell.value, Value::Bool(true)), theme))
                }
                // **Unprobed**, as in the open view: no number column's
                // alignment was measured; right is the convention a number
                // column reads in.
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

/// One cell or header box: the column's width, the shared padding, and the
/// hairline between it and the column before it. `stretch` is the last
/// column's own, which takes the rest of the width when nothing sized it.
fn cell_box(ix: usize, stretch: bool, width: f32, theme: &Theme) -> gpui::Div {
    let box_ = div()
        .flex()
        .flex_row()
        .items_center()
        .h_full()
        .px(px(CELL_PAD))
        .overflow_hidden()
        .text_size(px(FONT_UI_SMALL))
        .when(ix > 0, |d| d.border_l_1().border_color(theme.border));
    if stretch {
        box_.flex_1().min_w(px(width))
    } else {
        box_.flex_shrink_0().w(px(width))
    }
}

/// A view that resolved to no rows.
///
/// Observed: the header row stays drawn and no banner appears. The count is
/// the observed wording for one — `BadFilter.base` reports "0 results" — which
/// an embed draws in its body because it has no result count in its chrome
/// to put it in. **Unprobed**: the box, which is the shell's own `pane-empty`
/// — 13 px faint and centred.
fn empty_row(theme: &Theme) -> AnyElement {
    div()
        .w_full()
        .flex()
        .justify_center()
        .py(px(8.))
        .text_size(px(FONT_UI_SMALL))
        .text_color(theme.text_faint)
        .child(SharedString::from("0 results"))
        .into_any_element()
}

/// The line under the last drawn row, counting the rows [`EMBED_ROWS`] left
/// out. **Unprobed**: the cap is this build's, so the line is too.
fn more_rows(more: usize, theme: &Theme) -> AnyElement {
    div()
        .w_full()
        .py(px(8.))
        .text_size(px(FONT_UI_SMALL))
        .text_color(theme.text_faint)
        .child(SharedString::from(if more == 1 {
            "1 more row".to_string()
        } else {
            format!("{more} more rows")
        }))
        .into_any_element()
}

/// `.bases-error` inside an embed: one paragraph per message, the parse
/// failure's banner first.
///
/// The same two shapes the view draws, and the message is kept line for line
/// in the mono family because the YAML library's own text carries a caret
/// under the offending column, which only lines up in a fixed pitch.
fn error_block(banner: bool, message: &str, theme: &Theme) -> AnyElement {
    let mut block = div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .text_size(px(theme.base_font_size))
        .text_color(theme.text);
    if banner {
        block = block.child(SharedString::from(PARSE_ERROR_BANNER));
    }
    block
        .child(
            div()
                .flex()
                .flex_col()
                .font_family(theme.mono_font.clone())
                .text_size(px(FONT_UI_SMALL))
                .text_color(theme.text_muted)
                .children(message.lines().map(|line| {
                    // An empty line still needs a line box, or the caret
                    // under the snippet climbs onto the text above it.
                    div().child(SharedString::from(if line.is_empty() {
                        " ".to_string()
                    } else {
                        line.to_string()
                    }))
                })),
        )
        .into_any_element()
}

/// Every embedded base in a built note, in document order, as the control
/// socket reports them.
///
/// An embed draws no chrome a state dump could otherwise name — it has no
/// path of its own in the reading view and no toolbar — so this walk is how
/// `![[x.base]]` is checked from outside the app without a screenshot.
pub fn describe_all(blocks: &[RBlock]) -> Vec<Json> {
    let mut out = Vec::new();
    walk(blocks, &mut out);
    out
}

fn walk(blocks: &[RBlock], out: &mut Vec<Json>) {
    for block in blocks {
        match block {
            RBlock::Paragraph(segments) => {
                for segment in segments {
                    if let Segment::Embed(embed) = segment {
                        if let EmbedKind::Base(base) = &embed.kind {
                            out.push(describe(base));
                        }
                        if let EmbedKind::Note { blocks, .. } = &embed.kind {
                            walk(blocks, out);
                        }
                    }
                }
            }
            RBlock::Quote(children) | RBlock::Callout { children, .. } => walk(children, out),
            RBlock::List { items, .. } => {
                for item in items {
                    walk(&item.children, out);
                }
            }
            RBlock::Footnotes(notes) => {
                for (_, body) in notes {
                    walk(body, out);
                }
            }
            _ => {}
        }
    }
}

/// One embedded base, as the socket reports it.
fn describe(base: &EmbeddedBase) -> Json {
    json!({
        "path": base.path,
        "view_name": base.view_name,
        "layout": base.layout.kind(),
        "error": base.error,
        "unknown_kind": base.table.as_ref().and_then(|t| t.unknown_kind.clone()),
        "columns": base
            .table
            .iter()
            .flat_map(|t| t.columns.iter())
            .map(|c| c.display.clone())
            .collect::<Vec<_>>(),
        "rows": base.table.as_ref().map_or(0, |t| t.rows.len()),
        "drawn": base
            .table
            .as_ref()
            .map_or(0, |t| t.rows.len().min(EMBED_ROWS)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use silica_bases::{Cell, ColumnSource, FileData};
    use std::sync::Arc;

    fn row(path: &str) -> Row {
        Row {
            file: Arc::new(FileData {
                path: path.to_string(),
                name: path.trim_end_matches(".md").to_string(),
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

    fn embedded(rows: Vec<Row>) -> EmbeddedBase {
        EmbeddedBase {
            path: "Main.base".to_string(),
            view_name: Some("View".to_string()),
            layout: Layout::default(),
            table: Some(Table {
                columns: vec![Column {
                    id: "file.name".to_string(),
                    display: "file name".to_string(),
                    source: ColumnSource::File("name".into()),
                }],
                rows,
                groups: Vec::new(),
                limited: false,
                unknown_kind: None,
            }),
            error: None,
        }
    }

    #[test]
    fn every_row_of_a_short_table_is_drawn_and_none_are_left_over() {
        let base = embedded((0..3).map(|i| row(&format!("N{i}.md"))).collect());
        let (drawn, more) = base.drawn_rows();
        assert_eq!(drawn.len(), 3);
        assert_eq!(more, 0);
    }

    #[test]
    fn a_long_table_stops_at_the_cap_and_counts_the_rest() {
        let base = embedded(
            (0..EMBED_ROWS + 7)
                .map(|i| row(&format!("N{i}.md")))
                .collect(),
        );
        let (drawn, more) = base.drawn_rows();
        assert_eq!(drawn.len(), EMBED_ROWS);
        assert_eq!(more, 7);
        assert_eq!(drawn[0].file.path, "N0.md");
    }

    #[test]
    fn a_view_with_no_rows_draws_no_rows_and_leaves_nothing_over() {
        let base = embedded(Vec::new());
        assert_eq!(base.drawn_rows(), (&[][..], 0));
    }

    #[test]
    fn a_base_that_does_not_parse_keeps_the_views_own_message() {
        // One engine: the embed resolves through `rows::resolve`, so a file
        // the view refuses is refused here with the same text.
        let resolved = rows::resolve(None, None, "views:\n  - type: table\n", 0);
        let base = EmbeddedBase {
            path: "Broken.base".to_string(),
            view_name: None,
            layout: Layout::default(),
            table: resolved.table,
            error: resolved.error,
        };
        assert!(base.error.is_some(), "a view with no name is refused");
        assert!(base.table.is_none(), "and nothing is drawn but the banner");
        assert_eq!(base.drawn_rows(), (&[][..], 0));
    }

    #[test]
    fn an_unknown_view_type_names_itself_and_has_no_rows() {
        let resolved = rows::resolve(None, None, "views:\n  - type: nonsense\n    name: V\n", 0);
        let base = EmbeddedBase {
            path: "Odd.base".to_string(),
            view_name: Some("V".to_string()),
            layout: Layout::default(),
            table: resolved.table,
            error: resolved.error,
        };
        assert!(base.error.is_none(), "the file itself parses");
        let table = base.table.as_ref().expect("and resolves to a table");
        assert_eq!(table.unknown_kind.as_deref(), Some("nonsense"));
        assert_eq!(base.drawn_rows(), (&[][..], 0));
    }

    #[test]
    fn a_cards_view_sizes_its_columns_at_the_table_defaults() {
        // A layout that is not a table carries no `columnSize`, and its rows
        // are drawn at the widths an unsized table column takes.
        let mut base = embedded(vec![row("N.md")]);
        base.layout = Layout::Cards(Default::default());
        let widths = column_widths(&base.table.as_ref().unwrap().columns, &base.table_layout());
        assert_eq!(widths.len(), 1);
        assert!(widths[0] >= MIN_COLUMN_WIDTH);
    }

    /// What a note with an embedded base costs on a vault the size
    /// `silica-bench` gates on, and that a second embed of the same base is
    /// free. Ignored, because writing 20k notes is seconds of IO that every
    /// other test in the crate would pay for:
    /// `cargo test -p silica --release --bin silica embed -- --ignored --nocapture`.
    #[test]
    #[ignore = "writes a 20k-note vault; run it by name"]
    fn a_note_embedding_a_base_over_twenty_thousand_notes_resolves_it_once() {
        use crate::reading::{build_with, BuildOptions};
        use crate::workspace::VaultState;
        use std::fs;

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join(".obsidian")).unwrap();
        for folder in 0..20 {
            fs::create_dir_all(root.join(format!("f{folder}"))).unwrap();
        }
        for note in 0..20_000u32 {
            fs::write(
                root.join(format!("f{}/Note {note}.md", note % 20)),
                format!("---\nstatus: todo\n---\n\nBody of {note}.\n"),
            )
            .unwrap();
        }
        fs::write(
            root.join("Main.base"),
            "views:\n  - type: table\n    name: View\n    order:\n      - file.name\n      - status\n",
        )
        .unwrap();
        fs::write(root.join("Note.md"), "![[Main.base]]\n\n![[Main.base]]\n").unwrap();
        let state = Arc::new(VaultState::load(&root.to_path_buf()).unwrap());

        let one = std::time::Instant::now();
        let base = resolve(&state, "Main.base").expect("the base resolves");
        let resolve_ms = one.elapsed().as_secs_f64() * 1000.;
        // The note holding the embed is a note of the vault too, so it is a
        // row of the base like any other.
        let rows = base.table.as_ref().unwrap().rows.len();
        assert_eq!(rows, 20_001);
        assert_eq!(base.drawn_rows().0.len(), EMBED_ROWS);
        assert_eq!(base.drawn_rows().1, rows - EMBED_ROWS);

        let both = std::time::Instant::now();
        let model = build_with(
            "Note.md",
            "![[Main.base]]\n\n![[Main.base]]\n",
            state.as_ref(),
            BuildOptions::default(),
        );
        let model_ms = both.elapsed().as_secs_f64() * 1000.;
        println!("resolve {resolve_ms:.0} ms, a note with two embeds {model_ms:.0} ms");
        assert_eq!(model.blocks.len(), 2);
        // Two embeds, one resolve: the model's own cache answers the second,
        // so a note full of the same base costs one view over the vault.
        assert!(
            model_ms < resolve_ms * 1.8,
            "two embeds cost {model_ms:.0} ms against {resolve_ms:.0} ms for one resolve"
        );
    }
}
