//! What Obsidian's formatting commands write: the `**`/`*`/`==` toggles,
//! the heading and list prefixes, the link shapes, moving a line and
//! deleting a paragraph.
//!
//! Pure functions over the text the way [`crate::lists`] and
//! [`crate::plan`] are: each returns the edits in pre-edit coordinates, so
//! the buffer can apply a whole command as one undo step and a test can
//! read the bytes without a window.

use crate::lists::{self, Marker};
use silica_markdown::HighlightColor;
use std::ops::Range;

/// One line of `text`, without its newline.
fn line_text<'a>(text: &'a str, line_starts: &[usize], line: usize) -> &'a str {
    let start = line_starts[line];
    let end = line_starts
        .get(line + 1)
        .map(|n| n - 1)
        .unwrap_or(text.len());
    &text[start..end]
}

/// Where the list marker of `line` is, or would go: after the indentation
/// and any quote markers.
fn marker_point(prefix: &lists::Prefix) -> usize {
    match &prefix.marker {
        Some(Marker::Bullet { range, .. }) | Some(Marker::Ordered { range, .. }) => range.start,
        None => prefix.content,
    }
}

/// The marker and the space after it, which a list toggle removes.
fn marker_span(line: &str, prefix: &lists::Prefix) -> Option<Range<usize>> {
    let range = match &prefix.marker {
        Some(Marker::Bullet { range, .. }) | Some(Marker::Ordered { range, .. }) => range.clone(),
        None => return None,
    };
    let mut end = range.end;
    if line.as_bytes().get(end) == Some(&b' ') {
        end += 1;
    }
    Some(range.start..end)
}

/// Whether `marker` starts at byte `at` of `text`.
fn marker_at(text: &str, at: usize, marker: &str) -> bool {
    text.get(at..at + marker.len()) == Some(marker)
}

/// What a formatting toggle does to one selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Toggle {
    /// The marker is already there: strip these two ranges, the opener
    /// (with a highlight's colour emoji) and the closer.
    Strip(Range<usize>, Range<usize>),
    /// Wrap this range in the marker; empty for a bare caret, which then
    /// gets the pair with the caret between the halves.
    Wrap(Range<usize>),
    /// A highlight is already there in another colour: replace this range
    /// (the old emoji, or empty just after the opening `==`) with the new
    /// one.
    Recolor(Range<usize>, &'static str),
}

/// The colour emoji at `at`, if one of the six is there.
fn color_at(text: &str, at: usize) -> Option<(HighlightColor, usize)> {
    text.get(at..).and_then(HighlightColor::leading)
}

/// What `editor:toggle-bold` and the other wrapping toggles do to the
/// selection `sel`: strip the marker the selection already carries, inside
/// it or just outside it, or add the pair. `floor` is the end of the
/// previous selection's text, which an outward strip must not reach into.
///
/// A marker is only recognised when it is not part of a longer run of the
/// same character, so italics inside `**bold**` adds a third `*` rather
/// than eating one of the two that make the bold.
pub fn toggle_wrap(text: &str, sel: Range<usize>, marker: &str, floor: usize) -> Toggle {
    toggle_wrap_colored(text, sel, marker, None, floor)
}

/// [`toggle_wrap`] for the highlight commands, which carry a colour: the
/// six `editor:toggle-highlight-<colour>` commands write one emoji
/// directly after the opening `==`. The same colour again removes the
/// whole highlight, a different one replaces the emoji in place, and a
/// colour over a plain `==word==` inserts it, whether the selection is the
/// inner word or spans the marks. The plain toggle over a coloured
/// highlight takes the highlight off, emoji included.
pub fn toggle_wrap_colored(
    text: &str,
    sel: Range<usize>,
    marker: &str,
    color: Option<HighlightColor>,
    floor: usize,
) -> Toggle {
    let m = marker.len();
    let colored = marker == "==";
    // How many bytes of colour emoji sit at `at`; none for the other
    // markers, whose pairs carry nothing.
    let emoji_len = |at: usize| -> usize {
        if colored {
            color_at(text, at).map_or(0, |(_, n)| n)
        } else {
            0
        }
    };
    // `open` is the opener's range, emoji included; `close` the closer's.
    let found: Option<(Range<usize>, Range<usize>)> = (|| {
        // The markers inside the selection: `[==🔴word==]`.
        if sel.len() >= 2 * m
            && marker_at(text, sel.start, marker)
            && marker_at(text, sel.end - m, marker)
            && (sel.len() == 2 * m || !marker_at(text, sel.start + m, marker))
        {
            let e = emoji_len(sel.start + m);
            if sel.start + m + e <= sel.end - m {
                return Some((sel.start..sel.start + m + e, sel.end - m..sel.end));
            }
        }
        // The markers just outside it: `==🔴[word]==` or `==[🔴word]==`.
        if !marker_at(text, sel.end, marker) || marker_at(text, sel.end + m, marker) {
            return None;
        }
        let start = if sel.is_empty() {
            sel.start
        } else {
            sel.start + emoji_len(sel.start)
        };
        for pre in [0, 4] {
            if pre > 0 && !colored {
                break;
            }
            if start < floor + m + pre {
                continue;
            }
            let open_start = start - m - pre;
            if !marker_at(text, open_start, marker)
                || (pre > 0 && emoji_len(open_start + m) != pre)
                || (open_start >= m && marker_at(text, open_start - m, marker))
            {
                continue;
            }
            return Some((open_start..start, sel.end..sel.end + m));
        }
        None
    })();
    let Some((open, close)) = found else {
        return Toggle::Wrap(sel);
    };
    let old = if colored {
        color_at(text, open.start + m).map(|(c, _)| c)
    } else {
        None
    };
    match color {
        Some(c) if old != Some(c) => Toggle::Recolor(open.start + m..open.end, c.emoji()),
        _ => Toggle::Strip(open, close),
    }
}

/// What a line is to the two checklist commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ListState {
    /// Not a list item.
    Plain,
    /// A list item with no checkbox.
    Bullet,
    /// `[ ]`.
    Open,
    /// Any other status: `[x]`, `[X]`, `[?]`, which the commands all treat
    /// as checked, as the rendered click does.
    Checked,
}

fn list_state(lt: &str, p: &lists::Prefix) -> ListState {
    match (&p.marker, &p.task) {
        (None, _) => ListState::Plain,
        (Some(_), None) => ListState::Bullet,
        (Some(_), Some(t)) if lt.as_bytes()[t.start + 1] == b' ' => ListState::Open,
        _ => ListState::Checked,
    }
}

/// The edit that puts a checkbox on a line without one: `[ ] ` after the
/// list marker's space, or `- [ ] ` where a marker would go — after the
/// indentation and any `> `.
fn add_checkbox(lt: &str, p: &lists::Prefix) -> (Range<usize>, String) {
    match &p.marker {
        Some(Marker::Bullet { range, .. }) | Some(Marker::Ordered { range, .. }) => {
            let end = range.end;
            if lt.as_bytes().get(end) == Some(&b' ') {
                (end + 1..end + 1, "[ ] ".to_string())
            } else {
                (end..end, " [ ]".to_string())
            }
        }
        None => (p.content..p.content, "- [ ] ".to_string()),
    }
}

/// The edit that takes a line's checkbox off, the space after it too.
fn remove_checkbox(p: &lists::Prefix) -> (Range<usize>, String) {
    let t = p.task.clone().expect("the line has a checkbox");
    (t, String::new())
}

/// The edit that writes `[x]` or `[ ]` over a line's checkbox.
fn set_checkbox(p: &lists::Prefix, checked: bool) -> (Range<usize>, String) {
    let t = p.task.clone().expect("the line has a checkbox");
    let with = if checked { "[x]" } else { "[ ]" };
    (t.start..t.start + 3, with.to_string())
}

/// What `editor:toggle-checklist-status` (Mod+L) writes on `lines`
/// (Specification.md, "Observed editing command behavior"). One line
/// gains `- [ ] ` when it has no checkbox — after a list marker, a `> `
/// and leading whitespace — and flips the one it has otherwise, any
/// status but a space counting as checked. Over a selection the decision
/// is taken once, blank lines ignored: every line has a box, check them
/// all unless all are checked; no line has one, add one to all; some do,
/// invert each line's.
pub fn checklist_plan(
    text: &str,
    line_starts: &[usize],
    lines: Range<usize>,
) -> Vec<(Range<usize>, String)> {
    let at = |l: usize, (r, s): (Range<usize>, String)| {
        let start = line_starts[l];
        (start + r.start..start + r.end, s)
    };
    if lines.len() == 1 {
        let l = lines.start;
        let lt = line_text(text, line_starts, l);
        let p = lists::parse_prefix(lt);
        let edit = match list_state(lt, &p) {
            ListState::Plain | ListState::Bullet => add_checkbox(lt, &p),
            ListState::Open => set_checkbox(&p, true),
            ListState::Checked => set_checkbox(&p, false),
        };
        return vec![at(l, edit)];
    }
    let touched: Vec<(usize, &str, lists::Prefix)> = lines
        .filter_map(|l| {
            let lt = line_text(text, line_starts, l);
            (!lt.trim().is_empty()).then(|| (l, lt, lists::parse_prefix(lt)))
        })
        .collect();
    let states: Vec<ListState> = touched.iter().map(|(_, lt, p)| list_state(lt, p)).collect();
    let has_box = |s: &ListState| matches!(s, ListState::Open | ListState::Checked);
    let all = states.iter().all(has_box);
    let none = !states.iter().any(has_box);
    let all_checked = states.iter().all(|s| *s == ListState::Checked);
    touched
        .iter()
        .zip(&states)
        .map(|((l, lt, p), state)| {
            let edit = if all {
                set_checkbox(p, !all_checked)
            } else if none || !has_box(state) {
                add_checkbox(lt, p)
            } else {
                remove_checkbox(p)
            };
            at(*l, edit)
        })
        .collect()
}

/// What `editor:cycle-list-checklist` writes on `lines`: bullet → `[ ]`
/// → `[x]` → bullet, a line that is not a list item becoming a bullet
/// first, the marker kept as written and a custom status discarded on the
/// first press. Over a selection (blank lines ignored) a line that is not
/// a list item pulls every line back to a bullet with it; otherwise each
/// line advances one step. Obsidian's own output oscillates for a
/// selection of list lines that differ only in their checkbox; this is
/// the converging rule instead.
pub fn cycle_checklist_plan(
    text: &str,
    line_starts: &[usize],
    lines: Range<usize>,
) -> Vec<(Range<usize>, String)> {
    let at = |l: usize, (r, s): (Range<usize>, String)| {
        let start = line_starts[l];
        (start + r.start..start + r.end, s)
    };
    let single = lines.len() == 1;
    let touched: Vec<(usize, &str, lists::Prefix)> = lines
        .filter_map(|l| {
            let lt = line_text(text, line_starts, l);
            (single || !lt.trim().is_empty()).then(|| (l, lt, lists::parse_prefix(lt)))
        })
        .collect();
    let states: Vec<ListState> = touched.iter().map(|(_, lt, p)| list_state(lt, p)).collect();
    let normalise = !single && states.contains(&ListState::Plain);
    touched
        .iter()
        .zip(&states)
        .filter_map(|((l, lt, p), state)| {
            let edit = match (normalise, state) {
                (_, ListState::Plain) => (p.content..p.content, "- ".to_string()),
                (true, ListState::Bullet) => return None,
                (true, _) => remove_checkbox(p),
                (false, ListState::Bullet) => add_checkbox(lt, p),
                (false, ListState::Open) => set_checkbox(p, true),
                (false, ListState::Checked) => remove_checkbox(p),
            };
            Some(at(*l, edit))
        })
        .collect()
}

/// The six insert-block commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockInsert {
    /// `editor:insert-callout`.
    Callout,
    /// `editor:insert-codeblock`.
    CodeBlock,
    /// `editor:insert-mathblock`.
    MathBlock,
    /// `editor:insert-horizontal-rule`.
    HorizontalRule,
    /// `editor:insert-tag`: a bare `#`, no suggester.
    Tag,
    /// `editor:insert-footnote`.
    Footnote,
}

impl BlockInsert {
    pub fn from_id(id: &str) -> Option<BlockInsert> {
        Some(match id {
            "editor:insert-callout" => BlockInsert::Callout,
            "editor:insert-codeblock" => BlockInsert::CodeBlock,
            "editor:insert-mathblock" => BlockInsert::MathBlock,
            "editor:insert-horizontal-rule" => BlockInsert::HorizontalRule,
            "editor:insert-tag" => BlockInsert::Tag,
            "editor:insert-footnote" => BlockInsert::Footnote,
            _ => return None,
        })
    }
}

/// The number `editor:insert-footnote` uses: the highest numeric label in
/// the document plus one, a named label not taking a number.
pub fn next_footnote_number(text: &str) -> u64 {
    let re = regex::Regex::new(r"\[\^(\d+)\]").expect("a valid pattern");
    re.captures_iter(text)
        .filter_map(|c| c[1].parse::<u64>().ok())
        .max()
        .unwrap_or(0)
        + 1
}

/// What one of the insert-block commands writes at `caret`, and the
/// selection it leaves, in post-edit coordinates. The selection is
/// ignored by all six: the caret's line is what they act on. All but
/// `insert-callout` leave a bare caret.
pub fn block_insert_plan(
    text: &str,
    line_starts: &[usize],
    caret: usize,
    kind: BlockInsert,
) -> (Vec<(Range<usize>, String)>, Range<usize>) {
    let line = line_starts.partition_point(|&s| s <= caret) - 1;
    let lr = line_starts[line]..line_starts.get(line + 1).map_or(text.len(), |n| n - 1);
    let lt = &text[lr.clone()];
    let blank = lt.trim().is_empty();
    match kind {
        BlockInsert::Callout => {
            let (new, note) = if blank {
                (
                    "\n> [!NOTE] Title\n> Contents\n".to_string(),
                    lr.start + 1 + 4,
                )
            } else {
                (format!("> [!NOTE]\n> {lt}"), lr.start + 4)
            };
            (vec![(lr, new)], note..note + 4)
        }
        BlockInsert::CodeBlock | BlockInsert::MathBlock => {
            let fence = if kind == BlockInsert::CodeBlock {
                "```"
            } else {
                "$$"
            };
            let (new, at) = if blank {
                (format!("{fence}\n\n{fence}"), lr.start + fence.len() + 1)
            } else {
                (format!("{fence}\n{lt}\n{fence}"), caret + fence.len() + 1)
            };
            (vec![(lr, new)], at..at)
        }
        BlockInsert::HorizontalRule => {
            let new = if blank { "\n---\n" } else { "\n\n---\n" };
            let at = caret + new.len();
            (vec![(caret..caret, new.to_string())], at..at)
        }
        BlockInsert::Tag => (vec![(caret..caret, "#".to_string())], caret + 1..caret + 1),
        BlockInsert::Footnote => {
            let n = next_footnote_number(text);
            let reference = format!("[^{n}]");
            let trailing = text.len() - text.trim_end_matches('\n').len();
            let definition = format!("{}[^{n}]: \n", "\n".repeat(2usize.saturating_sub(trailing)));
            let at = caret + reference.len();
            // At the very end the two insertions are one, so they cannot
            // land in the wrong order.
            let edits = if caret == text.len() {
                vec![(caret..caret, format!("{reference}{definition}"))]
            } else {
                vec![
                    (caret..caret, reference),
                    (text.len()..text.len(), definition),
                ]
            };
            (edits, at..at)
        }
    }
}

/// What `editor:clear-formatting` removes from `sel`: the markers of
/// emphasis, strong, strikethrough, highlight (colour emoji included),
/// inline code and inline math that lie inside the selection, the `#` of
/// a tag, and the `#`s, `> ` and list marker of a line whose prefix the
/// selection covers. Links are not formatting and are left alone. Empty
/// without a selection.
pub fn clear_formatting_plan(
    text: &str,
    line_starts: &[usize],
    sel: Range<usize>,
) -> Vec<(Range<usize>, String)> {
    use silica_markdown::{walk_blocks, walk_inlines, BlockKind, InlineKind};
    if sel.is_empty() {
        return Vec::new();
    }
    let mut cuts: Vec<Range<usize>> = Vec::new();
    let mut cut = |r: Range<usize>| {
        if r.start >= sel.start && r.end <= sel.end && !r.is_empty() {
            cuts.push(r);
        }
    };
    let doc = silica_markdown::parse(text);
    let mut inline_lists: Vec<&[silica_markdown::Inline<'_>]> = Vec::new();
    walk_blocks(&doc.blocks, &mut |b| match &b.kind {
        BlockKind::Paragraph { content } | BlockKind::Heading { content, .. } => {
            inline_lists.push(content)
        }
        // A callout's title is block-parsed, so its inlines come out of the
        // block tree rather than straight off the field.
        BlockKind::Callout { title, .. } => {
            inline_lists.extend(silica_markdown::inline_runs(title))
        }
        BlockKind::Table { header, rows, .. } => {
            for c in header.iter().chain(rows.iter().flatten()) {
                inline_lists.push(&c.content);
            }
        }
        _ => {}
    });
    for inlines in inline_lists {
        walk_inlines(inlines, &mut |i| {
            let (s, e) = (i.span.start, i.span.end);
            let run = |at: usize, ch: u8| text[at..].bytes().take_while(|&b| b == ch).count();
            match &i.kind {
                InlineKind::Emphasis(_) => {
                    cut(s..s + 1);
                    cut(e - 1..e);
                }
                InlineKind::Strong(_) | InlineKind::Strikethrough(_) => {
                    cut(s..s + 2);
                    cut(e - 2..e);
                }
                InlineKind::Highlight { .. } => {
                    let emoji = color_at(text, s + 2).map_or(0, |(_, n)| n);
                    cut(s..s + 2 + emoji);
                    cut(e - 2..e);
                }
                InlineKind::Code(_) => {
                    let n = run(s, b'`');
                    cut(s..s + n);
                    cut(e - n..e);
                }
                InlineKind::Math { display, .. } => {
                    let n = if *display { 2 } else { 1 };
                    cut(s..s + n);
                    cut(e - n..e);
                }
                InlineKind::Tag(_) => cut(s..s + 1),
                _ => {}
            }
        });
    }
    // The prefix of each line the selection covers from its start.
    let first = line_starts.partition_point(|&s| s <= sel.start) - 1;
    let last = line_starts.partition_point(|&s| s < sel.end).max(first + 1) - 1;
    for l in first..=last {
        let start = line_starts[l];
        if start < sel.start {
            continue;
        }
        let lt = line_text(text, line_starts, l);
        let p = lists::parse_prefix(lt);
        let mut end = p.quotes.end;
        if let Some(span) = marker_span(lt, &p) {
            end = span.end;
        }
        if let Some(t) = &p.task {
            end = t.end;
        }
        let (heading, _) = heading_at(&lt[end..]);
        end += heading.end;
        if start + end <= sel.end {
            cuts.push(start + p.indent.start..start + end);
            // The indentation is kept; only what follows it goes.
            if let Some(last) = cuts.last_mut() {
                last.start = start + p.quotes.start.min(end);
            }
        }
    }
    cuts.sort_by_key(|r| r.start);
    cuts.dedup();
    cuts.into_iter().map(|r| (r, String::new())).collect()
}

/// Where a heading's `#`s sit on `line`: after the indentation and any
/// quote markers.
fn heading_point(line: &str) -> usize {
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
        i += 1;
    }
    while i < b.len() && b[i] == b'>' {
        i += 1;
        if i < b.len() && b[i] == b' ' {
            i += 1;
        }
    }
    i
}

/// The `#`s already on `line` with the space after them, and their level.
fn heading_at(line: &str) -> (Range<usize>, u8) {
    let start = heading_point(line);
    let b = line.as_bytes();
    let mut i = start;
    while i < b.len() && b[i] == b'#' && i - start < 6 {
        i += 1;
    }
    if i == start || (i < b.len() && b[i] != b' ') {
        return (start..start, 0);
    }
    let level = (i - start) as u8;
    if i < b.len() && b[i] == b' ' {
        i += 1;
    }
    (start..i, level)
}

/// What `editor:set-heading-<n>` writes on `lines`: the line's `#`s
/// replaced by `level` of them, or removed when `level` is 0. Blank lines
/// in a multi-line selection are left alone.
pub fn heading_plan(
    text: &str,
    line_starts: &[usize],
    lines: Range<usize>,
    level: u8,
) -> Vec<(Range<usize>, String)> {
    let single = lines.len() == 1;
    let mut edits = Vec::new();
    for l in lines {
        let lt = line_text(text, line_starts, l);
        if lt.trim().is_empty() && !single {
            continue;
        }
        let (range, at) = heading_at(lt);
        if at == level {
            continue;
        }
        let with = if level == 0 {
            String::new()
        } else {
            format!("{} ", "#".repeat(level as usize))
        };
        let start = line_starts[l];
        edits.push((start + range.start..start + range.end, with));
    }
    edits
}

/// The line prefix a toggle adds or removes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinePrefix {
    Bullet,
    Ordered,
    Quote,
}

/// Whether the line already carries `kind`.
fn has_prefix(prefix: &lists::Prefix, kind: LinePrefix) -> bool {
    match kind {
        LinePrefix::Bullet => matches!(prefix.marker, Some(Marker::Bullet { .. })),
        LinePrefix::Ordered => matches!(prefix.marker, Some(Marker::Ordered { .. })),
        LinePrefix::Quote => !prefix.quotes.is_empty(),
    }
}

/// What `editor:toggle-bullet-list`, `-numbered-list` and
/// `-blockquote` write on `lines`: the prefix is removed when every line
/// already has it and added to the ones that do not otherwise, which is
/// how a toggle over a mixed selection settles. An ordered list numbers
/// from 1 down the selection. Blank lines are skipped unless the selection
/// is a single line.
pub fn line_prefix_plan(
    text: &str,
    line_starts: &[usize],
    lines: Range<usize>,
    kind: LinePrefix,
) -> Vec<(Range<usize>, String)> {
    let single = lines.len() == 1;
    let touched: Vec<usize> = lines
        .filter(|l| single || !line_text(text, line_starts, *l).trim().is_empty())
        .collect();
    let all = !touched.is_empty()
        && touched.iter().all(|l| {
            let lt = line_text(text, line_starts, *l);
            has_prefix(&lists::parse_prefix(lt), kind)
        });
    let mut edits = Vec::new();
    let mut number = 1;
    for l in touched {
        let lt = line_text(text, line_starts, l);
        let start = line_starts[l];
        let p = lists::parse_prefix(lt);
        match kind {
            LinePrefix::Quote => {
                if all {
                    // The outermost `> ` of however many the line carries.
                    let at = lt.find('>').unwrap_or(0);
                    let mut end = at + 1;
                    if lt.as_bytes().get(end) == Some(&b' ') {
                        end += 1;
                    }
                    edits.push((start + at..start + end, String::new()));
                } else if !has_prefix(&p, kind) {
                    edits.push((start..start, "> ".to_string()));
                }
            }
            LinePrefix::Bullet | LinePrefix::Ordered => {
                let span = marker_span(lt, &p);
                if all {
                    let span = span.expect("every line was found to have a marker");
                    edits.push((start + span.start..start + span.end, String::new()));
                    continue;
                }
                // An item of the other kind is rewritten; one of this kind
                // is left alone, except that an ordered list renumbers.
                let ordered = kind == LinePrefix::Ordered;
                if has_prefix(&p, kind) && !ordered {
                    continue;
                }
                let with = if ordered {
                    let n = format!("{number}. ");
                    number += 1;
                    n
                } else {
                    "- ".to_string()
                };
                match span {
                    Some(span) => edits.push((start + span.start..start + span.end, with)),
                    None => {
                        let at = start + marker_point(&p);
                        edits.push((at..at, with));
                    }
                }
            }
        }
    }
    // A renumbering pass leaves the items that already had their number.
    edits.retain(|(r, new)| text[r.clone()] != **new);
    edits
}

/// The lines `editor:delete-paragraph` removes around `lines`: out to the
/// blank line either side, or the blank line itself when that is where the
/// cursor is. The newline before a block at the end of the file goes with
/// it, so the deletion does not leave a trailing blank line.
pub fn paragraph_span(text: &str, line_starts: &[usize], lines: Range<usize>) -> Range<usize> {
    let blank = |l: usize| line_text(text, line_starts, l).trim().is_empty();
    let mut first = lines.start;
    let mut last = lines.end - 1;
    if !blank(first) {
        while first > 0 && !blank(first - 1) {
            first -= 1;
        }
    }
    if !blank(last) {
        while last + 1 < line_starts.len() && !blank(last + 1) {
            last += 1;
        }
    }
    let start = line_starts[first];
    let end = line_starts.get(last + 1).copied().unwrap_or(text.len());
    if end == text.len() && start > 0 {
        start - 1..end
    } else {
        start..end
    }
}

/// The one replacement that moves `lines` over the line above or below,
/// and how far the moved text travels. `None` when there is no line that
/// way, so the command writes nothing.
///
/// The empty line a file ending in a newline has after its last one is not
/// a line to swap with: moving text past it would take the file's trailing
/// newline off, which is not this command's to change.
pub fn swap_lines_plan(
    text: &str,
    line_starts: &[usize],
    lines: Range<usize>,
    up: bool,
) -> Option<((Range<usize>, String), isize)> {
    let end_of = |l: usize| -> usize { line_starts.get(l + 1).copied().unwrap_or(text.len()) };
    let last = line_starts.len() - 1;
    let trailing_blank = line_starts[last] == text.len();
    if trailing_blank && lines.end - 1 == last {
        return None;
    }
    let block = line_starts[lines.start]..end_of(lines.end - 1);
    if up {
        if lines.start == 0 {
            return None;
        }
        let above = line_starts[lines.start - 1]..block.start;
        let moved = &text[block.clone()];
        let other = &text[above.clone()];
        // Only the last line of the file lacks a terminator, and after the
        // swap it is a different line that is last.
        let new = match moved.strip_suffix('\n') {
            Some(_) => format!("{moved}{other}"),
            None => format!("{moved}\n{}", &other[..other.len() - 1]),
        };
        Some(((above.start..block.end, new), -(above.len() as isize)))
    } else {
        if lines.end > last || (trailing_blank && lines.end == last) {
            return None;
        }
        let below = block.end..end_of(lines.end);
        let moved = &text[block.clone()];
        let other = &text[below.clone()];
        let (new, shift) = match other.strip_suffix('\n') {
            Some(_) => (format!("{other}{moved}"), other.len()),
            None => (
                format!("{other}\n{}", &moved[..moved.len() - 1]),
                other.len() + 1,
            ),
        };
        Some(((block.start..below.end, new), shift as isize))
    }
}

/// The link shape a command writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkSyntax {
    /// `editor:insert-link`: `[text](url)`, the caret in the url.
    Markdown,
    /// `editor:insert-wikilink`: `[[text]]`, the caret on the target.
    Wikilink,
    /// `editor:insert-embed`: `![[text]]`.
    Embed,
}

/// The text one of the link commands writes around `inner`, and where in
/// it the caret lands: an empty selection leaves the caret where the
/// target is typed, so the autocompletion popup opens on it.
pub fn link_insertion(inner: &str, kind: LinkSyntax) -> (String, usize) {
    match kind {
        LinkSyntax::Markdown => {
            let caret = if inner.is_empty() { 1 } else { inner.len() + 3 };
            (format!("[{inner}]()"), caret)
        }
        LinkSyntax::Wikilink => (format!("[[{inner}]]"), inner.len() + 2),
        LinkSyntax::Embed => (format!("![[{inner}]]"), inner.len() + 3),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn starts(text: &str) -> Vec<usize> {
        let mut v = vec![0];
        v.extend(text.match_indices('\n').map(|(i, _)| i + 1));
        v
    }

    fn apply(text: &str, mut edits: Vec<(Range<usize>, String)>) -> String {
        let mut out = text.to_string();
        edits.sort_by_key(|(r, _)| std::cmp::Reverse(r.start));
        for (r, new) in edits {
            out.replace_range(r, &new);
        }
        out
    }

    #[test]
    fn wraps_and_unwraps_a_selection() {
        let text = "alpha beta";
        assert_eq!(toggle_wrap(text, 0..5, "**", 0), Toggle::Wrap(0..5));
        let text = "**alpha** beta";
        // The markers inside the selection.
        assert_eq!(toggle_wrap(text, 0..9, "**", 0), Toggle::Strip(0..2, 7..9));
        // The markers just outside it.
        assert_eq!(toggle_wrap(text, 2..7, "**", 0), Toggle::Strip(0..2, 7..9));
    }

    #[test]
    fn a_caret_between_markers_strips_them() {
        assert_eq!(
            toggle_wrap("****", 2..2, "**", 0),
            Toggle::Strip(0..2, 2..4)
        );
        assert_eq!(toggle_wrap("ab", 1..1, "**", 0), Toggle::Wrap(1..1));
    }

    #[test]
    fn italics_inside_bold_does_not_eat_a_star() {
        let text = "**bold**";
        assert_eq!(toggle_wrap(text, 2..6, "*", 0), Toggle::Wrap(2..6));
    }

    #[test]
    fn an_outward_strip_stops_at_the_previous_selection() {
        let text = "*a*b*";
        assert_eq!(toggle_wrap(text, 3..4, "*", 3), Toggle::Wrap(3..4));
    }

    /// Mod+L on one line: the probe's table (Specification.md, "Observed
    /// editing command behavior").
    #[test]
    fn mod_l_adds_a_checkbox_or_flips_the_one_there() {
        let one = |line: &str| {
            let text = format!("{line}\n");
            let s = starts(&text);
            let out = apply(&text, checklist_plan(&text, &s, 0..1));
            out.trim_end_matches('\n').to_string()
        };
        assert_eq!(one("plain text"), "- [ ] plain text");
        assert_eq!(one("- bullet"), "- [ ] bullet");
        assert_eq!(one("1. ordered"), "1. [ ] ordered");
        assert_eq!(one("> quoted"), "> - [ ] quoted");
        assert_eq!(one("# heading"), "- [ ] # heading");
        assert_eq!(one("    indented"), "    - [ ] indented");
        assert_eq!(one(""), "- [ ] ");
        assert_eq!(one("- [ ] open"), "- [x] open");
        assert_eq!(one("- [x] done"), "- [ ] done");
        assert_eq!(one("- [?] q"), "- [ ] q");
        assert_eq!(one("- [-] dash"), "- [ ] dash");
        assert_eq!(one("* star"), "* [ ] star");
    }

    /// Mod+L over a selection decides once, blank lines ignored.
    #[test]
    fn mod_l_over_a_selection_decides_for_the_whole_of_it() {
        let all = |text: &str| {
            let s = starts(text);
            apply(text, checklist_plan(text, &s, 0..s.len() - 1))
        };
        // Every line has a box: check all unless all are checked.
        assert_eq!(all("- [ ] a\n- [x] b\n"), "- [x] a\n- [x] b\n");
        assert_eq!(all("- [x] a\n- [x] b\n"), "- [ ] a\n- [ ] b\n");
        assert_eq!(all("- [ ] a\n\n- [ ] b\n"), "- [x] a\n\n- [x] b\n");
        // No line has one: add one to all.
        assert_eq!(all("- a\n- b\n"), "- [ ] a\n- [ ] b\n");
        assert_eq!(all("a\nb\n"), "- [ ] a\n- [ ] b\n");
        // Some do: invert each line's.
        assert_eq!(all("- [ ] a\n- b\n"), "- a\n- [ ] b\n");
    }

    /// The cycle on one line, and the normalising first press over a
    /// selection that holds a line that is not a list item.
    #[test]
    fn the_cycle_goes_bullet_box_checked_bullet() {
        let cycle = |line: &str, presses: usize| {
            let mut text = format!("{line}\n");
            for _ in 0..presses {
                let s = starts(&text);
                text = apply(&text, cycle_checklist_plan(&text, &s, 0..1));
            }
            text.trim_end_matches('\n').to_string()
        };
        for (start, first, second, third) in [
            (
                "plain text",
                "- plain text",
                "- [ ] plain text",
                "- [x] plain text",
            ),
            ("- bullet", "- [ ] bullet", "- [x] bullet", "- bullet"),
            ("- [ ] open", "- [x] open", "- open", "- [ ] open"),
            ("- [x] done", "- done", "- [ ] done", "- [x] done"),
            ("- [?] q", "- q", "- [ ] q", "- [x] q"),
            ("> quoted", "> - quoted", "> - [ ] quoted", "> - [x] quoted"),
            ("", "- ", "- [ ] ", "- [x] "),
            ("* star", "* [ ] star", "* [x] star", "* star"),
            (
                "1. ordered",
                "1. [ ] ordered",
                "1. [x] ordered",
                "1. ordered",
            ),
            ("\t- two", "\t- [ ] two", "\t- [x] two", "\t- two"),
        ] {
            assert_eq!(cycle(start, 1), first, "{start:?} once");
            assert_eq!(cycle(start, 2), second, "{start:?} twice");
            assert_eq!(cycle(start, 3), third, "{start:?} three times");
        }
        let all = |text: &str| {
            let s = starts(text);
            apply(text, cycle_checklist_plan(text, &s, 0..s.len() - 1))
        };
        assert_eq!(all("a\nb\n"), "- a\n- b\n");
        assert_eq!(all("- a\n- b\n"), "- [ ] a\n- [ ] b\n");
        assert_eq!(all("a\n- b\n"), "- a\n- b\n");
        assert_eq!(all("a\n- [ ] b\n"), "- a\n- b\n");
        assert_eq!(all("a\n- b\n- [ ] c\n"), "- a\n- b\n- c\n");
        // Silica's converging rule where Obsidian oscillates: each list
        // line advances on its own.
        assert_eq!(all("- a\n- [ ] b\n"), "- [ ] a\n- [x] b\n");
        assert_eq!(all("- a\n\n- [x] b\n"), "- [ ] a\n\n- b\n");
    }

    /// The six insert-block commands, on a blank line and a non-blank one.
    #[test]
    fn the_insert_block_commands_write_their_observed_shapes() {
        let run = |text: &str, caret: usize, kind: BlockInsert| {
            let s = starts(text);
            let (edits, sel) = block_insert_plan(text, &s, caret, kind);
            (apply(text, edits), sel)
        };
        // Callout.
        let (out, sel) = run("", 0, BlockInsert::Callout);
        assert_eq!(out, "\n> [!NOTE] Title\n> Contents\n");
        assert_eq!(&out[sel.clone()], "NOTE");
        assert_eq!(sel, 5..9);
        let (out, sel) = run("alpha beta\n", 3, BlockInsert::Callout);
        assert_eq!(out, "> [!NOTE]\n> alpha beta\n");
        assert_eq!(sel, 4..8);
        // Code block and math block.
        let (out, sel) = run("\n", 0, BlockInsert::CodeBlock);
        assert_eq!(out, "```\n\n```\n");
        assert_eq!(sel, 4..4);
        let (out, sel) = run("hello world\n", 5, BlockInsert::CodeBlock);
        assert_eq!(out, "```\nhello world\n```\n");
        assert_eq!(sel, 9..9);
        let (out, sel) = run("x\n", 1, BlockInsert::MathBlock);
        assert_eq!(out, "$$\nx\n$$\n");
        assert_eq!(sel, 4..4);
        // Horizontal rule.
        let (out, sel) = run("", 0, BlockInsert::HorizontalRule);
        assert_eq!(out, "\n---\n");
        assert_eq!(sel, 5..5);
        let (out, sel) = run("hello world", 11, BlockInsert::HorizontalRule);
        assert_eq!(out, "hello world\n\n---\n");
        assert_eq!(sel, 17..17);
        let (out, _) = run("hello world", 5, BlockInsert::HorizontalRule);
        assert_eq!(out, "hello\n\n---\n world");
        // Tag.
        let (out, sel) = run("ab", 1, BlockInsert::Tag);
        assert_eq!(out, "a#b");
        assert_eq!(sel, 2..2);
        // Footnote: max plus one, the definition at the very end.
        let (out, sel) = run("body text", 9, BlockInsert::Footnote);
        assert_eq!(out, "body text[^1]\n\n[^1]: \n");
        assert_eq!(sel, 13..13);
        let (out, _) = run("a[^2] b\n\n[^2]: two\n", 7, BlockInsert::Footnote);
        assert_eq!(out, "a[^2] b[^3]\n\n[^2]: two\n\n[^3]: \n");
        let (out, _) = run("a[^note] b\n\n[^note]: named\n", 10, BlockInsert::Footnote);
        assert!(out.starts_with("a[^note] b[^1]\n"));
        let (out, _) = run("# Head\npara one\npara two\n", 11, BlockInsert::Footnote);
        assert_eq!(out, "# Head\npara[^1] one\npara two\n\n[^1]: \n");
    }

    /// `editor:clear-formatting`: the probe's table.
    #[test]
    fn clear_formatting_strips_markers_and_prefixes_but_not_links() {
        let clear = |text: &str, sel: Range<usize>| {
            let s = starts(text);
            apply(text, clear_formatting_plan(text, &s, sel))
        };
        let whole = |text: &str| clear(text, 0..text.len());
        assert_eq!(whole("**bold _italic_ text**"), "bold italic text");
        assert_eq!(
            clear("**bold _italic_ text**", 7..16),
            "**bold italic text**"
        );
        assert_eq!(whole("**==both==** tail"), "both tail");
        assert_eq!(whole("~~gone~~ and `code`"), "gone and code");
        assert_eq!(whole("## Heading text"), "Heading text");
        assert_eq!(whole("> quoted text"), "quoted text");
        assert_eq!(whole("- **bold** item"), "bold item");
        assert_eq!(whole("$x^2$ #tag"), "x^2 tag");
        assert_eq!(whole("**one**\n_two_\n==three=="), "one\ntwo\nthree");
        assert_eq!(whole("see [[Other|alias]] end"), "see [[Other|alias]] end");
        assert_eq!(whole("nothing to clear"), "nothing to clear");
        assert_eq!(whole("==\u{1f534}red== `x`"), "red x");
        // A bare caret changes nothing.
        assert_eq!(clear("**bold**", 3..3), "**bold**");
    }

    /// The highlight colours: a colour wraps with its emoji, the same
    /// colour again strips the whole highlight, another replaces the
    /// emoji, and a colour over a plain highlight inserts one.
    #[test]
    fn highlight_colours_write_an_emoji_after_the_opening_pair() {
        use HighlightColor::{Blue, Red};
        let red = Some(Red);
        assert_eq!(
            toggle_wrap_colored("word", 0..4, "==", red, 0),
            Toggle::Wrap(0..4)
        );
        let text = "==\u{1f534}word==";
        // The same colour, inner word or the whole span: strip.
        assert_eq!(
            toggle_wrap_colored(text, 6..10, "==", red, 0),
            Toggle::Strip(0..6, 10..12)
        );
        assert_eq!(
            toggle_wrap_colored(text, 0..12, "==", red, 0),
            Toggle::Strip(0..6, 10..12)
        );
        // A selection that starts on the emoji is the inner text too.
        assert_eq!(
            toggle_wrap_colored(text, 2..10, "==", red, 0),
            Toggle::Strip(0..6, 10..12)
        );
        // Another colour replaces the emoji in place.
        assert_eq!(
            toggle_wrap_colored(text, 6..10, "==", Some(Blue), 0),
            Toggle::Recolor(2..6, Blue.emoji())
        );
        // A colour over a plain highlight inserts the emoji.
        assert_eq!(
            toggle_wrap_colored("==word==", 2..6, "==", red, 0),
            Toggle::Recolor(2..2, Red.emoji())
        );
        assert_eq!(
            toggle_wrap_colored("==word==", 0..8, "==", red, 0),
            Toggle::Recolor(2..2, Red.emoji())
        );
        // The plain toggle takes a coloured highlight off whole.
        assert_eq!(
            toggle_wrap_colored(text, 6..10, "==", None, 0),
            Toggle::Strip(0..6, 10..12)
        );
    }

    #[test]
    fn heading_replaces_and_removes() {
        let text = "alpha\n## beta\n> quoted\n";
        let s = starts(text);
        assert_eq!(
            apply(text, heading_plan(text, &s, 0..1, 2)),
            "## alpha\n## beta\n> quoted\n"
        );
        assert_eq!(
            apply(text, heading_plan(text, &s, 1..2, 4)),
            "alpha\n#### beta\n> quoted\n"
        );
        assert_eq!(
            apply(text, heading_plan(text, &s, 1..2, 0)),
            "alpha\nbeta\n> quoted\n"
        );
        assert_eq!(
            apply(text, heading_plan(text, &s, 2..3, 1)),
            "alpha\n## beta\n> # quoted\n"
        );
    }

    #[test]
    fn heading_leaves_a_line_that_is_already_that_level() {
        let text = "### a\n";
        let s = starts(text);
        assert!(heading_plan(text, &s, 0..1, 3).is_empty());
    }

    #[test]
    fn bullet_toggles_over_a_selection() {
        let text = "one\ntwo\n";
        let s = starts(text);
        let on = apply(text, line_prefix_plan(text, &s, 0..2, LinePrefix::Bullet));
        assert_eq!(on, "- one\n- two\n");
        let s2 = starts(&on);
        assert_eq!(
            apply(&on, line_prefix_plan(&on, &s2, 0..2, LinePrefix::Bullet)),
            "one\ntwo\n"
        );
    }

    #[test]
    fn a_mixed_selection_gains_the_prefix_everywhere() {
        let text = "- one\ntwo\n";
        let s = starts(text);
        assert_eq!(
            apply(text, line_prefix_plan(text, &s, 0..2, LinePrefix::Bullet)),
            "- one\n- two\n"
        );
    }

    #[test]
    fn numbered_list_numbers_from_one_and_replaces_bullets() {
        let text = "- one\ntwo\n";
        let s = starts(text);
        assert_eq!(
            apply(text, line_prefix_plan(text, &s, 0..2, LinePrefix::Ordered)),
            "1. one\n2. two\n"
        );
    }

    #[test]
    fn blockquote_toggles_one_level() {
        let text = "a\nb\n";
        let s = starts(text);
        let on = apply(text, line_prefix_plan(text, &s, 0..2, LinePrefix::Quote));
        assert_eq!(on, "> a\n> b\n");
        let s2 = starts(&on);
        assert_eq!(
            apply(&on, line_prefix_plan(&on, &s2, 0..2, LinePrefix::Quote)),
            "a\nb\n"
        );
    }

    #[test]
    fn a_blank_line_in_a_multi_line_selection_is_left_alone() {
        let text = "a\n\nb\n";
        let s = starts(text);
        assert_eq!(
            apply(text, line_prefix_plan(text, &s, 0..3, LinePrefix::Bullet)),
            "- a\n\n- b\n"
        );
    }

    #[test]
    fn paragraph_spans_to_the_blank_lines() {
        let text = "one\n\ntwo\nthree\n\nfour\n";
        let s = starts(text);
        let span = paragraph_span(text, &s, 2..3);
        assert_eq!(&text[span.clone()], "two\nthree\n");
    }

    #[test]
    fn the_last_paragraph_takes_the_newline_before_it() {
        let text = "one\n\ntwo\n";
        let s = starts(text);
        let span = paragraph_span(text, &s, 2..3);
        assert_eq!(&text[span.clone()], "\ntwo\n");
    }

    #[test]
    fn swapping_moves_one_line_each_way() {
        let text = "a\nb\nc\n";
        let s = starts(text);
        let ((range, new), shift) =
            swap_lines_plan(text, &s, 1..2, true).expect("has a line above");
        assert_eq!(apply(text, vec![(range, new)]), "b\na\nc\n");
        assert_eq!(shift, -2);
        let ((range, new), shift) =
            swap_lines_plan(text, &s, 0..1, false).expect("has a line below");
        assert_eq!(apply(text, vec![(range, new)]), "b\na\nc\n");
        assert_eq!(shift, 2);
    }

    #[test]
    fn swapping_with_an_unterminated_last_line_keeps_the_terminator_last() {
        let text = "a\nb";
        let s = starts(text);
        let ((range, new), shift) = swap_lines_plan(text, &s, 0..1, false).expect("has a line");
        assert_eq!(apply(text, vec![(range, new)]), "b\na");
        assert_eq!(shift, 2);
        let ((range, new), shift) = swap_lines_plan(text, &s, 1..2, true).expect("has a line");
        assert_eq!(apply(text, vec![(range, new)]), "b\na");
        assert_eq!(shift, -2);
    }

    #[test]
    fn swapping_at_the_edges_writes_nothing() {
        let text = "a\nb\n";
        let s = starts(text);
        assert!(swap_lines_plan(text, &s, 0..1, true).is_none());
        assert!(swap_lines_plan(text, &s, 1..2, false).is_none());
    }

    #[test]
    fn link_insertions_put_the_caret_where_the_target_is_typed() {
        assert_eq!(link_insertion("", LinkSyntax::Markdown), ("[]()".into(), 1));
        assert_eq!(
            link_insertion("text", LinkSyntax::Markdown),
            ("[text]()".into(), 7)
        );
        assert_eq!(link_insertion("", LinkSyntax::Wikilink), ("[[]]".into(), 2));
        assert_eq!(
            link_insertion("Note", LinkSyntax::Embed),
            ("![[Note]]".into(), 7)
        );
    }
}
