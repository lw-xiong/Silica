//! Editing a pipe table: the sixteen table commands, and the one form
//! Obsidian writes a table back in.
//!
//! The table under the cursor is found with the one Markdown parser rather
//! than by looking for pipes, so a `|` inside a code span splits a cell
//! here exactly where it splits one in the reading view. Every command
//! rewrites the whole table, which is what Obsidian does: see
//! [[Specification#Observed table editing behavior, Obsidian 1.14.4]].

use silica_markdown::{parse, Alignment, BlockKind};
use std::ops::Range;

/// The narrowest a column may be: `---` is what the delimiter needs.
const MIN_WIDTH: usize = 3;

/// One of Obsidian's table commands, by its command id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableCommand {
    RowBefore,
    RowAfter,
    RowUp,
    RowDown,
    RowCopy,
    RowDelete,
    ColBefore,
    ColAfter,
    ColLeft,
    ColRight,
    ColCopy,
    ColDelete,
    AlignLeft,
    AlignCenter,
    AlignRight,
}

impl TableCommand {
    /// The command with this Obsidian id, if it is a table command.
    pub fn from_id(id: &str) -> Option<TableCommand> {
        Some(match id {
            "editor:table-row-before" => TableCommand::RowBefore,
            "editor:table-row-after" => TableCommand::RowAfter,
            "editor:table-row-up" => TableCommand::RowUp,
            "editor:table-row-down" => TableCommand::RowDown,
            "editor:table-row-copy" => TableCommand::RowCopy,
            "editor:table-row-delete" => TableCommand::RowDelete,
            "editor:table-col-before" => TableCommand::ColBefore,
            "editor:table-col-after" => TableCommand::ColAfter,
            "editor:table-col-left" => TableCommand::ColLeft,
            "editor:table-col-right" => TableCommand::ColRight,
            "editor:table-col-copy" => TableCommand::ColCopy,
            "editor:table-col-delete" => TableCommand::ColDelete,
            "editor:table-col-align-left" => TableCommand::AlignLeft,
            "editor:table-col-align-center" => TableCommand::AlignCenter,
            "editor:table-col-align-right" => TableCommand::AlignRight,
            _ => return None,
        })
    }
}

/// A table as the commands work on it: the header is row 0, so moving the
/// first body row up swaps it with the header the way Obsidian's does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    pub alignments: Vec<Alignment>,
    /// The header first, then the body rows, every row the same width.
    pub rows: Vec<Vec<String>>,
}

/// The table the cursor is in, and where in it the cursor is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableAt {
    /// The table's bytes in the document, the trailing newline excluded.
    pub span: Range<usize>,
    pub table: Table,
    /// The bytes each cell stands for, the header row first. A row written
    /// with too few cells has too few spans: there is nowhere to type.
    pub cells: Vec<Vec<Range<usize>>>,
    /// The row the cursor is in, the header being 0.
    pub row: usize,
    pub col: usize,
}

/// What a command writes: the text replacing [`TableAt::span`], and where
/// in it the cursor lands — the start of the cell it was in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableEdit {
    pub text: String,
    pub cursor: usize,
}

/// Characters in a cell, which is what the padding counts.
fn width(cell: &str) -> usize {
    cell.chars().count()
}

impl Table {
    pub fn columns(&self) -> usize {
        self.alignments.len()
    }

    /// Each column as wide as its widest cell, and never under three.
    fn widths(&self) -> Vec<usize> {
        (0..self.columns())
            .map(|c| {
                self.rows
                    .iter()
                    .filter_map(|r| r.get(c))
                    .map(|cell| width(cell))
                    .chain(std::iter::once(MIN_WIDTH))
                    .max()
                    .unwrap_or(MIN_WIDTH)
            })
            .collect()
    }

    /// The whole table as Obsidian writes it back.
    pub fn render(&self) -> String {
        let widths = self.widths();
        let mut out = String::new();
        for (ix, row) in self.rows.iter().enumerate() {
            push_row(&mut out, row, &widths, &self.alignments);
            if ix == 0 {
                push_delimiter(&mut out, &widths, &self.alignments);
            }
        }
        // The last line carries no newline: the span it replaces had none.
        out.pop();
        out
    }

    /// Byte offset of column `col`'s text within rendered row `row`, for
    /// putting the cursor back in the cell it was in.
    fn cell_start(&self, row: usize, col: usize) -> usize {
        let mut at = 1;
        for (c, w) in self.widths().iter().enumerate().take(col) {
            let cell = self.rows[row].get(c).map(String::as_str).unwrap_or("");
            at += 1 + cell.len() + w.saturating_sub(width(cell)) + 2;
        }
        at + 1
    }

    /// Byte offset of the cell at `row`, `col` in the whole rendered table.
    fn cursor_at(&self, text: &str, row: usize, col: usize) -> usize {
        // Row 0 is the header, and the delimiter follows it.
        let line = if row == 0 { 0 } else { row + 1 };
        let start: usize = text.split_inclusive('\n').take(line).map(|l| l.len()).sum();
        start + self.cell_start(row, col)
    }

    fn empty_row(&self) -> Vec<String> {
        vec![String::new(); self.columns()]
    }
}

fn pad(cell: &str, to: usize, align: Alignment) -> String {
    let room = to.saturating_sub(width(cell));
    match align {
        Alignment::Right => format!("{}{cell}", " ".repeat(room)),
        // The odd space goes on the right.
        Alignment::Center => {
            let left = room / 2;
            format!("{}{cell}{}", " ".repeat(left), " ".repeat(room - left))
        }
        _ => format!("{cell}{}", " ".repeat(room)),
    }
}

fn push_row(out: &mut String, row: &[String], widths: &[usize], alignments: &[Alignment]) {
    out.push('|');
    for (c, w) in widths.iter().enumerate() {
        let cell = row.get(c).map(String::as_str).unwrap_or("");
        let align = alignments.get(c).copied().unwrap_or(Alignment::None);
        out.push(' ');
        out.push_str(&pad(cell, *w, align));
        out.push_str(" |");
    }
    out.push('\n');
}

fn push_delimiter(out: &mut String, widths: &[usize], alignments: &[Alignment]) {
    out.push('|');
    for (c, w) in widths.iter().enumerate() {
        let align = alignments.get(c).copied().unwrap_or(Alignment::None);
        out.push(' ');
        match align {
            Alignment::None => out.push_str(&"-".repeat(*w)),
            Alignment::Left => {
                out.push(':');
                out.push_str(&"-".repeat(w - 1));
            }
            Alignment::Right => {
                out.push_str(&"-".repeat(w - 1));
                out.push(':');
            }
            Alignment::Center => {
                out.push(':');
                out.push_str(&"-".repeat(w - 2));
                out.push(':');
            }
        }
        out.push_str(" |");
    }
    out.push('\n');
}

/// Every table in `text`, by the bytes it covers, from one parse.
pub fn tables(text: &str) -> Vec<Range<usize>> {
    parse(text)
        .blocks
        .iter()
        .filter(|b| matches!(b.kind, BlockKind::Table { .. }))
        .map(|b| b.span.start..b.span.end)
        .collect()
}

/// The table containing `offset`, parsed from `text`.
///
/// The cursor counts as inside the table anywhere in its lines, the
/// delimiter row included; a cursor on the delimiter is treated as being in
/// the header, which is the row its alignment belongs to.
pub fn table_at(text: &str, offset: usize) -> Option<TableAt> {
    let doc = parse(text);
    let block = doc.blocks.iter().find(|b| {
        matches!(b.kind, BlockKind::Table { .. }) && (b.span.start..=b.span.end).contains(&offset)
    })?;
    let BlockKind::Table {
        alignments,
        header,
        rows,
    } = &block.kind
    else {
        return None;
    };
    let span = block.span.start..block.span.end;
    let columns = alignments.len().max(header.len());
    let cell_text = |span: &silica_markdown::Span| text[span.start..span.end].to_string();
    let mut table = Table {
        alignments: alignments.clone(),
        rows: Vec::with_capacity(rows.len() + 1),
    };
    table.alignments.resize(columns, Alignment::None);
    let mut spans: Vec<Vec<Range<usize>>> = Vec::with_capacity(rows.len() + 1);
    let mut push = |cells: &[silica_markdown::TableCell<'_>], table: &mut Table| {
        let mut row: Vec<String> = cells.iter().map(|c| cell_text(&c.span)).collect();
        row.resize(columns, String::new());
        table.rows.push(row);
        spans.push(cells.iter().map(|c| c.span.start..c.span.end).collect());
    };
    push(header, &mut table);
    for row in rows {
        push(row, &mut table);
    }
    // Which cell the cursor is in: the row whose line holds it, and the
    // last cell that starts at or before it.
    let line_of = |at: usize| text[span.start..at.min(span.end)].matches('\n').count();
    let line = line_of(offset);
    // Line 1 is the delimiter; the header owns its alignment.
    let row = if line == 0 { 0 } else { line.saturating_sub(1) };
    let row = row.min(table.rows.len() - 1);
    let col = spans
        .get(row)
        .map(|cells| {
            cells
                .iter()
                .rposition(|s| s.start <= offset)
                .unwrap_or(0)
                .min(columns - 1)
        })
        .unwrap_or(0);
    Some(TableAt {
        span,
        table,
        cells: spans,
        row,
        col,
    })
}

/// The text `cmd` writes for the table at the cursor, or `None` when it
/// cannot apply — deleting the only column, or moving the first column
/// left.
pub fn apply(at: &TableAt, cmd: TableCommand) -> Option<TableEdit> {
    let mut t = at.table.clone();
    let (mut row, mut col) = (at.row, at.col);
    let rows = t.rows.len();
    let cols = t.columns();
    match cmd {
        TableCommand::RowBefore => {
            t.rows.insert(row, t.empty_row());
        }
        TableCommand::RowAfter => {
            t.rows.insert(row + 1, t.empty_row());
            row += 1;
        }
        TableCommand::RowUp => {
            if row == 0 {
                return None;
            }
            t.rows.swap(row, row - 1);
            row -= 1;
        }
        TableCommand::RowDown => {
            if row + 1 >= rows {
                return None;
            }
            t.rows.swap(row, row + 1);
            row += 1;
        }
        TableCommand::RowCopy => {
            let copy = t.rows[row].clone();
            t.rows.insert(row + 1, copy);
            row += 1;
        }
        TableCommand::RowDelete => {
            // The header is a row like any other: deleting it promotes the
            // one below, and the last row standing cannot go.
            if rows <= 1 {
                return None;
            }
            t.rows.remove(row);
            row = row.min(t.rows.len() - 1);
        }
        TableCommand::ColBefore => {
            insert_column(&mut t, col, Alignment::None, None);
        }
        TableCommand::ColAfter => {
            insert_column(&mut t, col + 1, Alignment::None, None);
            col += 1;
        }
        TableCommand::ColLeft => {
            if col == 0 {
                return None;
            }
            swap_columns(&mut t, col, col - 1);
            col -= 1;
        }
        TableCommand::ColRight => {
            if col + 1 >= cols {
                return None;
            }
            swap_columns(&mut t, col, col + 1);
            col += 1;
        }
        TableCommand::ColCopy => {
            let cells: Vec<String> = t.rows.iter().map(|r| r[col].clone()).collect();
            let align = t.alignments[col];
            insert_column(&mut t, col + 1, align, Some(cells));
            col += 1;
        }
        TableCommand::ColDelete => {
            if cols <= 1 {
                return None;
            }
            for r in &mut t.rows {
                r.remove(col);
            }
            t.alignments.remove(col);
            col = col.min(t.columns() - 1);
        }
        TableCommand::AlignLeft => t.alignments[col] = Alignment::Left,
        TableCommand::AlignCenter => t.alignments[col] = Alignment::Center,
        TableCommand::AlignRight => t.alignments[col] = Alignment::Right,
    }
    let text = t.render();
    let cursor = t.cursor_at(&text, row, col.min(t.columns() - 1));
    Some(TableEdit { text, cursor })
}

fn insert_column(t: &mut Table, at: usize, align: Alignment, cells: Option<Vec<String>>) {
    for (ix, r) in t.rows.iter_mut().enumerate() {
        let cell = cells
            .as_ref()
            .and_then(|c| c.get(ix).cloned())
            .unwrap_or_default();
        r.insert(at.min(r.len()), cell);
    }
    t.alignments.insert(at.min(t.alignments.len()), align);
}

fn swap_columns(t: &mut Table, a: usize, b: usize) {
    for r in &mut t.rows {
        r.swap(a, b);
    }
    t.alignments.swap(a, b);
}

/// What text typed into a cell is written as.
///
/// A bare `|` would split the cell in two and a newline would end the row,
/// so both are kept inside the cell they were typed in. Editing a cell
/// re-pads nothing: Obsidian only re-pads when a table command runs, so the
/// rest of the table keeps its bytes.
pub fn cell_text(typed: &str) -> String {
    let mut out = String::with_capacity(typed.len());
    let mut chars = typed.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\n' | '\r' => out.push(' '),
            // One already escaped stays as it is.
            '\\' if chars.peek() == Some(&'|') => {
                out.push('\\');
                out.push(chars.next().expect("peeked"));
            }
            '|' => out.push_str("\\|"),
            _ => out.push(c),
        }
    }
    out
}

/// The table `editor:insert-table` writes: a blank line, then two by two
/// empty cells, with the cursor in the first header cell.
pub fn insert_table() -> TableEdit {
    let table = Table {
        alignments: vec![Alignment::None; 2],
        rows: vec![vec![String::new(); 2]; 2],
    };
    let text = format!("\n{}\n", table.render());
    // The cursor goes in the first header cell, just past its pipe.
    TableEdit {
        text,
        cursor: "\n|".len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs `cmd` with the cursor at the first byte of `cell` in `src`.
    fn run(src: &str, cursor: &str, cmd: TableCommand) -> String {
        let offset = src.find(cursor).expect("the cursor text is in the table");
        let at = table_at(src, offset).expect("the cursor is in a table");
        let edit = apply(&at, cmd).expect("the command applies");
        let mut out = src.to_string();
        out.replace_range(at.span.clone(), &edit.text);
        out
    }

    const PADDED: &str = "| A | B |\n| --- | --- |\n| 1 | 2 |\n| 3 | 4 |\n";

    #[test]
    fn any_command_rewrites_the_whole_table() {
        // Obsidian pads every cell, however the table was written.
        assert_eq!(
            run("|A|B|\n|-|-|\n|1|2|\n|3|4|\n", "1", TableCommand::RowAfter),
            "| A   | B   |\n| --- | --- |\n| 1   | 2   |\n|     |     |\n| 3   | 4   |\n"
        );
        // Outer pipes are added to a table written without them.
        assert_eq!(
            run("A | B\n--- | ---\n1 | 2\n", "1 |", TableCommand::RowAfter),
            "| A   | B   |\n| --- | --- |\n| 1   | 2   |\n|     |     |\n"
        );
    }

    #[test]
    fn rows_are_added_moved_copied_and_deleted() {
        assert_eq!(
            run(PADDED, "1", TableCommand::RowBefore),
            "| A   | B   |\n| --- | --- |\n|     |     |\n| 1   | 2   |\n| 3   | 4   |\n"
        );
        assert_eq!(
            run(PADDED, "1", TableCommand::RowAfter),
            "| A   | B   |\n| --- | --- |\n| 1   | 2   |\n|     |     |\n| 3   | 4   |\n"
        );
        // The first body row swaps with the header; the delimiter stays put.
        assert_eq!(
            run(PADDED, "1", TableCommand::RowUp),
            "| 1   | 2   |\n| --- | --- |\n| A   | B   |\n| 3   | 4   |\n"
        );
        assert_eq!(
            run(PADDED, "1", TableCommand::RowDown),
            "| A   | B   |\n| --- | --- |\n| 3   | 4   |\n| 1   | 2   |\n"
        );
        assert_eq!(
            run(PADDED, "1", TableCommand::RowCopy),
            "| A   | B   |\n| --- | --- |\n| 1   | 2   |\n| 1   | 2   |\n| 3   | 4   |\n"
        );
        assert_eq!(
            run(PADDED, "1", TableCommand::RowDelete),
            "| A   | B   |\n| --- | --- |\n| 3   | 4   |\n"
        );
        // Deleting the header promotes the row below it.
        assert_eq!(
            run(PADDED, "A", TableCommand::RowDelete),
            "| 1   | 2   |\n| --- | --- |\n| 3   | 4   |\n"
        );
    }

    #[test]
    fn columns_are_added_moved_copied_and_deleted() {
        assert_eq!(
            run(PADDED, "A", TableCommand::ColBefore),
            "|     | A   | B   |\n| --- | --- | --- |\n|     | 1   | 2   |\n|     | 3   | 4   |\n"
        );
        assert_eq!(
            run(PADDED, "A", TableCommand::ColAfter),
            "| A   |     | B   |\n| --- | --- | --- |\n| 1   |     | 2   |\n| 3   |     | 4   |\n"
        );
        assert_eq!(
            run(PADDED, "A", TableCommand::ColRight),
            "| B   | A   |\n| --- | --- |\n| 2   | 1   |\n| 4   | 3   |\n"
        );
        assert_eq!(
            run(PADDED, "B", TableCommand::ColLeft),
            "| B   | A   |\n| --- | --- |\n| 2   | 1   |\n| 4   | 3   |\n"
        );
        assert_eq!(
            run(PADDED, "A", TableCommand::ColCopy),
            "| A   | A   | B   |\n| --- | --- | --- |\n| 1   | 1   | 2   |\n| 3   | 3   | 4   |\n"
        );
        assert_eq!(
            run(PADDED, "A", TableCommand::ColDelete),
            "| B   |\n| --- |\n| 2   |\n| 4   |\n"
        );
    }

    #[test]
    fn alignment_sets_the_delimiter_and_repads_the_column() {
        assert_eq!(
            run(PADDED, "B", TableCommand::AlignLeft),
            "| A   | B   |\n| --- | :-- |\n| 1   | 2   |\n| 3   | 4   |\n"
        );
        assert_eq!(
            run(PADDED, "B", TableCommand::AlignCenter),
            "| A   |  B  |\n| --- | :-: |\n| 1   |  2  |\n| 3   |  4  |\n"
        );
        assert_eq!(
            run(PADDED, "B", TableCommand::AlignRight),
            "| A   |   B |\n| --- | --: |\n| 1   |   2 |\n| 3   |   4 |\n"
        );
        // Wider columns grow the delimiter and keep the odd space right.
        let ragged = "| Long header | B |\n| --- | ------ |\n| x | yyyyyy |\n| z | w |\n";
        assert_eq!(
            run(ragged, "yyyyyy", TableCommand::AlignCenter),
            "| Long header |   B    |\n| ----------- | :----: |\n| x           | yyyyyy |\n| z           |   w    |\n"
        );
        assert_eq!(
            run(ragged, "yyyyyy", TableCommand::AlignRight),
            "| Long header |      B |\n| ----------- | -----: |\n| x           | yyyyyy |\n| z           |      w |\n"
        );
    }

    #[test]
    fn existing_alignments_survive_an_edit() {
        let aligned = "| A | B | C |\n| :-- | :-: | --: |\n| 1 | 2 | 3 |\n";
        assert_eq!(
            run(aligned, "A", TableCommand::RowAfter),
            "| A   |  B  |   C |\n| :-- | :-: | --: |\n|     |     |     |\n| 1   |  2  |   3 |\n"
        );
        // Alignment travels with the column it belongs to.
        assert_eq!(
            run(aligned, "A", TableCommand::ColRight),
            "|  B  | A   |   C |\n| :-: | :-- | --: |\n|  2  | 1   |   3 |\n"
        );
        assert_eq!(
            run(aligned, "A", TableCommand::ColCopy),
            "| A   | A   |  B  |   C |\n| :-- | :-- | :-: | --: |\n| 1   | 1   |  2  |   3 |\n"
        );
        assert_eq!(
            run(aligned, "A", TableCommand::ColDelete),
            "|  B  |   C |\n| :-: | --: |\n|  2  |   3 |\n"
        );
    }

    #[test]
    fn short_rows_are_filled_out_and_escapes_are_kept() {
        assert_eq!(
            run(
                "| A | B | C |\n| --- | --- | --- |\n| 1 |\n",
                "1",
                TableCommand::RowAfter
            ),
            "| A   | B   | C   |\n| --- | --- | --- |\n| 1   |     |     |\n|     |     |     |\n"
        );
        // `\|` keeps the cell whole, and the escape is written back.
        let escaped = "| a | b |\n| --- | --- |\n| x \\| y | [[Alpha]] |\n";
        assert_eq!(
            run(escaped, "Alpha", TableCommand::RowAfter),
            "| a      | b         |\n| ------ | --------- |\n| x \\| y | [[Alpha]] |\n|        |           |\n"
        );
    }

    #[test]
    fn a_header_the_parser_rejects_is_not_a_table() {
        // A `|` inside a code span splits a cell, so this header has three
        // cells against the delimiter's two and GFM makes it a paragraph.
        // Obsidian's table editor is looser and would rewrite it into a
        // three-column table; Silica acts on what the parser calls a table,
        // so the command does not apply rather than turning prose into one.
        let rich = "| `a|b` | **bold** |\n| --- | --- |\n| x | y |\n";
        assert!(table_at(rich, rich.find("bold").unwrap()).is_none());
    }

    #[test]
    fn a_command_that_cannot_apply_writes_nothing() {
        let at = table_at(PADDED, PADDED.find('A').unwrap()).unwrap();
        // Nothing is above the header, nothing is left of the first column.
        assert_eq!(apply(&at, TableCommand::RowUp), None);
        assert_eq!(apply(&at, TableCommand::ColLeft), None);
        let one = "| A |\n| --- |\n| 1 |\n";
        let at = table_at(one, one.find('1').unwrap()).unwrap();
        assert_eq!(apply(&at, TableCommand::ColDelete), None);
    }

    #[test]
    fn the_cursor_is_found_in_the_right_cell() {
        let at = table_at(PADDED, PADDED.find('4').unwrap()).unwrap();
        assert_eq!((at.row, at.col), (2, 1));
        // The delimiter belongs to the header, whose alignment it carries.
        let at = table_at(PADDED, PADDED.find("---").unwrap()).unwrap();
        assert_eq!(at.row, 0);
        // Text outside any table is not in one.
        let src = "para\n\n| A |\n| --- |\n| 1 |\n\nafter\n";
        assert!(table_at(src, 0).is_none());
        assert!(table_at(src, src.find("after").unwrap()).is_none());
        assert!(table_at(src, src.find('1').unwrap()).is_some());
    }

    #[test]
    fn typed_cell_text_keeps_the_row_whole() {
        assert_eq!(cell_text("plain"), "plain");
        // A bare pipe would split the cell; an escaped one is already safe.
        assert_eq!(cell_text("x|y"), "x\\|y");
        assert_eq!(cell_text("x\\|y"), "x\\|y");
        assert_eq!(cell_text("a|b|c"), "a\\|b\\|c");
        // A newline would end the row.
        assert_eq!(cell_text("two\nlines"), "two lines");
        assert_eq!(cell_text("`a|b`"), "`a\\|b`");
    }

    #[test]
    fn insert_writes_an_empty_two_by_two_table() {
        let edit = insert_table();
        assert_eq!(edit.text, "\n|     |     |\n| --- | --- |\n|     |     |\n");
        assert_eq!(&edit.text[edit.cursor..edit.cursor + 5], "     ");
    }

    #[test]
    fn the_cursor_lands_in_the_cell_it_was_in() {
        // The cell the cursor was in, in the rewritten table.
        let at = table_at(PADDED, PADDED.find('4').unwrap()).unwrap();
        let edit = apply(&at, TableCommand::RowCopy).unwrap();
        assert_eq!(&edit.text[edit.cursor..edit.cursor + 1], "4");
        let at = table_at(PADDED, PADDED.find('A').unwrap()).unwrap();
        let edit = apply(&at, TableCommand::ColAfter).unwrap();
        // The new column is empty, and the cursor sits in it.
        assert_eq!(&edit.text[edit.cursor..edit.cursor + 3], "   ");
        // A multibyte cell still lands on a character boundary.
        let wide = "| é | b |\n| --- | --- |\n| ü | x |\n";
        let at = table_at(wide, wide.find('x').unwrap()).unwrap();
        let edit = apply(&at, TableCommand::RowAfter).unwrap();
        assert!(edit.text.is_char_boundary(edit.cursor));
    }
}
