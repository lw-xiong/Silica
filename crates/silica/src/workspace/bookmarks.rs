//! Bookmarks: the pane, its menu, adding and jumping to bookmarks, and
//! the clipboard export — the methods of the split-out bookmarks section,
//! unchanged apart from living in their own module. Split from
//! `workspace.rs` per the audit's module-size finding.

use super::*;

use crate::search::{menu, menu_item};

impl Workspace {
    /// Opens "Add bookmark" for a draft: the Title field, empty, with the
    /// name the bookmark gets without one as its placeholder.
    pub(crate) fn start_bookmark(
        &mut self,
        draft: BookmarkDraft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let placeholder = draft.placeholder();
        let (title, detail) = match &draft {
            BookmarkDraft::Tabs(paths) => (
                format!("Add {} bookmarks", paths.len()),
                Some("Title".to_string()),
            ),
            BookmarkDraft::Graph => ("Add bookmark".to_string(), Some("Title".to_string())),
            BookmarkDraft::Search(query) => {
                ("Add bookmark".to_string(), Some(format!("Query: {query}")))
            }
            BookmarkDraft::File { .. } | BookmarkDraft::Folder(_) => (
                "Add bookmark".to_string(),
                Some(format!("Path: {placeholder}")),
            ),
        };
        let initial = match &draft {
            BookmarkDraft::Graph => placeholder.clone(),
            _ => String::new(),
        };
        self.open_prompt(
            PendingOp::AddBookmark(draft),
            PromptSpec {
                title: title.into(),
                detail: detail.map(Into::into),
                input: Some(initial),
                placeholder: placeholder.into(),
                confirm: "Save".into(),
                ..PromptSpec::default()
            },
            window,
            cx,
        );
    }

    /// `bookmarks:bookmark-current-view` and the pane's first button: the
    /// open note, or the graph while it is up. A note already bookmarked
    /// opens "Edit bookmark", where Remove is the only change offered.
    pub(crate) fn start_bookmark_current_view(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.mode == ViewMode::Graph {
            self.start_bookmark(BookmarkDraft::Graph, window, cx);
            return;
        }
        let Some(path) = self.current.clone() else {
            self.report("No file is open", cx);
            return;
        };
        self.start_bookmark_path(&path, window, cx);
    }

    /// "Bookmark..." for a file or folder, from the explorer's menu or the
    /// open note.
    pub(crate) fn start_bookmark_path(
        &mut self,
        path: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.state.clone() else {
            return;
        };
        if state.vault().absolute(path).is_dir() {
            self.start_bookmark(BookmarkDraft::Folder(path.to_string()), window, cx);
            return;
        }
        if state.bookmarks().has_file(path) {
            let placeholder = Bookmark::file(path, None, None, 0).display_title();
            self.open_prompt(
                PendingOp::EditBookmark(path.to_string()),
                PromptSpec {
                    title: "Edit bookmark".into(),
                    detail: Some(format!("Path: {placeholder}").into()),
                    input: Some(String::new()),
                    placeholder: placeholder.into(),
                    confirm: "Save".into(),
                    secondary: Some("Remove".into()),
                    ..PromptSpec::default()
                },
                window,
                cx,
            );
            return;
        }
        self.start_bookmark(
            BookmarkDraft::File {
                path: path.to_string(),
                subpath: None,
            },
            window,
            cx,
        );
    }

    /// "New group": an empty group with the name typed.
    pub(crate) fn start_new_bookmark_group(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.start_bookmark(BookmarkDraft::Tabs(Vec::new()), window, cx);
    }

    /// The prompt answered: the item for the draft, with the typed title if
    /// any, appended and written.
    pub(crate) fn finish_bookmark(
        &mut self,
        draft: BookmarkDraft,
        title: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let title = (!title.trim().is_empty()).then(|| title.trim().to_string());
        let ctime = now_millis();
        let item = match draft {
            BookmarkDraft::File { path, subpath } => {
                Bookmark::file(&path, subpath.as_deref(), title.as_deref(), ctime)
            }
            BookmarkDraft::Folder(path) => Bookmark::folder(&path, title.as_deref(), ctime),
            BookmarkDraft::Search(query) => Bookmark::search(&query, title.as_deref(), ctime),
            BookmarkDraft::Graph => {
                let config = state.vault().graph_config().unwrap_or_default();
                Bookmark::graph(
                    title.as_deref().unwrap_or("Untitled Graph"),
                    crate::graph::full_document(&config),
                    ctime,
                )
            }
            BookmarkDraft::Tabs(paths) => {
                // * Every member shares one time, the group's is its own, as
                //   "Bookmark all tabs" writes them.
                let members: Vec<Bookmark> = paths
                    .iter()
                    .map(|p| Bookmark::file(p, None, None, ctime))
                    .collect();
                let name = title.unwrap_or_else(|| BookmarkDraft::Tabs(Vec::new()).placeholder());
                Bookmark::group(Some(&name), members, now_millis())
            }
        };
        self.change_bookmarks(cx, move |b| b.push(item));
    }

    /// Applies a change to the bookmarks, writes the file and redraws the
    /// pane; a write that fails leaves both as they were and says so.
    pub(crate) fn change_bookmarks(
        &mut self,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut Bookmarks),
    ) {
        let Some(state) = self.state.clone() else {
            return;
        };
        if let Err(err) = state.update_bookmarks(change) {
            self.report(format!("bookmarks.json could not be written: {err}"), cx);
            return;
        }
        self.refresh_bookmarks_pane(cx);
    }

    pub(crate) fn refresh_bookmarks_pane(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let items = state.bookmarks().items.clone();
        let current = self.current.clone();
        self.bookmarks.update(cx, |b, cx| {
            b.set_items(items, cx);
            b.set_active(current, cx);
        });
    }

    /// Renames, rewriting links right away or asking first when
    /// `alwaysUpdateLinks` is off and there are links to rewrite.
    pub(crate) fn rename_with_links(
        &mut self,
        from: &str,
        to: &str,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.state.clone() else {
            return;
        };
        if from == to {
            self.pending = None;
            if let Some(w) = window {
                self.close_modal(w, cx);
            }
            return;
        }
        if state.update_links_automatically() {
            self.finish_rename(from, to, true, window, cx);
            return;
        }
        let (files, links) = state.rename_preview(from, to);
        if links == 0 {
            self.finish_rename(from, to, false, window, cx);
            return;
        }
        let name = from.rsplit('/').next().unwrap_or(from);
        if let Some(w) = window {
            self.open_prompt(
                PendingOp::UpdateLinks {
                    from: from.to_string(),
                    to: to.to_string(),
                },
                PromptSpec {
                    title: "Update links?".into(),
                    detail: Some(
                        format!(
                            "{links} link{} in {files} file{} point to {name}. Update them to the new name?",
                            if links == 1 { "" } else { "s" },
                            if files == 1 { "" } else { "s" }
                        )
                        .into(),
                    ),
                    input: None,
                    confirm: "Update links".into(),
                    secondary: Some("Don't update".into()),
                    // * Obsidian's third answer: from here on, every rename
                    //   rewrites without asking, which is the one control
                    //   that writes `alwaysUpdateLinks: true`.
                    tertiary: Some("Always update".into()),
                    ..PromptSpec::default()
                },
                w,
                cx,
            );
        } else {
            // * No window available (inline title rename): auto-update links.
            self.finish_rename(from, to, true, None, cx);
        }
    }

    pub(crate) fn finish_rename(
        &mut self,
        from: &str,
        to: &str,
        update_links: bool,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.state.clone() else {
            return;
        };
        // * The open note's edits must be on disk before its file moves or
        //   its links are rewritten.
        self.flush_save(cx);
        match state.rename(from, to, update_links) {
            Ok(outcome) => {
                self.pending = None;
                if let Some(w) = window {
                    self.close_modal(w, cx);
                }
                self.apply_rename(outcome, cx);
            }
            Err(err) => self.prompt.update(cx, |p, cx| p.set_error(err, cx)),
        }
    }

    /// Follows the open note and the history to their new paths, then
    /// refreshes everything the way a filesystem event would.
    pub(crate) fn apply_rename(&mut self, outcome: RenameOutcome, cx: &mut Context<Self>) {
        let renamed: HashMap<&str, &str> = outcome
            .renamed
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect();
        // * A note's fold record follows it to its new path.
        if let Some(root) = self.state.as_ref().map(|s| s.root.clone()) {
            if let Some(store) = self.fold_store(&root) {
                for (from, to) in &renamed {
                    store.rename(from, to);
                }
            }
        }
        for entry in &mut self.history {
            if let Some(new) = renamed.get(entry.path.as_str()) {
                entry.path = new.to_string();
            }
        }
        for tab in self.stashed_tabs_mut() {
            for entry in &mut tab.history {
                if let Some(new) = renamed.get(entry.path.as_str()) {
                    entry.path = new.to_string();
                }
            }
            if let Some(path) = tab.path.as_mut() {
                if let Some(new) = renamed.get(path.as_str()) {
                    *path = new.to_string();
                }
            }
        }
        // * Every group's views follow the rename, the active one's included.
        for group in &self.groups {
            group.editor.update(cx, |e, _| {
                if let Some(new) = renamed.get(e.name.as_ref()) {
                    e.set_name(new.to_string());
                }
            });
            group.reading.update(cx, |r, _| {
                if let Some(new) = r.path.as_deref().and_then(|p| renamed.get(p)) {
                    r.path = Some(new.to_string());
                }
            });
        }
        if let Some(current) = self.current.clone() {
            if let Some(new) = renamed.get(current.as_str()) {
                self.current = Some(new.to_string());
            }
        }
        // * Obsidian says nothing about the rename itself. It shows one
        //   Notice, and only when links were rewritten:
        //   [[Specification#A move that changes the file's basename]] records
        //   it as exactly `Updated 1 link in 1 file.`
        let links = (outcome.rewritten_links > 0).then(|| {
            format!(
                "Updated {} link{} in {} file{}.",
                outcome.rewritten_links,
                plural(outcome.rewritten_links),
                outcome.rewritten_files,
                plural(outcome.rewritten_files),
            )
        });
        // * A note that changed underneath keeps its own bytes. Obsidian has
        //   nothing to say here because it never skips, and silence would
        //   leave stale links behind with no sign of it, so this one stays.
        let skipped = match outcome.stale.len() {
            0 => None,
            n => Some(format!(
                "{n} file{} changed on disk and {} left alone",
                plural(n),
                if n == 1 { "was" } else { "were" }
            )),
        };
        self.after_changes(outcome.changes, cx);
        if let Some(current) = self.current.clone() {
            self.explorer.update(cx, |e, cx| e.follow(&current, cx));
        }
        let notice = match (links, skipped) {
            (None, None) => None,
            (Some(l), None) => Some(l),
            (None, Some(s)) => Some(s),
            (Some(l), Some(s)) => Some(format!("{l} {s}")),
        };
        if let Some(notice) = notice {
            self.report(notice, cx);
        }
    }

    /// Renames the open note to `new_name` (same folder, same extension)
    /// right away; `--rename` on the command line, for smoke tests.
    pub fn rename_current(&mut self, new_name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(from) = self.current.clone() else {
            return;
        };
        let folder = from.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        let name = from.rsplit('/').next().unwrap_or(&from);
        let ext = name.rsplit_once('.').map(|(_, e)| e).unwrap_or("md");
        let to = if folder.is_empty() {
            format!("{new_name}.{ext}")
        } else {
            format!("{folder}/{new_name}.{ext}")
        };
        self.finish_rename(&from, &to, true, Some(window), cx);
    }

    /// Opens one of the file prompts for the open note (`--prompt` on the
    /// command line, for screenshots): `rename`, `move`, `delete`, `folder`.
    pub fn open_file_prompt(&mut self, kind: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(current) = self.current.clone() else {
            return;
        };
        match kind {
            "move" => self.start_move(&current, window, cx),
            "delete" => self.start_delete(&current, window, cx),
            "folder" => self.start_new_folder("", window, cx),
            _ => self.start_rename(&current, window, cx),
        }
    }

    /// Which view the main area paints for the mode and the open file. A
    /// view only gets the main area when it has this file to show: an
    /// editor with no file behind it — one left over from a note that is
    /// gone, or one never given a note, as on launch over a vault with
    /// nothing to open — takes edits nobody saves, and a reading view left
    /// behind keeps a deleted note's links on screen and clickable.
    pub(crate) fn main_view(&self, cx: &App) -> MainView {
        let has_note = self.reading.read(cx).path.is_some();
        let image = self
            .current
            .as_deref()
            .is_some_and(|p| FileKind::from_path(p) == FileKind::Image);
        let has_buffer = !self.editor.read(cx).name.is_empty();
        let canvas =
            self.current.is_some() && self.canvas.read(cx).path() == self.current.as_deref();
        let bases = self.current.is_some() && self.bases.read(cx).path() == self.current.as_deref();
        let pdf = self.current.is_some() && self.pdf.read(cx).path() == self.current.as_deref();
        Self::main_view_for(self.mode, has_note, image, has_buffer, canvas, bases, pdf)
    }

    /// The rule itself, over what the views have to show: `has_note` is the
    /// reading view's built note, `image` whether the open file is one,
    /// `has_buffer` whether the editor's buffer names a file, `canvas`
    /// whether the canvas view holds the open file, `bases` whether the
    /// Bases view does and `pdf` whether the PDF view does. None of a
    /// canvas, a base or a PDF has a reading or a source mode, so any of
    /// them is painted in every mode but the graph's.
    pub(crate) fn main_view_for(
        mode: ViewMode,
        has_note: bool,
        image: bool,
        has_buffer: bool,
        canvas: bool,
        bases: bool,
        pdf: bool,
    ) -> MainView {
        match mode {
            ViewMode::Graph => MainView::Graph,
            _ if canvas => MainView::Canvas,
            _ if bases => MainView::Bases,
            _ if pdf => MainView::Pdf,
            ViewMode::Reading if has_note => MainView::Reading,
            // * An image has no source to edit, so it reads the same in the
            //   editing modes as it does in the reading view.
            ViewMode::LivePreview | ViewMode::Source if image && has_note => MainView::Reading,
            ViewMode::LivePreview | ViewMode::Source if has_buffer => MainView::Editor,
            _ => MainView::Empty,
        }
    }

    /// What one group's rendering reads: the live fields for the active
    /// group, the stashed ones for the others.
    pub(crate) fn group_view(&self, ix: usize) -> GroupView {
        let g = &self.groups[ix];
        if ix == self.active_group {
            return GroupView {
                id: g.id,
                active: true,
                tabs: self.tab_snapshots(),
                active_tab: self.active_tab,
                stacked: self.stacked,
                current: self.current.clone(),
                mode: self.mode,
                history_len: self.history.len(),
                history_ix: self.history_ix,
                detached: self.detached.clone(),
                editor: self.editor.clone(),
                reading: self.reading.clone(),
                canvas: self.canvas.clone(),
                bases: self.bases.clone(),
                pdf: self.pdf.clone(),
            };
        }
        let tab = g.tabs.get(g.active_tab);
        GroupView {
            id: g.id,
            active: false,
            tabs: g.tabs.clone(),
            active_tab: g.active_tab,
            current: tab.and_then(|t| t.path.clone()),
            mode: tab.map(|t| t.mode).unwrap_or(ViewMode::Reading),
            history_len: tab.map(|t| t.history.len()).unwrap_or(0),
            history_ix: tab.map(|t| t.history_ix).unwrap_or(0),
            detached: g.detached.clone(),
            editor: g.editor.clone(),
            reading: g.reading.clone(),
            canvas: g.canvas.clone(),
            bases: g.bases.clone(),
            pdf: g.pdf.clone(),
            stacked: g.stacked,
        }
    }

    /// The main area: the layout tree drawn as nested rows and columns of
    /// groups, a draggable divider between each pair of children.
    pub(crate) fn render_layout(
        &self,
        node: &Layout,
        path: &mut Vec<usize>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        match node {
            Layout::Group(id) => match self.group_ix(*id) {
                Some(ix) => self.render_group(ix, theme, cx),
                None => div().into_any_element(),
            },
            Layout::Split {
                direction,
                children,
                ..
            } => {
                let row = *direction == SplitDirection::Vertical;
                // * A split whose children do not all carry a `dimension`
                //   shares its room equally, as Obsidian draws one that was
                //   never resized.
                let dims: Option<Vec<f64>> =
                    children.iter().map(|c| self.dimension_of(c)).collect();
                let split_path = path.clone();
                let drag_path = split_path.clone();
                let mut container = div()
                    .id(SharedString::from(format!("split-{split_path:?}")))
                    .flex()
                    .size_full()
                    .min_w_0()
                    .min_h_0()
                    .overflow_hidden()
                    .when(row, |d| d.flex_row())
                    .when(!row, |d| d.flex_col())
                    .on_drag_move(cx.listener(
                        move |this, e: &DragMoveEvent<DividerDrag>, _, cx| {
                            let drag = e.drag(cx).clone();
                            if drag.path == drag_path {
                                this.drag_divider(&drag, e.bounds, e.event.position, cx);
                            }
                        },
                    ));
                for (ix, child) in children.iter().enumerate() {
                    path.push(ix);
                    let body = self.render_layout(child, path, theme, cx);
                    path.pop();
                    let grow = dims.as_ref().map(|d| d[ix] as f32).unwrap_or(1.0);
                    let mut cell = div()
                        .relative()
                        .min_w_0()
                        .min_h_0()
                        .overflow_hidden()
                        .flex_basis(px(0.))
                        .child(body);
                    cell.style().flex_grow = Some(grow.max(0.01));
                    if ix > 0 {
                        let drag = DividerDrag {
                            path: split_path.clone(),
                            before: ix - 1,
                        };
                        let handle = div()
                            .id(("divider", ix as u64))
                            .absolute()
                            .when(row, |d| {
                                d.left(px(0.))
                                    .top_0()
                                    .h_full()
                                    .w(px(6.))
                                    .cursor_col_resize()
                            })
                            .when(!row, |d| {
                                d.top(px(0.))
                                    .left_0()
                                    .w_full()
                                    .h(px(6.))
                                    .cursor_row_resize()
                            })
                            .occlude()
                            .on_drag(drag, |_, _, _, cx| cx.new(|_| NoDragPreview));
                        cell = cell
                            .when(row, |d| d.border_l_1())
                            .when(!row, |d| d.border_t_1())
                            .border_color(theme.border)
                            .child(handle);
                    }
                    container = container.child(cell);
                }
                container.into_any_element()
            }
        }
    }

    /// One group: its tab strip, view header and the view itself. A mouse
    /// down anywhere in a group that is not the active one makes it so
    /// before the view underneath handles the click.
    pub(crate) fn render_group(
        &self,
        ix: usize,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let view = self.group_view(ix);
        let id = view.id;
        let mut order = Vec::new();
        self.layout_tree.groups(&mut order);
        // * The corner groups: top left is the first group however the
        //   splits run, top right needs the tree's shape.
        let first = order.first() == Some(&id);
        let last = self.layout_tree.top_right() == Some(id);
        div()
            .id(("group", id))
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .when(!view.active, |d| {
                d.capture_any_mouse_down(
                    cx.listener(move |this, _, _, cx| this.activate_group(id, cx)),
                )
            })
            .child(self.render_tab_strip(&view, first, last, theme, cx))
            .when(self.show_view_header(), |d| {
                d.child(self.render_header(&view, theme, cx))
            })
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(self.render_group_main(&view, theme, cx))
                    .when(self.dragging_tab, |d| {
                        d.child(self.render_drop_zones(id, theme, cx))
                    }),
            )
            .into_any_element()
    }

    /// The drop target over a group's content while a tab is dragged: it
    /// follows the pointer to light the zone the tab would land in — the
    /// whole area for the strip, the near half for a split — and takes the
    /// drop. It covers the view, so nothing under it reacts to the drag.
    pub(crate) fn render_drop_zones(
        &self,
        id: u64,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let zone = self.tab_drop.filter(|(g, _)| *g == id).map(|(_, z)| z);
        let wash = theme.accent.opacity(0.15);
        let edge = theme.accent.opacity(0.6);
        div()
            .id(("drop-zones", id))
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .bottom_0()
            .occlude()
            .on_drag_move(cx.listener(move |this, e: &DragMoveEvent<TabDrag>, _, cx| {
                this.tab_drag_moved(id, e.bounds, e.event.position, cx)
            }))
            .on_drop(cx.listener(move |this, drag: &TabDrag, window, cx| {
                this.drop_tab_on_group(drag, id, window, cx)
            }))
            .when_some(zone, |d, zone| {
                let half = relative(0.5);
                d.child(
                    div()
                        .absolute()
                        .bg(wash)
                        .border_2()
                        .border_color(edge)
                        .rounded(px(6.))
                        .map(|d| match zone {
                            DropZone::Center => d.top_0().left_0().right_0().bottom_0(),
                            DropZone::Left => d.top_0().left_0().bottom_0().w(half),
                            DropZone::Right => d.top_0().right_0().bottom_0().w(half),
                            DropZone::Top => d.top_0().left_0().right_0().h(half),
                            DropZone::Bottom => d.bottom_0().left_0().right_0().h(half),
                        }),
                )
            })
    }

    /// The view header: back and forward, the note's folder and name, the
    /// graph toggle and the mode toggle. The buttons act on their own
    /// group, which they make active first.
    pub(crate) fn render_header(
        &self,
        view: &GroupView,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let id = view.id;
        // * The leaf's name, the same string its tab carries: "New tab" with
        //   nothing open, not "No file is open".
        let title = leaf_title(view.current.as_deref(), view.mode);
        let crumbs: SharedString = view
            .current
            .as_deref()
            .and_then(|p| p.rsplit_once('/').map(|(dir, _)| dir.replace('/', " / ")))
            .unwrap_or_default()
            .into();
        // * Obsidian's view action shows the current mode: a book while
        //   reading, a pencil while editing.
        let mode_icon = match view.mode {
            ViewMode::Reading => "book-open",
            _ => "pencil",
        };
        let can_back = view.history_ix > 1;
        let can_forward = view.history_ix < view.history_len;
        let title_color = if view.active {
            theme.text
        } else {
            theme.text_muted
        };
        // * Obsidian's view header: 40 px, 12 px side padding, the history
        //   arrows at 40 percent while they have nowhere to go, the title in
        //   the small UI size, the actions as icons at the right.
        div()
            .flex()
            .flex_row()
            .items_center()
            .h(px(HEADER_HEIGHT))
            .px(px(12.))
            .gap(px(8.))
            .text_size(px(FONT_UI_SMALL))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .child(
                        icon_button("back", "arrow-left", theme)
                            .when(!can_back, |d| d.opacity(0.4))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.activate_group(id, cx);
                                this.go_back(&GoBack, window, cx)
                            })),
                    )
                    .child(
                        icon_button("forward", "arrow-right", theme)
                            .when(!can_forward, |d| d.opacity(0.4))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.activate_group(id, cx);
                                this.go_forward(&GoForward, window, cx)
                            })),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_row()
                    .gap_1()
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .when(!crumbs.is_empty(), |d| {
                        d.child(div().text_color(theme.text_muted).child(crumbs.clone()))
                            .child(div().text_color(theme.text_muted).child("/"))
                    })
                    .child(self.render_title(view, title.clone(), title_color, theme, cx)),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(2.))
                    .relative()
                    .child(
                        icon_button("graph", "git-fork", theme)
                            .when(view.mode == ViewMode::Graph, |d| d.bg(theme.bg_active))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.activate_group(id, cx);
                                this.open_graph_action(&OpenGraph, window, cx)
                            })),
                    )
                    .when(header_shows_mode_switch(&view.current, view.mode), |d| {
                        d.child(icon_button("mode", mode_icon, theme).on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.activate_group(id, cx);
                                this.toggle_mode(&ToggleMode, window, cx)
                            },
                        )))
                        // * Obsidian's view-header bookmark action:
                        // `lucide-bookmark`, and it bookmarks the note on
                        // screen through the same prompt the palette's
                        // bookmark command opens (spec: the mark action in
                        // both file menus, and the PDF pane's header, which
                        // carries Bookmark and More options only).
                        .when(view.current.is_some(), |d| {
                            d.child(icon_button("bookmark", "bookmark", theme).on_click(
                                cx.listener(move |this, _, window, cx| {
                                    this.activate_group(id, cx);
                                    if let Some(path) = this.current.clone() {
                                        this.start_bookmark(
                                            BookmarkDraft::File {
                                                path,
                                                subpath: None,
                                            },
                                            window,
                                            cx,
                                        );
                                    }
                                }),
                            ))
                        })
                    })
                    // * The ellipsis: "More options" — pin tab, copy path,
                    //   close. The menu opens under the button and closes
                    //   again on a pick or on the ellipsis itself.
                    .child(
                        icon_button("more-options", "ellipsis-vertical", theme)
                            .tooltip(crate::canvas::menu::tooltip("More options"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.toggle_header_more_options(cx);
                            })),
                    )
                    .when(self.header_more_options, |d| {
                        d.child(
                            div()
                                .id(("header-more-options", id))
                                .absolute()
                                .top_0()
                                .right_0()
                                .size_0()
                                .child(self.render_header_menu(view, theme, cx)),
                        )
                    }),
            )
    }

    /// "More options" under the header's ellipsis: pin the active tab of
    /// the group, copy its path, close the tab. A pick runs its command
    /// and closes the menu again.
    fn render_header_menu(
        &self,
        view: &GroupView,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let pinned = view.tabs.get(view.active_tab).is_some_and(|tab| tab.pinned);
        let pin_label = if pinned { "Unpin tab" } else { "Pin tab" };

        let mut list = menu("header-more-options-menu", theme);
        for (ix, (label, command, icon_name)) in [
            (pin_label, "silica:pin-tab", "pin"),
            ("Copy path", "workspace:copy-path", "copy"),
            ("Close", "workspace:close", "x"),
        ]
        .into_iter()
        .enumerate()
        {
            list = list.child(
                menu_item(
                    "header-more-options-item",
                    ix,
                    Some(icon_name),
                    label,
                    false,
                    theme,
                )
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.toggle_header_more_options(cx);
                    this.run_command(command, window, cx);
                })),
            );
        }
        list.into_any_element()
    }

    /// The view header's title, Obsidian's `.view-header-title`: a field
    /// that is editable where it stands, so a single click starts an edit
    /// (see [[Specification#The view-header title is an inline rename
    /// field]]).
    ///
    /// The text is painted as a shaped line rather than put in a div, for
    /// two reasons: a click has to be measured against the glyphs to land
    /// the caret where the pointer was, and an open edit draws its caret
    /// and selection in the same place. A leaf with no file is not
    /// renamed, so "New tab" is drawn and clicks on it do nothing.
    pub(crate) fn render_title(
        &self,
        view: &GroupView,
        title: SharedString,
        color: Hsla,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let id = view.id;
        let editing = self.title_edit.as_ref().is_some_and(|e| e.group == id);
        let read_entity = cx.entity();
        let paint_entity = cx.entity();
        let read_focus = self.title_focus.clone();
        let paint_focus = self.title_focus.clone();
        let mut selection_color = theme.accent;
        selection_color.a = if theme.dark { 0.35 } else { 0.25 };
        let caret_color = theme.text;
        let body = canvas(
            move |bounds, window, cx| {
                let style = window.text_style();
                let font_size = style.font_size.to_pixels(window.rem_size());
                let line_height = window.line_height();
                let ws = read_entity.read(cx);
                let edit = ws.title_edit.as_ref().filter(|e| e.group == id);
                let text: SharedString = match edit {
                    Some(edit) => edit.field.text().to_string().into(),
                    None => title.clone(),
                };
                let cursor = edit.map(|e| e.field.cursor()).unwrap_or(0);
                let selection = edit.map(|e| e.field.selection()).filter(|s| !s.is_empty());
                let mut run = style.to_run(text.len());
                run.color = color;
                let line = window
                    .text_system()
                    .shape_line(text.clone(), font_size, &[run], None);
                let cursor_x = if text.is_empty() {
                    px(0.)
                } else {
                    line.x_for_index(cursor)
                };
                let selection_x =
                    selection.map(|s| line.x_for_index(s.start)..line.x_for_index(s.end));
                // * A title too long for the header scrolls to keep the
                //   caret in view, the way the crate's other field does.
                let width = bounds.size.width;
                let mut scroll_x = ws
                    .title_layout
                    .get(&id)
                    .map(|l| l.scroll_x)
                    .unwrap_or(px(0.));
                if cursor_x - scroll_x > width - px(2.) {
                    scroll_x = cursor_x - width + px(2.);
                }
                if cursor_x - scroll_x < px(0.) {
                    scroll_x = cursor_x;
                }
                if line.width < width {
                    scroll_x = px(0.);
                }
                TitlePrepaint {
                    line,
                    cursor_x,
                    selection_x,
                    scroll_x,
                    line_height,
                    editing: edit.is_some(),
                    caret: edit.is_some() && read_focus.is_focused(window),
                }
            },
            move |bounds, prepaint: TitlePrepaint, window, cx| {
                // * The platform's input method is pointed at the field only
                //   while one is open: the title is not a tab stop, and
                //   nothing types into it until a click puts a caret there.
                if prepaint.editing {
                    window.handle_input(
                        &paint_focus,
                        ElementInputHandler::new(bounds, paint_entity.clone()),
                        cx,
                    );
                }
                let y = bounds.origin.y + (bounds.size.height - prepaint.line_height) / 2.;
                let origin = point(bounds.origin.x - prepaint.scroll_x, y);
                window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
                    if let Some(selection) = &prepaint.selection_x {
                        window.paint_quad(fill(
                            Bounds::from_corners(
                                point(origin.x + selection.start, y),
                                point(origin.x + selection.end, y + prepaint.line_height),
                            ),
                            selection_color,
                        ));
                    }
                    let _ = prepaint.line.paint(
                        origin,
                        prepaint.line_height,
                        TextAlign::Left,
                        None,
                        window,
                        cx,
                    );
                    if prepaint.caret {
                        window.paint_quad(fill(
                            Bounds::new(
                                point(origin.x + prepaint.cursor_x, y + px(2.)),
                                size(px(1.5), prepaint.line_height - px(4.)),
                            ),
                            caret_color,
                        ));
                    }
                });
                paint_entity.update(cx, |ws, _| {
                    ws.title_layout.insert(
                        id,
                        TitleLayout {
                            bounds,
                            line: prepaint.line,
                            line_height: prepaint.line_height,
                            scroll_x: prepaint.scroll_x,
                        },
                    );
                });
            },
        )
        .size_full();
        let mut host = div()
            .id(("view-title", id))
            .flex_1()
            .min_w_0()
            // * The canvas has no size of its own, so the row's height is
            //   given here; the line is centred in it when it paints.
            .h(px(HEADER_HEIGHT))
            .flex()
            .items_center()
            .overflow_hidden()
            .text_size(px(FONT_UI_SMALL))
            .text_color(color)
            .child(body);
        if let Some(path) = view.current.clone() {
            host = host.cursor_text().on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.title_clicked(id, path.clone(), event, window, cx)
                }),
            );
        }
        if !editing {
            return host.into_any_element();
        }
        // * The keys only come here while a field is open, and they are the
        //   crate's own field keys: the same context, so the same bindings.
        host.track_focus(&self.title_focus)
            .key_context("TextInput")
            .on_action(cx.listener(|this, _: &text_input::Left, _, cx| {
                this.with_title_field(cx, |f| f.step(false, false))
            }))
            .on_action(cx.listener(|this, _: &text_input::Right, _, cx| {
                this.with_title_field(cx, |f| f.step(true, false))
            }))
            .on_action(cx.listener(|this, _: &text_input::SelectLeft, _, cx| {
                this.with_title_field(cx, |f| f.step(false, true))
            }))
            .on_action(cx.listener(|this, _: &text_input::SelectRight, _, cx| {
                this.with_title_field(cx, |f| f.step(true, true))
            }))
            .on_action(cx.listener(|this, _: &text_input::WordLeft, _, cx| {
                this.with_title_field(cx, |f| f.step_word(false, false))
            }))
            .on_action(cx.listener(|this, _: &text_input::WordRight, _, cx| {
                this.with_title_field(cx, |f| f.step_word(true, false))
            }))
            .on_action(cx.listener(|this, _: &text_input::SelectWordLeft, _, cx| {
                this.with_title_field(cx, |f| f.step_word(false, true))
            }))
            .on_action(cx.listener(|this, _: &text_input::SelectWordRight, _, cx| {
                this.with_title_field(cx, |f| f.step_word(true, true))
            }))
            .on_action(cx.listener(|this, _: &text_input::Home, _, cx| {
                this.with_title_field(cx, |f| f.move_to_edge(false, false))
            }))
            .on_action(cx.listener(|this, _: &text_input::End, _, cx| {
                this.with_title_field(cx, |f| f.move_to_edge(true, false))
            }))
            .on_action(cx.listener(|this, _: &text_input::SelectHome, _, cx| {
                this.with_title_field(cx, |f| f.move_to_edge(false, true))
            }))
            .on_action(cx.listener(|this, _: &text_input::SelectEnd, _, cx| {
                this.with_title_field(cx, |f| f.move_to_edge(true, true))
            }))
            .on_action(cx.listener(|this, _: &text_input::SelectAll, _, cx| {
                this.with_title_field(cx, |f| f.select_all())
            }))
            .on_action(cx.listener(|this, _: &text_input::Backspace, _, cx| {
                this.with_title_field(cx, |f| f.backspace())
            }))
            .on_action(cx.listener(|this, _: &text_input::Delete, _, cx| {
                this.with_title_field(cx, |f| f.delete())
            }))
            .on_action(
                cx.listener(|this, _: &text_input::DeleteWordBackward, _, cx| {
                    this.with_title_field(cx, |f| f.delete_word_back())
                }),
            )
            .on_action(cx.listener(|this, _: &text_input::Clear, _, cx| {
                this.with_title_field(cx, |f| {
                    f.select_all();
                    f.insert("");
                })
            }))
            .on_action(cx.listener(|this, _: &text_input::Copy, _, cx| this.title_copy(false, cx)))
            .on_action(cx.listener(|this, _: &text_input::Cut, _, cx| this.title_copy(true, cx)))
            .on_action(cx.listener(|this, _: &text_input::Paste, _, cx| this.title_paste(cx)))
            .on_action(
                cx.listener(|this, _: &text_input::Submit, window, cx| {
                    this.commit_title(window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &text_input::Cancel, window, cx| {
                    this.cancel_title(window, cx)
                }),
            )
            // * Ctrl+E in a field is swallowed rather than toggling the mode
            //   behind it, as it is in every other field here.
            .on_action(cx.listener(|_, _: &text_input::Swallow, _, _| {}))
            .into_any_element()
    }

    /// A click on a view-header title: Obsidian's focuses the field and
    /// puts the caret at the offset the pointer landed on, and a double
    /// click takes the word it landed in -- no modal and no menu at any
    /// point. A click past the end of the text is a click on the header,
    /// not on the title, and changes nothing.
    pub(crate) fn title_clicked(
        &mut self,
        group: u64,
        path: String,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(layout) = self.title_layout.get(&group).cloned() else {
            return;
        };
        let x = event.position.x - layout.bounds.origin.x + layout.scroll_x;
        if x > layout.line.width + px(4.) {
            return;
        }
        // * A click in another group's header makes that group active first,
        //   so the edit always belongs to the leaf the keyboard is in.
        self.activate_group(group, cx);
        // * The field holds the name the header shows, which is the name
        //   without its extension -- the same stem the rename prompt's box
        //   holds, so both commit the same way.
        let name = leaf_title(Some(&path), ViewMode::Reading).to_string();
        let mut edit = match self.title_edit.take() {
            Some(edit) if edit.group == group && edit.path == path => edit,
            _ => TitleEdit {
                group,
                path,
                field: TitleField::new(name),
                marked: None,
            },
        };
        let mut at = layout
            .line
            .closest_index_for_x(x)
            .min(edit.field.text().len());
        while !edit.field.text().is_char_boundary(at) {
            at -= 1;
        }
        edit.field
            .click(at, event.click_count, event.modifiers.shift);
        self.title_edit = Some(edit);
        // * The field's focus handle only joins the dispatch tree with the
        //   frame that paints it, and the click that opens one lands before
        //   that frame: a field that was not there yet takes the keyboard on
        //   the next frame instead.
        let focus = self.title_focus.clone();
        window.focus(&focus, cx);
        window.on_next_frame(move |window, cx| window.focus(&focus, cx));
        cx.notify();
    }

    /// Applies an editing key to the open title field.
    pub(crate) fn with_title_field(
        &mut self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut TitleField),
    ) {
        if let Some(edit) = self.title_edit.as_mut() {
            f(&mut edit.field);
            edit.marked = None;
            cx.notify();
        }
    }

    pub(crate) fn title_copy(&mut self, cut: bool, cx: &mut Context<Self>) {
        let Some(edit) = self.title_edit.as_mut() else {
            return;
        };
        let range = edit.field.selection();
        if range.is_empty() {
            return;
        }
        let text = edit.field.text()[range.clone()].to_string();
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        if cut {
            edit.field.replace(range, "");
            edit.marked = None;
            cx.notify();
        }
    }

    pub(crate) fn title_paste(&mut self, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        self.with_title_field(cx, |f| f.insert(&text));
    }

    /// Enter in the view-header title: the file takes the typed name. The
    /// field holds the stem, so this goes through the same path the rename
    /// prompt's box does -- one validator, one rename, one link rewrite.
    /// A name the filesystem would refuse leaves the field open and says
    /// why, rather than losing what was typed.
    pub(crate) fn commit_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = self.title_edit.take() else {
            return;
        };
        let value = edit.field.text().trim().to_string();
        let to = match self.renamed_path(&edit.path, &value) {
            Ok(to) => to,
            Err(err) => {
                self.title_edit = Some(edit);
                self.report(err, cx);
                return;
            }
        };
        // * The keyboard goes back to the view before the rename, which may
        //   put the "Update links?" prompt up and want it for itself.
        self.focus_main(window, cx);
        self.rename_with_links(&edit.path, &to, Some(window), cx);
        cx.notify();
    }

    /// Escape in the view-header title: the edit is abandoned and nothing
    /// is renamed, as Obsidian's does.
    pub(crate) fn cancel_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.title_edit.take().is_none() {
            return;
        }
        self.focus_main(window, cx);
        cx.notify();
    }

    /// The file group `id` shows, wherever that group's fields live.
    pub(crate) fn group_current(&self, id: u64) -> Option<&str> {
        let ix = self.group_ix(id)?;
        if ix == self.active_group {
            return self.current.as_deref();
        }
        let group = &self.groups[ix];
        group.tabs.get(group.active_tab)?.path.as_deref()
    }

    /// [`renamed_path`] for a vault path, which is asked whether it is a
    /// folder.
    pub(crate) fn renamed_path(&self, from: &str, value: &str) -> Result<String, String> {
        let is_folder = self
            .state
            .as_ref()
            .map(|s| s.vault().absolute(from).is_dir())
            .unwrap_or(false);
        renamed_path(from, value, is_folder)
    }

    /// Whether a rename may land on `to`: anything already at the
    /// destination refuses it, except a case-only rename of the same entry,
    /// which is what the vault's own rename allows. The pre-check exists so
    /// the inline field can keep its editor up on a colliding name instead
    /// of closing it and reporting after the fact.
    pub(crate) fn destination_free(&self, from: &str, to: &str) -> bool {
        let Some(state) = self.state.as_ref() else {
            return true;
        };
        let vault = state.vault();
        let to_abs = vault.absolute(to);
        if !to_abs.exists() {
            return true;
        }
        from.eq_ignore_ascii_case(to)
            && std::fs::canonicalize(vault.absolute(from)).ok()
                == std::fs::canonicalize(&to_abs).ok()
    }

    /// What a group's area paints, by the same rule as [`Workspace::main_view`]
    /// over that group's own views. There is one graph view; a second group
    /// showing the graph says where it is instead.
    pub(crate) fn render_group_main(
        &self,
        view: &GroupView,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let has_note = view.reading.read(cx).path.is_some();
        let image = view
            .current
            .as_deref()
            .is_some_and(|p| FileKind::from_path(p) == FileKind::Image);
        let has_buffer = !view.editor.read(cx).name.is_empty();
        let canvas =
            view.current.is_some() && view.canvas.read(cx).path() == view.current.as_deref();
        let bases = view.current.is_some() && view.bases.read(cx).path() == view.current.as_deref();
        let pdf = view.current.is_some() && view.pdf.read(cx).path() == view.current.as_deref();
        match Self::main_view_for(view.mode, has_note, image, has_buffer, canvas, bases, pdf) {
            MainView::Canvas => view.canvas.clone().into_any_element(),
            MainView::Bases => view.bases.clone().into_any_element(),
            MainView::Pdf => view.pdf.clone().into_any_element(),
            MainView::Graph if self.graph_group() == Some(view.id) => {
                self.graph.clone().into_any_element()
            }
            MainView::Graph => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme.text_muted)
                .child("The graph is open in another pane")
                .into_any_element(),
            MainView::Reading => view.reading.clone().into_any_element(),
            MainView::Editor => Self::render_editor(&view.editor, view.detached.clone(), theme),
            MainView::Empty => match view.current.as_deref() {
                Some(path) => Self::render_unsupported_file(path, theme),
                None => self.render_empty_tab(theme, cx),
            },
        }
    }

    /// The editor, under a line naming the file it lost if its buffer is no
    /// longer backed by one. The user has to be told; the alternative is
    /// typing into a buffer that is never written and never mentioned.
    pub(crate) fn render_editor(
        editor: &Entity<Editor>,
        detached: Option<SharedString>,
        theme: &Theme,
    ) -> gpui::AnyElement {
        let Some(name) = detached else {
            return editor.clone().into_any_element();
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(
                div()
                    .px_3()
                    .py_1()
                    .text_size(px(12.))
                    .bg(theme.bg_alt)
                    .text_color(theme.text_muted)
                    .border_b_1()
                    .border_color(theme.border)
                    .child(format!(
                        "{name} — this buffer no longer belongs to a file in the vault, \
                         and nothing saves it"
                    )),
            )
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .child(editor.clone().into_any_element()),
            )
            .into_any_element()
    }

    /// A file whose kind no view can draw: an error message, not a front
    /// door, so it gets the line on its own.
    pub(crate) fn render_unsupported_file(path: &str, theme: &Theme) -> gpui::AnyElement {
        let message: SharedString = format!(
            "{:?} files are not supported yet",
            FileKind::from_path(path)
        )
        .into();
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_5()
            .items_center()
            .justify_center()
            .text_color(theme.text_muted)
            .child(message)
            .into_any_element()
    }

    /// The empty tab: the four pills Obsidian offers there, each running
    /// the command it names, under Silica's mark -- an empty tab is the one
    /// blank canvas the window has, so it is where the mark goes. The body
    /// is the same however the tab came to be empty, a fresh vault and the
    /// last tab closing alike, because both leave a tab with no file.
    pub(crate) fn render_empty_tab(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let registry = cx.global::<Commands>().0.clone();
        let pills: Vec<_> =
            empty_tab_actions(&registry)
                .into_iter()
                .map(|action| {
                    let command = action.command;
                    // * Obsidian's `mod-pill`: fully rounded, the label in the
                    //   UI size with the hotkey behind it at muted strength.
                    div()
                        .id(action.id)
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.))
                        .px(px(14.))
                        .py(px(6.))
                        .rounded(px(14.))
                        .bg(theme.bg_alt)
                        .border_1()
                        .border_color(theme.border)
                        .cursor_pointer()
                        .hover(|s| s.bg(theme.bg_active))
                        .text_size(px(FONT_UI_SMALL))
                        .child(div().text_color(theme.text).child(action.label))
                        .children(
                            action
                                .hotkey
                                .map(|keys| div().text_color(theme.text_faint).child(keys)),
                        )
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.run_command(command, window, cx)
                        }))
                })
                .collect();
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_5()
            .items_center()
            .justify_center()
            .text_color(theme.text_muted)
            .child(logo(104.))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .justify_center()
                    .gap(px(8.))
                    .children(pills),
            )
            .into_any_element()
    }
}
