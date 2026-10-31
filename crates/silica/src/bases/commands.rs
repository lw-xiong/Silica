//! The `bases:*` commands, as far as they are the view's own work.
//!
//! Observed (`tools/obsidian-probe/results/2026-10-25-bases.json`; the table
//! is in
//! [[Specification#Observed Bases write behavior, Obsidian 1.14.5 (installer build 1.13.7)]]):
//!
//! | Command | Palette name | Effect |
//! | --- | --- | --- |
//! | `bases:new-file` | Bases: Create new base | a **zero-byte** `Untitled.base`, opened |
//! | `bases:add-view` | Bases: Add view | appends a view named `View 2` and opens its Configure view page |
//! | `bases:insert` | Bases: Insert new base | another zero-byte base, and `![[Untitled 1.base]]` on a line of its own at the cursor |
//! | `bases:copy-table` | Bases: Copy table to clipboard | the visible columns as TSV, and a "Copied to your clipboard" notice |
//! | `bases:change-view` | Bases: Switch view... | not probed |
//! | `bases:add-item` | Bases: Add item | not probed |
//!
//! `bases:new-file` is the workspace's own (it creates a file and opens it);
//! the rest need the showing view, which is what lives here. Each of these is
//! half a command: the view answers what only it knows — the next view's
//! name, the TSV of the showing table, where a new item would go — and the
//! workspace does the file, the clipboard and the Notice.
//!
//! A command that needs a base does **nothing at all** when none is showing:
//! the file that failed to parse has no base to append a view to, and
//! Obsidian's own check callbacks keep such a command off the palette rather
//! than letting it run and fail.

use crate::bases::cells::table_tsv;
use crate::bases::view::BasesView;
use gpui::Context;
use silica_formats::base::{Base, View, DEFAULT_VIEW_NAME, DEFAULT_VIEW_TYPE};

/// Where `bases:add-item` would put the note it creates, which is the base's
/// own `newItemFolder` and `newItemTemplate` — two keys the published schema
/// never mentions and both of which round-tripped from the probe's fixture.
///
/// Both are optional, and an absent folder is the workspace's to fall back
/// on: the view knows nothing about the vault's default location for a new
/// note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewItem {
    pub folder: Option<String>,
    pub template: Option<String>,
}

impl BasesView {
    /// `bases:add-view`: appends a view to the base and shows it.
    ///
    /// Observed: the second view of a base is named `View 2`, and Obsidian
    /// opens its Configure view page — which is the toolbar's views panel,
    /// so the command shows the new view and then opens that page on it.
    ///
    /// Answers the new view's index, or `None` when there is no base to
    /// append to: a file that did not parse is never written, so there is
    /// nothing a view could be added to either.
    pub fn add_view(&mut self, cx: &mut Context<Self>) -> Option<usize> {
        self.base()?;
        self.edit_base(append_view, cx);
        let ix = self.base()?.views.len().checked_sub(1)?;
        self.show_view(ix, cx);
        self.configure_view(ix, cx);
        Some(ix)
    }

    /// The names of the views `bases:change-view` chooses between, in file
    /// order.
    ///
    /// A base whose file names no views has one default view all the same —
    /// the parsed query of a zero-byte file is one `table` named `View` — so
    /// that is what this answers for it. A file that did not parse has no
    /// views at all.
    pub fn view_names(&self) -> Vec<String> {
        let Some(base) = self.base() else {
            return Vec::new();
        };
        if base.views.is_empty() {
            return vec![DEFAULT_VIEW_NAME.to_string()];
        }
        base.views.iter().map(|v| v.name.clone()).collect()
    }

    /// `bases:change-view`: shows the view the chooser picked.
    ///
    /// **Unprobed**: the command was never driven, so neither the chooser nor
    /// what it looks like is observed; this is [`BasesView::show_view`], the
    /// same switch the views panel makes.
    pub fn switch_view(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.show_view(ix, cx);
    }

    /// `bases:copy-table`: the showing view's visible columns as TSV, or
    /// `None` when no base is showing.
    ///
    /// Observed: one header row of **display names**, tab separators, no
    /// trailing newline (`status\tPrice per unit\ntodo\t0.02\ntodo\t0.06`),
    /// and the rows of a grouped view in group order with no group headings
    /// — which is [`Table::rows`] as it stands. The results panel's own
    /// *Copy to clipboard* row produces the same bytes, so it shares this.
    ///
    /// Putting it on the clipboard and announcing it is the workspace's: the
    /// notice depends on where the keyboard is, which the view cannot see.
    pub fn table_tsv(&self) -> Option<String> {
        self.table().map(table_tsv)
    }

    /// `bases:add-item`: the base's own `newItemFolder` and
    /// `newItemTemplate`, which is what a new item would be created from.
    ///
    /// **Unprobed**: the command was never driven. These two keys are what
    /// the file carries for it, so they are the reading; creating the note is
    /// the workspace's half, and nothing here writes the `.base`.
    pub fn new_item(&self) -> Option<NewItem> {
        let base = self.base()?;
        Some(NewItem {
            folder: base.new_item_folder.clone(),
            template: base.new_item_template.clone(),
        })
    }
}

/// What `bases:add-view` does to the base: appends a [`DEFAULT_VIEW_TYPE`]
/// view under [`next_view_name`].
///
/// A base whose file names no views is given its default view first, the way
/// [`BasesView::edit_view`] materializes one, because that view exists in
/// Obsidian's parsed query already and the write that follows has to carry
/// it: appending to nothing would otherwise write a base whose only view is
/// `View 2`.
pub(super) fn append_view(base: &mut Base) {
    if base.views.is_empty() {
        base.views.push(View {
            kind: DEFAULT_VIEW_TYPE.to_string(),
            name: DEFAULT_VIEW_NAME.to_string(),
            ..View::default()
        });
    }
    base.views.push(View {
        kind: DEFAULT_VIEW_TYPE.to_string(),
        name: next_view_name(&base.views),
        ..View::default()
    });
}

/// The name the appended view takes: `View 2` for the second view of a base,
/// which is observed, and `View 3`, `View 4`, ... after it, which is the
/// reading of that one datum — the number is the view's own position.
///
/// A name already taken is stepped past rather than repeated, since `name` is
/// the only thing the views panel has to tell two views apart.
/// **Unprobed**: whether Obsidian dedupes at all, and whether it compares
/// names case-insensitively; this compares them byte for byte.
pub(super) fn next_view_name(views: &[View]) -> String {
    let mut n = views.len() + 1;
    loop {
        let name = format!("{DEFAULT_VIEW_NAME} {n}");
        if !views.iter().any(|v| v.name == name) {
            return name;
        }
        n += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use silica_bases::{Cell, CellKind, Column, ColumnSource, FileData, Group, Row, Table, Value};
    use std::sync::Arc;

    fn column(id: &str, display: &str) -> Column {
        Column {
            id: id.to_string(),
            display: display.to_string(),
            source: ColumnSource::Property(id.to_string()),
        }
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
            file: Arc::new(FileData {
                path: path.to_string(),
                name: path.to_string(),
                basename: path.trim_end_matches(".md").to_string(),
                ..FileData::default()
            }),
            cells: cells.iter().map(|c| cell(c)).collect(),
            image: None,
        }
    }

    /// The bytes the probe read off the clipboard, for a two-column table of
    /// two rows.
    #[test]
    fn the_tsv_is_the_display_names_and_a_line_a_row_with_no_trailing_newline() {
        let rows = vec![
            row("a.md", &["todo", "0.02"]),
            row("b.md", &["todo", "0.06"]),
        ];
        let table = Table {
            columns: vec![column("status", "status"), column("ppu", "Price per unit")],
            rows: rows.clone(),
            groups: vec![Group { key: None, rows }],
            limited: false,
            unknown_kind: None,
        };
        assert_eq!(
            table_tsv(&table),
            "status\tPrice per unit\ntodo\t0.02\ntodo\t0.06"
        );
    }

    /// A view with no rows is still a header row, and still no newline after
    /// it.
    #[test]
    fn a_table_with_no_rows_is_its_header_alone() {
        let table = Table {
            columns: vec![column("status", "status")],
            rows: Vec::new(),
            groups: Vec::new(),
            limited: false,
            unknown_kind: None,
        };
        assert_eq!(table_tsv(&table), "status");
    }

    #[test]
    fn add_view_appends_a_view_named_view_2_and_writes_obsidians_bytes() {
        let mut base = Base::from_yaml("views:\n  - type: table\n    name: View\n").unwrap();
        append_view(&mut base);
        assert_eq!(base.views.len(), 2);
        assert_eq!(base.views[1].name, "View 2");
        assert_eq!(base.views[1].kind, "table");
        assert_eq!(
            base.to_yaml(),
            "views:\n  - type: table\n    name: View\n  - type: table\n    name: View 2\n"
        );
    }

    /// The default view of a zero-byte base is in Obsidian's parsed query but
    /// not in the file, so appending to one has to write both views.
    #[test]
    fn add_view_on_an_empty_base_writes_the_default_view_too() {
        let mut base = Base::from_yaml("").unwrap();
        append_view(&mut base);
        assert_eq!(
            base.to_yaml(),
            "views:\n  - type: table\n    name: View\n  - type: table\n    name: View 2\n"
        );
    }

    #[test]
    fn the_next_free_name_steps_past_an_existing_view_2() {
        let base =
            Base::from_yaml("views:\n  - type: table\n    name: View 2\n").expect("it parses");
        assert_eq!(next_view_name(&base.views), "View 3");
        let base = Base::from_yaml(
            "views:\n  - type: table\n    name: View\n  - type: table\n    name: View 3\n",
        )
        .expect("it parses");
        assert_eq!(next_view_name(&base.views), "View 4");
    }
}
