//! Smart lists and quotes for the editor: what Enter continues, what Tab
//! and Shift+Tab indent, and how ordered items renumber. Pure functions
//! over one line's text so they are testable without a window.
//!
//! A line is read as `indent` `quotes` `list_indent` `marker` `task` `rest`,
//! for example `  > > - [x] done`: two spaces of outer indent, two quote
//! markers, no list indent, a `-` bullet, a checked task and `done`.

use std::ops::Range;

/// The prefix of one line that Enter would continue.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Prefix {
    /// Whitespace before the first quote marker (or list marker).
    pub indent: Range<usize>,
    /// `> ` markers, verbatim, possibly nested; empty when the line is not quoted.
    pub quotes: Range<usize>,
    /// Whitespace between the quotes and the list marker.
    pub list_indent: Range<usize>,
    /// The list marker without its trailing space; `None` on a plain line.
    pub marker: Option<Marker>,
    /// `[ ]` or `[x]` after the marker, with its trailing space.
    pub task: Option<Range<usize>>,
    /// Where the item's own text starts.
    pub content: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Marker {
    Bullet {
        range: Range<usize>,
        ch: char,
    },
    Ordered {
        range: Range<usize>,
        number: u64,
        delimiter: char,
    },
}

impl Marker {
    fn range(&self) -> Range<usize> {
        match self {
            Marker::Bullet { range, .. } | Marker::Ordered { range, .. } => range.clone(),
        }
    }
}

/// Parses the continuable prefix of `line` (one line, no newline).
pub fn parse_prefix(line: &str) -> Prefix {
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
        i += 1;
    }
    let indent = 0..i;
    let quotes_start = i;
    loop {
        if i < b.len() && b[i] == b'>' {
            i += 1;
            if i < b.len() && b[i] == b' ' {
                i += 1;
            }
            // Up to three spaces may precede a nested marker.
            let mut j = i;
            let mut spaces = 0;
            while j < b.len() && b[j] == b' ' && spaces < 3 {
                j += 1;
                spaces += 1;
            }
            if j < b.len() && b[j] == b'>' {
                i = j;
                continue;
            }
        }
        break;
    }
    let quotes = quotes_start..i;
    let mut indent = indent;
    let mut list_start = i;
    if quotes.is_empty() {
        // No quote: the leading whitespace is the list's own nesting.
        list_start = 0;
        i = indent.end;
        indent = 0..0;
    }
    while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
        i += 1;
    }
    let list_indent = list_start..i;
    let mut prefix = Prefix {
        indent,
        quotes,
        list_indent: list_indent.clone(),
        marker: None,
        task: None,
        content: i,
    };
    // A marker needs a space (or the end of the line) after it.
    let marker = if i < b.len() && matches!(b[i], b'-' | b'*' | b'+') {
        if i + 1 == b.len() || b[i + 1] == b' ' {
            Some(Marker::Bullet {
                range: i..i + 1,
                ch: b[i] as char,
            })
        } else {
            None
        }
    } else {
        let mut j = i;
        while j < b.len() && b[j].is_ascii_digit() && j - i < 9 {
            j += 1;
        }
        if j > i
            && j < b.len()
            && matches!(b[j], b'.' | b')')
            && (j + 1 == b.len() || b[j + 1] == b' ')
        {
            Some(Marker::Ordered {
                range: i..j + 1,
                number: line[i..j].parse().unwrap_or(0),
                delimiter: b[j] as char,
            })
        } else {
            None
        }
    };
    let Some(marker) = marker else {
        // No list: the whitespace belongs to the line's indentation.
        if prefix.quotes.is_empty() {
            prefix.indent = 0..list_indent.end;
        }
        prefix.list_indent = list_start..list_start;
        prefix.content = if prefix.quotes.is_empty() {
            prefix.indent.end
        } else {
            list_start
        };
        return prefix;
    };
    i = marker.range().end;
    if i < b.len() && b[i] == b' ' {
        i += 1;
    }
    // `[ ] ` or `[x] ` with any single character inside.
    if i + 3 <= b.len()
        && b[i] == b'['
        && b[i + 2] == b']'
        && (i + 3 == b.len() || b[i + 3] == b' ')
    {
        let mut end = i + 3;
        if end < b.len() && b[end] == b' ' {
            end += 1;
        }
        prefix.task = Some(i..end);
        i = end;
    }
    prefix.marker = Some(marker);
    prefix.content = i;
    prefix
}

/// What Enter does at byte `col` of `line`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Continuation {
    /// Insert a newline followed by this prefix (the indentation alone on a
    /// plain line, the quote and list markers on a continued item).
    Insert(String),
    /// The item is empty: replace `range` of the line with `with` (clears
    /// the marker, or outdents a nested item) instead of adding a line.
    Clear { range: Range<usize>, with: String },
}

/// Smart-list behaviour for Enter at `col`: continues the marker, numbering
/// and checkbox, and clears or outdents an empty item by one `tab_size` step.
pub fn continuation(line: &str, col: usize, tab_size: u8) -> Continuation {
    let p = parse_prefix(line);
    if col < p.content {
        // Before the marker: only the leading whitespace carries over.
        let mut ws = line[p.indent.clone()].to_string();
        if p.quotes.is_empty() {
            ws.push_str(&line[p.list_indent.clone()]);
        }
        return Continuation::Insert(ws);
    }
    let mut prefix = String::new();
    prefix.push_str(&line[p.indent.clone()]);
    prefix.push_str(&line[p.quotes.clone()]);
    let Some(marker) = &p.marker else {
        // A bare quote line continues the quote; an empty one ends it.
        if !p.quotes.is_empty() && line[p.content..].trim().is_empty() {
            return Continuation::Clear {
                range: p.indent.start..line.len(),
                with: String::new(),
            };
        }
        return Continuation::Insert(prefix);
    };
    if line[p.content..].trim().is_empty() {
        // Empty item: a nested one outdents, a top-level one loses its marker.
        if !p.list_indent.is_empty() {
            let unit = outdent_width(&line[p.list_indent.clone()], tab_size);
            let cut = p.list_indent.start..p.list_indent.start + unit;
            return Continuation::Clear {
                range: cut,
                with: String::new(),
            };
        }
        return Continuation::Clear {
            range: p.list_indent.start..line.len(),
            with: String::new(),
        };
    }
    prefix.push_str(&line[p.list_indent.clone()]);
    match marker {
        Marker::Bullet { ch, .. } => {
            prefix.push(*ch);
            prefix.push(' ');
        }
        Marker::Ordered {
            number, delimiter, ..
        } => {
            prefix.push_str(&(number + 1).to_string());
            prefix.push(*delimiter);
            prefix.push(' ');
        }
    }
    if p.task.is_some() {
        prefix.push_str("[ ] ");
    }
    Continuation::Insert(prefix)
}

/// Bytes of one indentation step to remove from the start of `ws`: a tab,
/// or up to `tab_size` spaces, so that Shift+Tab undoes exactly what Tab
/// did. Ragged indentation gives up only the spaces it has rather than
/// snapping back to the previous multiple of the step.
pub fn outdent_width(ws: &str, tab_size: u8) -> usize {
    let b = ws.as_bytes();
    if b.first() == Some(&b'\t') {
        return 1;
    }
    b.iter()
        .take(tab_size.max(1) as usize)
        .take_while(|&&c| c == b' ')
        .count()
}

/// The indentation unit for Tab (`useTab`, `tabSize`): a tab, or four
/// spaces whatever `tabSize` says — with `useTab: false` Obsidian was
/// observed inserting four spaces under `tabSize: 2` (Specification.md,
/// "Observed editing command behavior").
pub fn indent_unit(use_tab: bool, tab_size: u8) -> String {
    let _ = tab_size;
    if use_tab {
        "\t".to_string()
    } else {
        " ".repeat(4)
    }
}

/// The display columns of an indentation string: a tab counts `tab_size`
/// columns, a space one.
fn indent_columns(ws: &str, tab_size: u8) -> usize {
    ws.bytes()
        .map(|b| {
            if b == b'\t' {
                tab_size.max(1) as usize
            } else {
                1
            }
        })
        .sum()
}

/// `columns` of indentation written back: tabs then spaces with
/// `use_tab`, spaces alone otherwise.
fn indent_string(columns: usize, use_tab: bool, tab_size: u8) -> String {
    if use_tab {
        let t = tab_size.max(1) as usize;
        format!("{}{}", "\t".repeat(columns / t), " ".repeat(columns % t))
    } else {
        " ".repeat(columns)
    }
}

/// What Shift+Tab does to this line's indentation: one unit of display
/// columns is taken off, clamped at zero, and the rest is written back as
/// tabs then spaces (`use_tab`) or spaces. The unit is `tab_size` columns
/// with `use_tab`, four without, as [`indent_unit`]. `None` when the line
/// has no indentation to give up.
pub fn outdent_edit(line: &str, use_tab: bool, tab_size: u8) -> Option<(Range<usize>, String)> {
    let p = parse_prefix(line);
    let ws = if p.marker.is_some() {
        p.list_indent.clone()
    } else {
        p.indent.clone()
    };
    let old = &line[ws.clone()];
    let unit = if use_tab { tab_size.max(1) as usize } else { 4 };
    let columns = indent_columns(old, tab_size).saturating_sub(unit);
    let new = indent_string(columns, use_tab, tab_size);
    (new != old).then_some((ws, new))
}

/// Where an indentation step goes on this line: after the quote markers
/// for a list item, at the line start otherwise.
pub fn indent_point(line: &str) -> usize {
    let p = parse_prefix(line);
    if p.marker.is_some() {
        p.list_indent.start
    } else {
        0
    }
}

/// The range an outdent removes from this line, if there is anything to remove.
pub fn outdent_range(line: &str, tab_size: u8) -> Option<Range<usize>> {
    let p = parse_prefix(line);
    let ws = if p.marker.is_some() {
        p.list_indent.clone()
    } else {
        p.indent.clone()
    };
    let n = outdent_width(&line[ws.clone()], tab_size);
    (n > 0).then(|| ws.start..ws.start + n)
}

/// Ordered items that follow an item on `lines[from]` at the same depth,
/// each with the number it should carry, counting from `next`. Deeper
/// items are skipped; a blank line, a bullet or a shallower line ends the
/// list. Returns `(line index, marker range, new number)` for items whose
/// number is wrong.
pub fn renumber_following<'a>(
    lines: impl Iterator<Item = &'a str>,
    reference: &str,
    mut next: u64,
) -> Vec<(usize, Range<usize>, u64)> {
    let r = parse_prefix(reference);
    let depth = (
        &reference[r.indent.clone()],
        &reference[r.quotes.clone()],
        &reference[r.list_indent.clone()],
    );
    let mut out = Vec::new();
    for (ix, line) in lines.enumerate() {
        if line.trim().is_empty() {
            break;
        }
        let p = parse_prefix(line);
        let here = (
            &line[p.indent.clone()],
            &line[p.quotes.clone()],
            &line[p.list_indent.clone()],
        );
        if here.0 != depth.0 || here.1 != depth.1 {
            break;
        }
        if here.2.len() > depth.2.len() {
            continue;
        }
        if here.2 != depth.2 {
            break;
        }
        match p.marker {
            Some(Marker::Ordered { range, number, .. }) => {
                if number != next {
                    out.push((ix, range.start..range.end - 1, next));
                }
                next += 1;
            }
            _ => break,
        }
    }
    out
}

/// After the lines in `changed` were indented or outdented, the numbers
/// the ordered items from the first changed line to the end of the block
/// (a blank line) should carry: each follows its previous sibling at the
/// same depth, and one with no previous sibling (the first of a nested
/// list, or the first left behind) restarts at 1. Deeper lines are
/// skipped on the way up; a bullet or a shallower line ends a run. Returns
/// `(line index, digits range, new number)` for items whose number is wrong.
pub fn renumber_block(lines: &[String], changed: Range<usize>) -> Vec<(usize, Range<usize>, u64)> {
    #[derive(Clone)]
    struct Item {
        key: (String, String, String),
        number: Option<u64>,
        digits: Range<usize>,
        bullet: bool,
    }
    let mut items: Vec<Option<Item>> = lines
        .iter()
        .map(|line| {
            if line.trim().is_empty() {
                return None;
            }
            let p = parse_prefix(line);
            let key = (
                line[p.indent.clone()].to_string(),
                line[p.quotes.clone()].to_string(),
                line[p.list_indent.clone()].to_string(),
            );
            Some(match p.marker {
                Some(Marker::Ordered { range, number, .. }) => Item {
                    key,
                    number: Some(number),
                    digits: range.start..range.end - 1,
                    bullet: false,
                },
                Some(Marker::Bullet { .. }) => Item {
                    key,
                    number: None,
                    digits: 0..0,
                    bullet: true,
                },
                None => Item {
                    key,
                    number: None,
                    digits: 0..0,
                    bullet: false,
                },
            })
        })
        .collect();
    let mut out = Vec::new();
    let mut i = changed.start;
    while i < lines.len() {
        let Some(item) = items[i].clone() else {
            break;
        };
        if item.number.is_none() {
            i += 1;
            continue;
        }
        // The previous sibling: up past deeper lines and plain text.
        let mut prev = None;
        let mut j = i;
        while j > 0 {
            j -= 1;
            let Some(above) = &items[j] else {
                break;
            };
            if !above.bullet && above.number.is_none() {
                // Plain text (an item's continuation): not a sibling.
                continue;
            }
            if above.key.0 != item.key.0 || above.key.1 != item.key.1 {
                break;
            }
            if above.key.2.len() > item.key.2.len() {
                continue;
            }
            if above.key.2 != item.key.2 {
                break;
            }
            if above.bullet {
                break;
            }
            prev = above.number;
            break;
        }
        let want = prev.map(|n| n + 1).unwrap_or(1);
        if item.number != Some(want) {
            out.push((i, item.digits.clone(), want));
            if let Some(it) = &mut items[i] {
                it.number = Some(want);
            }
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cont(line: &str) -> Continuation {
        continuation(line, line.len(), 4)
    }

    #[test]
    fn bullets_numbers_and_tasks_continue() {
        assert_eq!(cont("- item"), Continuation::Insert("- ".into()));
        assert_eq!(cont("* item"), Continuation::Insert("* ".into()));
        assert_eq!(cont("3. item"), Continuation::Insert("4. ".into()));
        assert_eq!(cont("3) item"), Continuation::Insert("4) ".into()));
        assert_eq!(cont("- [x] done"), Continuation::Insert("- [ ] ".into()));
        assert_eq!(cont("  - nested"), Continuation::Insert("  - ".into()));
        assert_eq!(cont("\t1. nested"), Continuation::Insert("\t2. ".into()));
    }

    #[test]
    fn quotes_continue_and_combine_with_lists() {
        assert_eq!(cont("> quoted"), Continuation::Insert("> ".into()));
        assert_eq!(cont("> > deep"), Continuation::Insert("> > ".into()));
        assert_eq!(cont("> - item"), Continuation::Insert("> - ".into()));
        assert_eq!(cont("> [!note] Title"), Continuation::Insert("> ".into()));
    }

    #[test]
    fn plain_lines_keep_their_indentation_only() {
        assert_eq!(cont("text"), Continuation::Insert(String::new()));
        assert_eq!(cont("    code"), Continuation::Insert("    ".into()));
        assert_eq!(cont("-not a list"), Continuation::Insert(String::new()));
        assert_eq!(cont("1.5 not a list"), Continuation::Insert(String::new()));
    }

    #[test]
    fn empty_items_clear_or_outdent() {
        assert_eq!(
            cont("- "),
            Continuation::Clear {
                range: 0..2,
                with: String::new()
            }
        );
        assert_eq!(
            cont("- [ ] "),
            Continuation::Clear {
                range: 0..6,
                with: String::new()
            }
        );
        assert_eq!(
            cont("    - "),
            Continuation::Clear {
                range: 0..4,
                with: String::new()
            }
        );
        assert_eq!(
            cont("\t- "),
            Continuation::Clear {
                range: 0..1,
                with: String::new()
            }
        );
        assert_eq!(
            cont("> - "),
            Continuation::Clear {
                range: 2..4,
                with: String::new()
            }
        );
        assert_eq!(
            cont("> "),
            Continuation::Clear {
                range: 0..2,
                with: String::new()
            }
        );
    }

    #[test]
    fn enter_before_the_marker_does_not_continue() {
        assert_eq!(
            continuation("- item", 0, 4),
            Continuation::Insert(String::new())
        );
        assert_eq!(
            continuation("  - item", 1, 4),
            Continuation::Insert("  ".into())
        );
        // Right after the marker's space the item counts as continued.
        assert_eq!(
            continuation("- item", 2, 4),
            Continuation::Insert("- ".into())
        );
    }

    #[test]
    fn indent_and_outdent_points() {
        assert_eq!(indent_point("- a"), 0);
        assert_eq!(indent_point("> - a"), 2);
        assert_eq!(indent_point("text"), 0);
        assert_eq!(outdent_range("    - a", 4), Some(0..4));
        assert_eq!(outdent_range("\t\t- a", 4), Some(0..1));
        assert_eq!(outdent_range("> \t- a", 4), Some(2..3));
        assert_eq!(outdent_range("- a", 4), None);
        assert_eq!(outdent_range("  text", 4), Some(0..2));
        assert_eq!(indent_unit(true, 4), "\t");
        // Four spaces whatever the width: observed under `tabSize: 2`.
        assert_eq!(indent_unit(false, 2), "    ");
    }

    /// Shift+Tab removes one unit of display columns, clamped at zero, a
    /// tab counting `tabSize` columns and the rest written back as tabs
    /// then spaces (Specification.md, "Observed editing command behavior").
    #[test]
    fn shift_tab_removes_one_unit_of_columns() {
        let sp = |n: usize| format!("{}- b", " ".repeat(n));
        // `tabSize: 2`, `useTab: true`.
        assert_eq!(outdent_edit(&sp(3), true, 2), Some((0..3, " ".into())));
        assert_eq!(outdent_edit(&sp(2), true, 2), Some((0..2, "".into())));
        assert_eq!(outdent_edit(&sp(1), true, 2), Some((0..1, "".into())));
        assert_eq!(outdent_edit(&sp(5), true, 2), Some((0..5, "\t ".into())));
        assert_eq!(outdent_edit("\t- b", true, 2), Some((0..1, "".into())));
        assert_eq!(outdent_edit("\t - b", true, 2), Some((0..2, " ".into())));
        assert_eq!(outdent_edit("   plain", true, 2), Some((0..3, " ".into())));
        assert_eq!(outdent_edit("- a", true, 2), None);
        // `tabSize: 4`, `useTab: true`.
        assert_eq!(outdent_edit(&sp(3), true, 4), Some((0..3, "".into())));
        assert_eq!(outdent_edit(&sp(5), true, 4), Some((0..5, " ".into())));
        assert_eq!(outdent_edit("\t - b", true, 4), Some((0..2, " ".into())));
        assert_eq!(outdent_edit("   plain", true, 4), Some((0..3, "".into())));
        // `useTab: false`: the unit is four columns whatever `tabSize` says.
        assert_eq!(outdent_edit(&sp(3), false, 2), Some((0..3, "".into())));
        assert_eq!(outdent_edit(&sp(5), false, 2), Some((0..5, " ".into())));
        // After a quote marker the list's own indentation is what goes.
        assert_eq!(outdent_edit("> \t- a", true, 4), Some((2..3, "".into())));
    }

    /// Shift+Tab, and Enter on an empty nested item, give back exactly what
    /// Tab put there: one `tabSize` step, not the fixed four spaces that
    /// outdented two levels at a time in a two-space vault.
    #[test]
    fn an_outdent_step_is_the_configured_width() {
        assert_eq!(outdent_range("    - a", 2), Some(0..2));
        assert_eq!(outdent_range("  - a", 2), Some(0..2));
        assert_eq!(outdent_range("        - a", 8), Some(0..8));
        // A tab is one level whatever the width is.
        assert_eq!(outdent_range("\t\t- a", 2), Some(0..1));
        // Indentation shallower than a step gives up what it has.
        assert_eq!(outdent_range(" - a", 2), Some(0..1));
        assert_eq!(
            continuation("    - ", 6, 2),
            Continuation::Clear {
                range: 0..2,
                with: String::new()
            }
        );
    }

    #[test]
    fn following_ordered_items_renumber() {
        let lines = ["2. b", "   - nested", "3. c", "9. d", "", "7. e"];
        let fixes = renumber_following(lines.iter().copied(), "1. a", 2);
        assert_eq!(fixes, vec![(3, 0..1, 4)]);
        let fixes = renumber_following(lines.iter().copied(), "1. a", 3);
        assert_eq!(fixes, vec![(0, 0..1, 3), (2, 0..1, 4), (3, 0..1, 5)]);
        // A bullet or a shallower line ends the run.
        let lines = ["  2. b", "- c"];
        assert_eq!(
            renumber_following(lines.iter().copied(), "  1. a", 2),
            vec![]
        );
    }

    fn block(lines: &[&str], changed: Range<usize>) -> Vec<(usize, Range<usize>, u64)> {
        let lines: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
        renumber_block(&lines, changed)
    }

    #[test]
    fn indented_items_restart_and_the_rest_close_the_gap() {
        // `2. b` was indented under `1. a`.
        let lines = ["1. a", "\t2. b", "3. c", "4. d", "", "9. e"];
        assert_eq!(
            block(&lines, 1..2),
            vec![(1, 1..2, 1), (2, 0..1, 2), (3, 0..1, 3)]
        );
        // The first item was indented: the rest restart at 1.
        let lines = ["\t1. a", "2. b", "3. c"];
        assert_eq!(block(&lines, 0..1), vec![(1, 0..1, 1), (2, 0..1, 2)]);
        // Outdented back: it follows its new sibling and its old siblings
        // below become a nested list that restarts.
        let lines = ["1. a", "\t1. x", "1. b", "\t3. y", "\t4. z", "2. c"];
        assert_eq!(
            block(&lines, 2..3),
            vec![(2, 0..1, 2), (3, 1..2, 1), (4, 1..2, 2), (5, 0..1, 3)]
        );
        // Bullets and quotes bound a run; plain lines are skipped.
        let lines = ["- a", "\t5. b", "> 1. q", "> \t7. r"];
        assert_eq!(block(&lines, 1..2), vec![(1, 1..2, 1), (3, 3..4, 1)]);
        let lines = ["1. a", "   text", "7. b"];
        assert_eq!(block(&lines, 2..3), vec![(2, 0..1, 2)]);
    }
}
