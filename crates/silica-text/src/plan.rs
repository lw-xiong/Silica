//! What the editing commands plan: the edits Enter, Tab and Shift+Tab make
//! over the text, and where a byte offset lands after a set of edits. Pure
//! functions over the text and its line index, so the editor's keys and a
//! round-trip test take the same path.

use crate::lists::{self, Continuation};
use std::collections::HashMap;
use std::ops::Range;

/// The edits Enter makes at `head` with smart lists on: the newline and
/// continued prefix (or the clearing of an empty item, which outdents by one
/// `tab_size` step), plus the renumbering of the ordered items that follow.
/// Returns them with the cursor's new offset.
pub fn newline_plan(
    text: &str,
    line_starts: &[usize],
    head: usize,
    tab_size: u8,
) -> (Vec<(Range<usize>, String)>, usize) {
    let line_ix = match line_starts.binary_search(&head) {
        Ok(i) => i,
        Err(i) => i - 1,
    };
    let line_start = line_starts[line_ix];
    let line_end = line_starts
        .get(line_ix + 1)
        .map(|n| n - 1)
        .unwrap_or(text.len());
    let line = &text[line_start..line_end];
    let col = head - line_start;
    match lists::continuation(line, col, tab_size) {
        Continuation::Insert(prefix) => {
            let mut edits = vec![(head..head, format!("\n{prefix}"))];
            let cursor = head + 1 + prefix.len();
            let p = lists::parse_prefix(line);
            if let Some(lists::Marker::Ordered { number, .. }) = p.marker {
                if col >= p.content {
                    let following = line_starts[line_ix + 1..]
                        .iter()
                        .enumerate()
                        .map(|(i, &s)| {
                            let e = line_starts
                                .get(line_ix + 2 + i)
                                .map(|n| n - 1)
                                .unwrap_or(text.len());
                            &text[s..e]
                        });
                    for (ix, range, n) in lists::renumber_following(following, line, number + 2) {
                        let at = line_starts[line_ix + 1 + ix];
                        edits.push((at + range.start..at + range.end, n.to_string()));
                    }
                }
            }
            (edits, cursor)
        }
        Continuation::Clear { range, with } => {
            let abs = line_start + range.start..line_start + range.end;
            let cursor = abs.start + with.len() + head.saturating_sub(abs.end);
            (vec![(abs, with)], cursor)
        }
    }
}

/// The edits Tab (`indent`) or Shift+Tab make on `lines`: one indentation
/// step of `unit` added after each line's quote markers, or one `tab_size`
/// step removed, plus (with `renumber`) the new numbers of the ordered items
/// the change unsettled, all in pre-edit coordinates for one undo step.
// The two indent settings and the two switches are what the one command
// takes; a struct for them would be read by exactly two callers.
#[allow(clippy::too_many_arguments)]
pub fn indent_plan(
    text: &str,
    line_starts: &[usize],
    lines: Range<usize>,
    unit: &str,
    use_tab: bool,
    tab_size: u8,
    indent: bool,
    renumber: bool,
) -> Vec<(Range<usize>, String)> {
    let line_text = |l: usize| -> &str {
        let start = line_starts[l];
        let end = line_starts.get(l + 1).map(|n| n - 1).unwrap_or(text.len());
        &text[start..end]
    };
    let mut edits = Vec::new();
    // Bytes each changed line gained before its marker, post minus pre.
    let mut deltas: HashMap<usize, isize> = HashMap::new();
    for l in lines.clone() {
        let lt = line_text(l);
        if indent {
            let at = line_starts[l] + lists::indent_point(lt);
            edits.push((at..at, unit.to_string()));
            deltas.insert(l, unit.len() as isize);
        } else if let Some((r, new)) = lists::outdent_edit(lt, use_tab, tab_size) {
            let start = line_starts[l];
            deltas.insert(l, new.len() as isize - r.len() as isize);
            edits.push((start + r.start..start + r.end, new));
        }
    }
    if !renumber || edits.is_empty() {
        return edits;
    }
    // The list block around the change: its lines up to a blank one
    // either side, as they will read after the edit.
    let mut first = lines.start;
    while first > 0 && !line_text(first - 1).trim().is_empty() {
        first -= 1;
    }
    let mut last = lines.end;
    while last < line_starts.len() && !line_text(last).trim().is_empty() {
        last += 1;
    }
    let block: Vec<String> = (first..last)
        .map(|l| {
            let lt = line_text(l);
            match deltas.get(&l) {
                Some(d) if *d > 0 => {
                    let at = lists::indent_point(lt);
                    format!("{}{}{}", &lt[..at], unit, &lt[at..])
                }
                Some(_) => {
                    let (r, new) =
                        lists::outdent_edit(lt, use_tab, tab_size).unwrap_or((0..0, String::new()));
                    format!("{}{}{}", &lt[..r.start], new, &lt[r.end..])
                }
                None => lt.to_string(),
            }
        })
        .collect();
    let changed = lines.start - first..lines.end - first;
    for (ix, digits, n) in lists::renumber_block(&block, changed) {
        let l = first + ix;
        let delta = deltas.get(&l).copied().unwrap_or(0);
        let start = (digits.start as isize - delta) as usize;
        let at = line_starts[l] + start;
        edits.push((at..at + digits.len(), n.to_string()));
    }
    edits
}

/// Where byte `at` of the text sits after `edits` (disjoint, in any order)
/// are applied; a position inside a replaced range lands after its
/// replacement, and an insertion exactly at `at` goes before it.
pub fn shifted(at: usize, edits: &[(Range<usize>, String)]) -> usize {
    let mut sorted: Vec<&(Range<usize>, String)> = edits.iter().collect();
    sorted.sort_by_key(|(r, _)| (r.start, r.end));
    let mut delta: isize = 0;
    for (r, new) in sorted {
        if r.end <= at {
            delta += new.len() as isize - r.len() as isize;
        } else if r.start < at {
            return (r.start as isize + delta) as usize + new.len();
        } else {
            break;
        }
    }
    (at as isize + delta) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line_starts(text: &str) -> Vec<usize> {
        std::iter::once(0)
            .chain(text.match_indices('\n').map(|(i, _)| i + 1))
            .collect()
    }

    fn apply(text: &str, head: usize) -> (String, usize) {
        let (mut edits, cursor) = newline_plan(text, &line_starts(text), head, 4);
        edits.sort_by_key(|e| std::cmp::Reverse(e.0.start));
        let mut out = text.to_string();
        for (r, s) in edits {
            out.replace_range(r, &s);
        }
        (out, cursor)
    }

    #[test]
    fn enter_continues_items_and_renumbers_the_rest() {
        assert_eq!(apply("- a", 3), ("- a\n- ".into(), 6));
        assert_eq!(
            apply("1. a\n2. b\n3. c", 4),
            ("1. a\n2. \n3. b\n4. c".into(), 8)
        );
        // Splitting an item carries the rest of the text to the new one.
        assert_eq!(apply("- ab", 3), ("- a\n- b".into(), 6));
        // An empty item clears; a nested one outdents and keeps the cursor.
        assert_eq!(apply("- a\n- ", 6), ("- a\n".into(), 4));
        assert_eq!(apply("- a\n\t- ", 7), ("- a\n- ".into(), 6));
        assert_eq!(apply("> q", 3), ("> q\n> ".into(), 6));
    }

    fn apply_many(text: &str, edits: Vec<(Range<usize>, String)>) -> String {
        let mut edits = edits;
        edits.sort_by_key(|e| (std::cmp::Reverse(e.0.start), std::cmp::Reverse(e.0.end)));
        let mut out = text.to_string();
        for (r, new) in edits {
            out.replace_range(r, &new);
        }
        out
    }

    #[test]
    fn tab_and_shift_tab_renumber_ordered_lists() {
        let text = "1. a\n2. b\n3. c\n";
        let edits = indent_plan(text, &line_starts(text), 1..2, "\t", true, 4, true, true);
        assert_eq!(apply_many(text, edits), "1. a\n\t1. b\n2. c\n");
        let text = "1. a\n\t1. b\n2. c\n";
        let edits = indent_plan(text, &line_starts(text), 1..2, "", true, 4, false, true);
        assert_eq!(apply_many(text, edits), "1. a\n2. b\n3. c\n");
        // Without smart lists only the indentation changes.
        let text = "1. a\n2. b\n3. c\n";
        let edits = indent_plan(text, &line_starts(text), 1..2, "  ", true, 4, true, false);
        assert_eq!(apply_many(text, edits), "1. a\n  2. b\n3. c\n");
        // A selection over two items indents both; numbers follow.
        let text = "1. a\n2. b\n3. c\n4. d\n";
        let edits = indent_plan(text, &line_starts(text), 1..3, "\t", true, 4, true, true);
        assert_eq!(apply_many(text, edits), "1. a\n\t1. b\n\t2. c\n2. d\n");
    }

    #[test]
    fn shifted_positions_follow_the_edits() {
        let edits = vec![(2..2, "\t".to_string()), (10..11, "12".to_string())];
        assert_eq!(shifted(0, &edits), 0);
        assert_eq!(shifted(2, &edits), 3);
        assert_eq!(shifted(5, &edits), 6);
        assert_eq!(shifted(11, &edits), 13);
        assert_eq!(shifted(20, &edits), 22);
        // Inside a replaced range: after the replacement.
        let edits = vec![(3..6, "".to_string())];
        assert_eq!(shifted(4, &edits), 3);
    }
}
