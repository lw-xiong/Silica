//! The Task inbox: every checkbox task in the vault, in one right-dock
//! pane.
//!
//! The rows are read from the metadata cache's own `listItems` index —
//! the same records the search view's `task:` scopes run on — so nothing
//! here parses a note a second time. A row's click opens the note it came
//! from with the caret on the task's own line; the checkbox beside the
//! text flips the task, rewriting **only that line** through the vault's
//! checked write path, so a toggle can never touch another byte of the
//! note.

use super::*;
/// One task as the inbox lists it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TaskRow {
    /// The note's vault-relative path.
    pub(crate) path: String,
    /// The zero-based source line of the task in the note.
    pub(crate) line: usize,
    /// The task's own text, without its checkbox marker.
    pub(crate) text: String,
    /// Whether the task is done (`[x]`) or pending (`[ ]`).
    pub(crate) done: bool,
}

/// The byte range a line covers, the line ending included: what a toggle
/// rewrites and what the caret's offset is measured against.
fn line_range(text: &str, line: usize) -> Range<usize> {
    let start = line_start(text, line);
    let rest = text.get(start..).unwrap_or_default();
    let len = rest.find('\n').map_or(rest.len(), |ix| ix + 1);
    start..start + len
}

/// The line as it is written, without its ending.
fn line_at(text: &str, line: usize) -> &str {
    text.get(line_range(text, line))
        .unwrap_or_default()
        .trim_end_matches(['\n', '\r'])
}

/// The list marker a line may start with — `-`, `*`, `+` or `1.`/`1)` —
/// and the whitespace after it, off the front of `body`.
fn strip_list_marker(body: &str) -> &str {
    let bytes = body.as_bytes();
    match bytes.first() {
        Some(b'-') | Some(b'*') | Some(b'+') => body[1..].trim_start(),
        _ => {
            let digits = bytes.iter().take_while(|b| b.is_ascii_digit()).count();
            if digits > 0 && matches!(bytes.get(digits), Some(b'.') | Some(b')')) {
                body[1 + digits..].trim_start()
            } else {
                body
            }
        }
    }
}

/// Where a task line's three checkbox bytes start in the line, and the
/// character between them. `None` for anything the note index would not
/// call a task either: prose, a link, a plain list item, or a bracket
/// with no whitespace after it.
fn task_marker(line: &str) -> Option<(usize, char)> {
    let body = strip_list_marker(line.trim_start());
    let at = line.len() - body.len();
    let bytes = body.as_bytes();
    if bytes.len() >= 3 && bytes[0] == b'[' && bytes[2] == b']' {
        // * A marker is one character, so a byte there that is not a
        //   character of its own is not one.
        if !body.is_char_boundary(1) {
            return None;
        }
        // * The index asks for whitespace after the bracket, so that
        //   `- [x]` on its own is text; a toggle writes the same way.
        if !matches!(bytes.get(3), Some(b' ') | Some(b'\t')) {
            return None;
        }
        Some((at, body[1..2].chars().next()?))
    } else {
        None
    }
}

/// A task's label is its own text, without the marker: the list item's
/// first line (a parent item's span covers its children), with the
/// checkbox and the whitespace after it taken off.
pub(crate) fn task_label(text: &str) -> String {
    let first = text.split(['\n', '\r']).next().unwrap_or_default();
    match task_marker(first) {
        Some((at, _)) => first[at + 3..].trim_start().to_string(),
        // * Not a task line: the item's own text still says something.
        None => strip_list_marker(first.trim_start()).to_string(),
    }
}

/// The note with `line`'s checkbox flipped and every other byte as it
/// was: the line keeps its indentation, its ending and everything after
/// the marker, and no line is added or removed. `None` when that line is
/// not a task line, or is past the end of the note.
pub(crate) fn toggle_task_line(text: &str, line: usize) -> Option<String> {
    let range = line_range(text, line);
    let body = line_at(text, line);
    let (at, mark) = task_marker(body)?;
    let flipped = if mark == ' ' { 'x' } else { ' ' };
    let mut out = String::with_capacity(text.len());
    out.push_str(&text[..range.start]);
    out.push_str(&body[..at]);
    out.push('[');
    out.push(flipped);
    out.push(']');
    out.push_str(&body[at + 3..]);
    out.push_str(&text[body.len() + range.start..range.end]);
    out.push_str(&text[range.end..]);
    Some(out)
}

/// The byte offset the line starts at: where the caret goes. Past the
/// last line it is the text's own length, which the editor clamps.
pub(crate) fn line_start(text: &str, line: usize) -> usize {
    if line == 0 {
        return 0;
    }
    let mut seen = 0;
    for (ix, _) in text.match_indices('\n') {
        seen += 1;
        if seen == line {
            return ix + 1;
        }
    }
    text.len()
}

/// Every task in the vault, read from the note index's own list items:
/// path-then-line order, one row per checkbox. Nothing here parses a note
/// a second time — the label comes from the text the index already holds.
pub(crate) fn collect_rows(state: &VaultState) -> Vec<TaskRow> {
    let cache = state.cache();
    let texts = state.texts();
    let mut rows = Vec::new();
    for (path, meta) in cache.files() {
        let Some(text) = texts.get(path) else {
            continue;
        };
        for item in &meta.list_items {
            let Some(mark) = item.task else {
                continue;
            };
            let span = text
                .get(item.position.start.offset..item.position.end.offset)
                .unwrap_or_default();
            rows.push(TaskRow {
                path: path.to_string(),
                line: item.position.start.line as usize,
                text: task_label(span),
                done: mark != ' ',
            });
        }
    }
    rows
}

/// The rows a pane draws: the completed ones dropped while they are
/// hidden, and only the rows whose note path or text holds the filter
/// query.
pub(crate) fn visible_rows(rows: &[TaskRow], show_done: bool, query: &str) -> Vec<TaskRow> {
    let query = query.trim().to_lowercase();
    rows.iter()
        .filter(|row| {
            (show_done || !row.done)
                && (query.is_empty()
                    || row.path.to_lowercase().contains(&query)
                    || row.text.to_lowercase().contains(&query))
        })
        .cloned()
        .collect()
}

/// A small checkbox, as a properties panel's rows carry.
fn check_box(checked: bool, theme: &Theme) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .flex_shrink_0()
        .size(px(14.))
        .rounded_sm()
        .border_1()
        .border_color(if checked { theme.accent } else { theme.border })
        .when(checked, |d| {
            d.bg(theme.accent).child(icon("check", 10., theme.bg))
        })
}

impl Workspace {
    /// Every task in the vault, straight off the note index.
    pub(crate) fn task_rows(&self) -> Vec<TaskRow> {
        self.state.as_deref().map(collect_rows).unwrap_or_default()
    }

    /// The rows the inbox draws: the completed ones hidden when they are
    /// hidden, the filter query applied.
    pub(crate) fn visible_task_rows(&self, cx: &App) -> Vec<TaskRow> {
        visible_rows(
            &self.task_rows(),
            self.task_state.show_done,
            self.task_filter.read(cx).text(),
        )
    }

    /// An inbox row's click: opens the note it came from with the caret on
    /// the task's own line, the way an Outline row reveals its heading.
    pub(crate) fn open_task(&mut self, row: &TaskRow, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        self.open_file(row.path.clone(), None, cx);
        let offset = state
            .texts()
            .get(&row.path)
            .map(|text| line_start(text, row.line))
            .unwrap_or(0);
        self.reading
            .update(cx, |r, cx| r.scroll_to_offset(offset, cx));
        if self.mode.is_editing() {
            self.editor
                .update(cx, |e, cx| e.reveal_offset_centered(offset, cx));
        } else {
            self.editor.update(cx, |e, cx| e.reveal_offset(offset, cx));
        }
        self.request_focus_main(cx);
    }

    /// The checkbox beside a row: flips that task's own line and writes the
    /// note back through the vault's checked write path, so no other byte
    /// of it moves. A note a buffer is holding with unsaved edits is left
    /// alone — that buffer's next save would throw the write away — and so
    /// is a row whose line no longer says what the index said it did.
    pub(crate) fn toggle_inbox_task(&mut self, row: &TaskRow, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let absolute = match state.vault().write_path(&row.path) {
            Ok(absolute) => absolute,
            Err(err) => {
                self.report(format!("{} not written: {err}", row.path), cx);
                return;
            }
        };
        let on_disk = match std::fs::read_to_string(&absolute) {
            Ok(text) => text,
            Err(err) => {
                self.report(format!("Could not read {}: {err}", row.path), cx);
                return;
            }
        };
        // * Any group's editor may be holding the note, not just the active
        //   one, and a buffer that differs from the disk is unsaved work.
        if self.buffer_differs(&row.path, &on_disk, cx) {
            self.report(
                format!("{} has unsaved edits; task not toggled", row.path),
                cx,
            );
            return;
        }
        // * The index may be a note behind the disk: a row whose own line
        //   no longer reads as it did is refused, rather than flipping
        //   whatever checkbox moved into its place.
        if task_label(line_at(&on_disk, row.line)) != row.text {
            self.report(
                format!("{} changed on disk; task not toggled", row.path),
                cx,
            );
            return;
        }
        let Some(written) = toggle_task_line(&on_disk, row.line) else {
            self.report(
                format!("Line {} of {} is not a task", row.line + 1, row.path),
                cx,
            );
            return;
        };
        if written == on_disk {
            return;
        }
        if let Err(err) = silica_vault::write_no_follow(&absolute, written.as_bytes()) {
            self.report(format!("Could not write {}: {err}", row.path), cx);
            return;
        }
        self.report(format!("Task toggled in {}", row.path), cx);
        // * The index is what the inbox reads, so the row comes back the
        //   same way an outside edit would bring it.
        self.files_changed(vec![absolute], cx);
    }

    /// Whether the completed tasks are listed, which the header's button
    /// flips.
    pub(crate) fn toggle_tasks_done(&mut self, cx: &mut Context<Self>) {
        self.task_state.show_done = !self.task_state.show_done;
        self.task_state.selected = None;
        cx.notify();
    }

    /// The Task inbox pane: the filter and completed-tasks buttons, the
    /// filter when shown, then every checkbox in the vault as a row — the
    /// box, the task's text, the note it came from. A row's click opens
    /// that note on the task's line; the box flips the task.
    pub(crate) fn render_tasks_pane(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let (show_search, show_done) = (self.task_state.show_search, self.task_state.show_done);
        let header = Self::pane_header()
            .child(
                Self::nav_button(
                    "tasks-search",
                    "search",
                    "Show search filter",
                    show_search,
                    theme,
                )
                .on_click(cx.listener(|this, _, window, cx| {
                    this.toggle_pane_search(RightTab::Tasks, window, cx)
                })),
            )
            .child(
                Self::nav_button(
                    "tasks-done",
                    "square-check",
                    if show_done {
                        "Hide completed tasks"
                    } else {
                        "Show completed tasks"
                    },
                    show_done,
                    theme,
                )
                .on_click(cx.listener(|this, _, _, cx| this.toggle_tasks_done(cx))),
            );
        let filter = show_search.then(|| self.render_pane_filter(RightTab::Tasks, theme, cx));
        let rows = self.visible_task_rows(cx);
        // * Obsidian's `.pane-empty`: 13 px faint, padding 24px 16px,
        //   margin 8px 0, centred.
        let body: gpui::AnyElement = if rows.is_empty() {
            div()
                .flex_1()
                .px(px(12.))
                .child(
                    div()
                        .my(px(8.))
                        .px(px(16.))
                        .py(px(24.))
                        .text_size(px(FONT_UI_SMALL))
                        .text_color(theme.text_faint)
                        .text_center()
                        .child(if show_done {
                            "No tasks found."
                        } else {
                            "No pending tasks. Show completed tasks to see the rest."
                        }),
                )
                .into_any_element()
        } else {
            let mut list = div()
                .id("tasks")
                .flex()
                .flex_col()
                .flex_1()
                .overflow_y_scroll()
                .px(px(12.))
                .pt(px(4.))
                .pb(px(32.));
            for (ix, row) in rows.into_iter().enumerate() {
                let open_row = row.clone();
                let toggle_row = row.clone();
                let note = SharedString::from(row.path.clone());
                list = list.child(
                    div()
                        .id(("task-row", ix))
                        .group("task-row")
                        .relative()
                        .w_full()
                        .flex_shrink_0()
                        .min_h(px(25.))
                        .mb(px(2.))
                        .pl(px(4.))
                        .pr(px(8.))
                        .py(px(4.))
                        .flex()
                        .flex_row()
                        .items_start()
                        .gap(px(6.))
                        .rounded(px(8.))
                        .cursor_pointer()
                        .when(self.task_state.selected == Some(ix), |d| d.bg(theme.bg_alt))
                        .hover(|s| s.bg(theme.bg_alt))
                        // * The box is its own click target: flipping a task
                        //   is not the same act as opening the note it lives
                        //   in, and a click on it must not do both.
                        .child(
                            div()
                                .id(("task-check", ix))
                                .flex_shrink_0()
                                .pt(px(1.))
                                .cursor_pointer()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.toggle_inbox_task(&toggle_row, cx);
                                }))
                                .child(check_box(row.done, theme)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .text_size(px(FONT_UI_SMALL))
                                .line_height(px(17.))
                                .text_color(if row.done {
                                    theme.text_faint
                                } else {
                                    theme.text
                                })
                                .group_hover("task-row", |s| s.text_color(theme.text))
                                .child(
                                    div()
                                        .whitespace_nowrap()
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .child(row.text.clone()),
                                )
                                .child(
                                    div()
                                        .whitespace_nowrap()
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .text_size(px(FONT_UI_SMALLER))
                                        .line_height(px(12.))
                                        .text_color(theme.text_faint)
                                        .child(note),
                                ),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_task(&open_row, cx);
                        })),
                );
            }
            list.into_any_element()
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(header)
            .children(filter)
            .child(body)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::VaultState;
    use std::fs;

    /// A task's label is its own text, without the marker.
    #[test]
    fn the_label_is_the_items_text_without_the_marker() {
        assert_eq!(task_label("- [ ] Buy milk"), "Buy milk");
        assert_eq!(task_label("- [x] Ship it"), "Ship it");
        assert_eq!(task_label("* [X] shouted"), "shouted");
        assert_eq!(task_label("  - [ ] indented"), "indented");
        assert_eq!(task_label("1. [ ] ordered"), "ordered");
        // * A parent's span covers its children; the label is its own
        //   first line, not the child's text.
        assert_eq!(task_label("- [ ] Parent\n  - [ ] Child"), "Parent");
        // * Not a marker: brackets that are not a checkbox stay put.
        assert_eq!(task_label("- see [[Other]]"), "see [[Other]]");
    }

    /// A toggle rewrites the line it was given and nothing else: every
    /// other line is byte-identical, and no line is added or removed.
    #[test]
    fn a_toggle_changes_only_the_line_it_was_given() {
        let text = "# Title\n\n- [ ] Buy milk\n- [x] Ship it\nProse with a [bracket] in it.\n";
        let toggled = toggle_task_line(text, 2).expect("line 2 holds a checkbox");
        let before: Vec<&str> = text.split_inclusive('\n').collect();
        let after: Vec<&str> = toggled.split_inclusive('\n').collect();
        assert_eq!(before.len(), after.len(), "no line was added or removed");
        for (ix, (old, new)) in before.iter().zip(after.iter()).enumerate() {
            if ix != 2 {
                assert_eq!(*old, *new, "line {ix} must be left alone");
            }
        }
        assert_eq!(after[2], "- [x] Buy milk\n");
        assert_eq!(
            toggled,
            "# Title\n\n- [x] Buy milk\n- [x] Ship it\nProse with a [bracket] in it.\n"
        );
    }

    /// A completed task toggles back to pending.
    #[test]
    fn a_completed_task_toggles_back_to_pending() {
        let text = "- [x] Ship it\n";
        assert_eq!(
            toggle_task_line(text, 0).as_deref(),
            Some("- [ ] Ship it\n")
        );
        let text = "- [X] shouted\n";
        assert_eq!(
            toggle_task_line(text, 0).as_deref(),
            Some("- [ ] shouted\n")
        );
    }

    /// A line with no checkbox writes nothing: prose, a link, a plain
    /// list item, a past-the-end line number.
    #[test]
    fn a_line_without_a_checkbox_is_not_rewritten() {
        assert_eq!(toggle_task_line("Plain prose.\n", 0), None);
        assert_eq!(toggle_task_line("- see [[Other]] here\n", 0), None);
        assert_eq!(toggle_task_line("- [link](https://example.com)\n", 0), None);
        assert_eq!(toggle_task_line("- plain item\n", 0), None);
        // * The index asks for whitespace after the bracket, so a bare
        //   `- [x]` is text there too and writes nothing here.
        assert_eq!(toggle_task_line("- [x]\n", 0), None);
        assert_eq!(toggle_task_line("- [ ] task\n", 5), None);
        assert_eq!(toggle_task_line("", 0), None);
    }

    /// CRLF notes keep their line endings: only the marker byte moves.
    #[test]
    fn crlf_line_endings_are_kept() {
        let text = "- [ ] one\r\n- [ ] two\r\n";
        let toggled = toggle_task_line(text, 1).expect("line 1 is a task");
        assert_eq!(toggled, "- [ ] one\r\n- [x] two\r\n");
    }

    /// The caret's line is the byte offset the line starts at.
    #[test]
    fn a_line_becomes_the_offset_it_starts_at() {
        let text = "# Heading\n\nSome prose.\n\n- [ ] First task\n";
        assert_eq!(line_start(text, 0), 0);
        assert_eq!(line_start(text, 4), 24);
        // * Past the end: the text's length, which the editor clamps.
        assert_eq!(line_start(text, 99), text.len());
    }

    /// The rows are read from the vault's own index, not from a second
    /// parse: path, line, label and state all come from the cache's
    /// list items, and a plain list item is not a task.
    #[test]
    fn the_rows_come_from_the_vaults_own_index() {
        let (_dir, state) = vault_with(&[
            ("Tasks.md", "# Tasks\n\n- [ ] Buy milk\n- [x] Ship it\n"),
            (
                "Notes/Nested.md",
                "- [ ] Parent\n  - [x] Child\n- plain item\n",
            ),
            ("Prose.md", "# Just a note\n\nWith a [bracket] in it.\n"),
        ]);
        let rows = collect_rows(&state);
        let found: Vec<(&str, usize, &str, bool)> = rows
            .iter()
            .map(|r| (r.path.as_str(), r.line, r.text.as_str(), r.done))
            .collect();
        assert_eq!(
            found,
            vec![
                ("Notes/Nested.md", 0, "Parent", false),
                ("Notes/Nested.md", 1, "Child", true),
                ("Tasks.md", 2, "Buy milk", false),
                ("Tasks.md", 3, "Ship it", true),
            ],
            "every checkbox in the vault, in path-then-line order"
        );
    }

    /// The line a row names is the line a toggle rewrites: flipping a row
    /// moves that note's own checkbox and no other byte of it.
    #[test]
    fn the_line_a_row_names_is_the_line_a_toggle_rewrites() {
        let (_dir, state) =
            vault_with(&[("Tasks.md", "# Tasks\n\n- [ ] Buy milk\n- [x] Ship it\n")]);
        let text = state
            .texts()
            .get("Tasks.md")
            .expect("the note is indexed")
            .as_str()
            .to_string();
        let rows = collect_rows(&state);
        let pending = rows.iter().find(|r| r.text == "Buy milk").expect("a row");
        let done = rows.iter().find(|r| r.text == "Ship it").expect("a row");
        assert!(!pending.done && done.done, "the row carries its own state");
        assert_eq!(
            toggle_task_line(&text, pending.line).as_deref(),
            Some("# Tasks\n\n- [x] Buy milk\n- [x] Ship it\n")
        );
        assert_eq!(
            toggle_task_line(&text, done.line).as_deref(),
            Some("# Tasks\n\n- [ ] Buy milk\n- [ ] Ship it\n")
        );
        // * A row whose line on disk no longer reads as it did is refused:
        //   the label it carries is what the write is checked against.
        assert_eq!(task_label(line_at(&text, done.line)), done.text);
        assert_ne!(task_label(line_at(&text, 0)), done.text);
    }

    /// A vault whose notes hold no tasks yields no rows, not an error.
    #[test]
    fn a_vault_without_tasks_yields_no_rows() {
        let (_dir, state) = vault_with(&[("A.md", "# A\n\nprose\n"), ("B.md", "- item\n")]);
        assert!(collect_rows(&state).is_empty());
    }

    /// The completed tasks go when they are hidden, and the filter query
    /// keeps the rows whose note or text holds it.
    #[test]
    fn completed_rows_go_when_hidden_and_the_filter_narrows_the_rest() {
        let (_dir, state) = vault_with(&[
            ("Tasks.md", "- [ ] Buy milk\n- [x] Ship it\n"),
            ("Notes/Other.md", "- [ ] Read [[Tasks]]\n"),
        ]);
        let rows = collect_rows(&state);
        assert_eq!(visible_rows(&rows, false, "").len(), 2);
        assert_eq!(visible_rows(&rows, true, "").len(), 3);
        let shown = visible_rows(&rows, true, "ship");
        assert_eq!(shown.len(), 1, "the query matches the task's text");
        assert_eq!(shown[0].text, "Ship it");
        let by_note = visible_rows(&rows, false, "tasks.md");
        assert_eq!(by_note.len(), 1, "the query matches the note's path");
        assert_eq!(by_note[0].path, "Tasks.md");
    }

    /// A temp vault holding `(relative path, content)` notes, written
    /// before the load so the index sees them.
    fn vault_with(notes: &[(&str, &str)]) -> (tempfile::TempDir, VaultState) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join(".obsidian")).unwrap();
        fs::write(
            root.join(".obsidian/app.json"),
            r#"{"trashOption":"local"}"#,
        )
        .unwrap();
        for (path, content) in notes {
            if let Some(parent) = root.join(path).parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(root.join(path), content).unwrap();
        }
        let state = VaultState::load(&root.to_path_buf()).unwrap();
        (dir, state)
    }
}
