//! Tab strip: opening and closing tabs, tab groups, split layout of
//! groups, and the drag-to-detach/drag-between-groups interaction — the
//! methods of the split-out tab sections, unchanged apart from living in
//! their own module. Split from `workspace.rs` per the audit's
//! module-size finding.

use super::*;

impl Workspace {
    /// Copies the active tab's live fields into its slot.
    pub(crate) fn stash_active_tab(&mut self) {
        let slot = self.tabs.get(self.active_tab);
        let leaf = slot.and_then(|t| t.leaf.clone());
        // * The view state belongs to the canvas the tab shows; a tab that
        //   moved on to another file leaves it behind.
        let view_state = slot
            .filter(|t| t.path == self.current)
            .and_then(|t| t.view_state.clone());
        let tab = Tab {
            path: self.current.clone(),
            mode: self.mode,
            history: self.history.clone(),
            history_ix: self.history_ix,
            pinned: slot.map(|t| t.pinned).unwrap_or(false),
            leaf,
            view_state,
        };
        match self.tabs.get_mut(self.active_tab) {
            Some(slot) => *slot = tab,
            None => {
                self.tabs.push(tab);
                self.active_tab = self.tabs.len() - 1;
            }
        }
    }

    /// Every tab as it stands, the active one from the live fields.
    pub(crate) fn tab_snapshots(&self) -> Vec<Tab> {
        let mut tabs = self.tabs.clone();
        if let Some(slot) = tabs.get_mut(self.active_tab) {
            slot.path = self.current.clone();
            slot.mode = self.mode;
            slot.history = self.history.clone();
            slot.history_ix = self.history_ix;
        }
        tabs
    }

    /// Puts `tabs[active_tab]` on screen: its note in its mode, or the
    /// empty state.
    pub(crate) fn load_tab(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active_tab).cloned() else {
            return;
        };
        self.history = tab.history;
        self.history_ix = tab.history_ix;
        if tab.mode == ViewMode::Graph && self.mode != ViewMode::Graph {
            self.mode_before_graph = self.mode;
        }
        self.mode = tab.mode;
        if tab.mode.is_editing() {
            self.editor.update(cx, |e, cx| {
                e.set_live_preview(tab.mode == ViewMode::LivePreview, cx)
            });
        }
        match (tab.path, self.state.clone()) {
            (Some(path), Some(state)) => {
                self.current = None;
                self.show_file(&path, &state, cx);
            }
            (None, Some(state)) => self.show_empty(&state, cx),
            (path, None) => self.current = path,
        }
        self.request_focus_main(cx);
        self.sync_reading(cx);
        cx.notify();
    }

    /// The empty tab: no note in the main area and nothing describing one.
    /// A buffer with unsaved edits nowhere else stays, marked as saved
    /// nowhere, the way a file that went away leaves it.
    pub(crate) fn show_empty(&mut self, state: &Arc<VaultState>, cx: &mut Context<Self>) {
        self.release_buffer(cx);
        self.release_canvas(cx);
        self.release_bases(cx);
        self.release_pdf(cx);
        self.reading.update(cx, |r, cx| {
            r.set_note(
                None,
                reading::build_with("", "", state.as_ref(), state.build_options()),
                cx,
            )
        });
        self.current = None;
        self.explorer.update(cx, |e, cx| e.clear_active(cx));
        self.graph.update(cx, |g, cx| g.set_current(None, cx));
        self.clear_panes(cx);
        self.status = SharedString::default();
    }

    /// `workspace:new-tab`: an empty tab to the right of the active one,
    /// in the mode "Default view for new tabs" names.
    /// `workspace:new-tab` and the strip's `+`: the new tab goes on the
    /// strip's right end, whichever tab was active, and becomes the active
    /// one (probed 2026-10-22, `tools/obsidian-probe/new-tab.mjs`).
    pub fn new_tab(&mut self, cx: &mut Context<Self>) {
        self.flush_save(cx);
        self.stash_active_tab();
        let mode = self.default_mode();
        self.tabs.push(Tab::empty(mode));
        self.active_tab = self.tabs.len() - 1;
        self.load_tab(cx);
    }

    /// Opens `path` in a new tab, leaving the active one as it is — unless
    /// `alwaysFocusNewTabs` is on, which is Obsidian's default and switches
    /// to the tab it just made.
    pub fn open_in_new_tab(
        &mut self,
        path: String,
        subpath: Option<Subpath<'_>>,
        cx: &mut Context<Self>,
    ) {
        let was = self.active_tab;
        self.new_tab(cx);
        self.open_file(path, subpath, cx);
        if !self.focus_new_tab() && was < self.tabs.len() - 1 {
            self.activate_tab(was, cx);
        }
    }

    pub fn activate_tab(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() || ix == self.active_tab {
            return;
        }
        self.flush_save(cx);
        self.stash_active_tab();
        self.active_tab = ix;
        self.load_tab(cx);
    }

    /// `workspace:next-tab` and `previous-tab`: `step` tabs along, wrapping.
    pub(crate) fn cycle_tab(&mut self, step: usize, cx: &mut Context<Self>) {
        let n = self.tabs.len();
        if n > 1 {
            self.activate_tab((self.active_tab + step) % n, cx);
        }
    }

    /// `workspace:toggle-stacked-tabs`: stack the group's tabs into one
    /// slot, so only the active tab shows in the strip, or unstack them.
    pub(crate) fn toggle_stacked_tabs(&mut self, cx: &mut Context<Self>) {
        self.stacked = !self.stacked;
        cx.notify();
    }

    /// The stacked-tabs badge: step to the next tab, which becomes the one
    /// the stack shows. A no-op unless the group is stacked and has more
    /// than one tab.
    pub(crate) fn cycle_stack(&mut self, cx: &mut Context<Self>) {
        if self.stacked && self.tabs.len() > 1 {
            self.activate_tab((self.active_tab + 1) % self.tabs.len(), cx);
        }
    }

    /// `workspace:close`: the last close leaves the empty tab, as Obsidian's
    /// does; the tab to the right takes over, or the one to the left at the
    /// end of the strip.
    pub fn close_tab(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.flush_save(cx);
        self.stash_active_tab();
        if ix >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(ix);
        // Evict the note's full text from the unbounded `texts` map so
        // long sessions with many opened/closed notes don't leak memory.
        // Search falls back to a disk read for evicted notes (see run_search).
        if let Some(ref path) = tab.path {
            if let Some(state) = &self.state {
                let _ = state
                    .texts
                    .write()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(path);
            }
        }
        self.remember_closed(ix, tab);
        if self.tabs.is_empty() {
            // * A group whose last tab closes goes with it, as Obsidian's
            //   does, unless it is the only one.
            if self.groups.len() > 1 {
                self.tabs.push(Tab::empty(self.default_mode()));
                self.active_tab = 0;
                let id = self.active_group_id();
                self.close_group(id, false, cx);
                return;
            }
            self.tabs.push(Tab::empty(self.default_mode()));
            self.active_tab = 0;
            self.load_tab(cx);
        } else if ix == self.active_tab {
            self.active_tab = ix.min(self.tabs.len() - 1);
            self.load_tab(cx);
        } else {
            if ix < self.active_tab {
                self.active_tab -= 1;
            }
            cx.notify();
        }
    }

    pub(crate) fn remember_closed(&mut self, ix: usize, tab: Tab) {
        // * An empty tab is not worth bringing back.
        if tab.path.is_some() || tab.mode == ViewMode::Graph {
            self.closed_tabs.push((ix, tab));
            if self.closed_tabs.len() > CLOSED_TABS_KEPT {
                self.closed_tabs.remove(0);
            }
        }
    }

    /// `workspace:close-others`: every tab but the active one.
    pub(crate) fn close_other_tabs(&mut self, cx: &mut Context<Self>) {
        self.stash_active_tab();
        let keep = self.tabs.remove(self.active_tab);
        let others = std::mem::take(&mut self.tabs);
        // Evict closed tabs' texts from the unbounded map.
        if let Some(state) = &self.state {
            let mut texts = state.texts.write().unwrap_or_else(|e| e.into_inner());
            for tab in others.iter() {
                if let Some(ref path) = tab.path {
                    texts.remove(path);
                }
            }
        }
        for (ix, tab) in others.into_iter().enumerate() {
            self.remember_closed(ix, tab);
        }
        self.tabs.push(keep);
        self.active_tab = 0;
        cx.notify();
    }

    /// `workspace:undo-close-pane`: the most recently closed tab, back
    /// where it was.
    pub(crate) fn undo_close_tab(&mut self, cx: &mut Context<Self>) {
        let Some((ix, tab)) = self.closed_tabs.pop() else {
            return;
        };
        self.flush_save(cx);
        self.stash_active_tab();
        let at = ix.min(self.tabs.len());
        self.tabs.insert(at, tab);
        self.active_tab = at;
        self.load_tab(cx);
    }

    // ----- tab groups --------------------------------------------------------------

    pub(crate) fn active_group_id(&self) -> u64 {
        self.groups[self.active_group].id
    }

    pub(crate) fn group_ix(&self, id: u64) -> Option<usize> {
        self.groups.iter().position(|g| g.id == id)
    }

    /// Copies the active group's live fields into its slot.
    pub(crate) fn stash_active_group(&mut self) {
        self.stash_active_tab();
        let ix = self.active_group;
        let group = &mut self.groups[ix];
        group.tabs = self.tabs.clone();
        group.active_tab = self.active_tab;
        group.stacked = self.stacked;
        group.mode_before_graph = self.mode_before_graph;
        group.detached = self.detached.clone();
        group.words = self.words;
        group.chars = self.chars;
    }

    /// Makes group `ix` the live one. Nothing is reloaded — its views still
    /// show its note — but everything that follows the active note (the
    /// explorer's highlight, the graph's centre, the right dock, the status
    /// bar) is pointed at it, and a note that went away while the group was
    /// off screen is let go of the way the watcher would have.
    pub(crate) fn load_group(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.active_group = ix;
        let group = &self.groups[ix];
        let id = group.id;
        self.tabs = group.tabs.clone();
        self.active_tab = group.active_tab.min(self.tabs.len().saturating_sub(1));
        self.stacked = group.stacked;
        self.mode_before_graph = group.mode_before_graph;
        self.detached = group.detached.clone();
        self.words = group.words;
        self.chars = group.chars;
        self.editor = group.editor.clone();
        self.reading = group.reading.clone();
        self.canvas = group.canvas.clone();
        self.bases = group.bases.clone();
        self.pdf = group.pdf.clone();
        let tab = self
            .tabs
            .get(self.active_tab)
            .cloned()
            .unwrap_or_else(|| Tab::empty(self.default_mode()));
        self.current = tab.path;
        self.mode = tab.mode;
        self.history = tab.history;
        self.history_ix = tab.history_ix;
        self.group_mru.retain(|g| *g != id);
        self.group_mru.insert(0, id);
        self.save_task = None;
        match (self.current.clone(), self.state.clone()) {
            (Some(path), Some(state)) => {
                if state.vault().absolute(&path).is_file() {
                    self.explorer.update(cx, |e, cx| e.follow(&path, cx));
                    self.graph
                        .update(cx, |g, cx| g.set_current(Some(path.clone()), cx));
                    self.refresh_panes(&state, cx);
                    self.status = path.into();
                } else {
                    self.detach_current(true, &state, cx);
                }
            }
            (None, Some(state)) => self.show_empty(&state, cx),
            (_, None) => {}
        }
        self.request_focus_main(cx);
        cx.notify();
    }

    /// Hands the keyboard and the live fields to group `id`: a click in its
    /// area, a focus command, or the group that takes over when one closes.
    pub fn activate_group(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(ix) = self.group_ix(id) else {
            return;
        };
        if ix == self.active_group {
            return;
        }
        self.flush_save(cx);
        self.stash_active_group();
        self.load_group(ix, cx);
    }

    /// A group of its own, with an editor and a reading view set up like the
    /// others', not yet in the layout.
    pub(crate) fn new_group(
        &mut self,
        mode: ViewMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> u64 {
        let editor = cx.new(|cx| Editor::new(window, cx));
        let reading = cx.new(ReadingView::new);
        let canvas = cx.new(CanvasView::new);
        let bases = cx.new(BasesView::new);
        let pdf = cx.new(PdfView::new);
        self._subscriptions.extend(view_subscriptions(
            &editor, &reading, &canvas, &bases, &pdf, cx,
        ));
        let id = self.next_group_id;
        self.next_group_id += 1;
        if let Some(state) = self.state.clone() {
            Self::give_editor_the_vault(&editor, &state, cx);
            Self::give_reading_the_vault(&reading, &state, cx);
        }
        self.groups
            .push(TabGroup::new(id, editor, reading, canvas, bases, pdf, mode));
        if let Some(state) = self.state.clone() {
            self.apply_app_config(&state, cx);
        }
        id
    }

    /// `workspace:split-vertical` ("Split right") and `split-horizontal`
    /// ("Split down"): a new group beside the active one, showing the same
    /// note in the same mode, and it takes the focus. A `dimension` the group
    /// had is halved between the two, or moves to the split that now wraps
    /// them (see [[Specification#Observed tab group behavior]]).
    pub(crate) fn split(
        &mut self,
        direction: SplitDirection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.split_beside(None, direction, window, cx);
    }

    /// The explorer menu's "Open to the right": a new group beside the active
    /// one showing `path`, which is [`Workspace::split`] with a file of its
    /// own. The group it leaves behind keeps whatever it was showing, which is
    /// the difference from a split.
    pub(crate) fn open_to_right(
        &mut self,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.split_beside(Some(path), SplitDirection::Vertical, window, cx);
    }

    /// What the two share. `None` duplicates the active tab -- the same note,
    /// mode, history and view state, which is what a split is; `Some(path)` is
    /// a fresh tab on that file, with no history and no view state to inherit,
    /// opened in the vault's default view mode the way any first open is.
    ///
    /// The order here matters and is the order [`Workspace::split`] had: the
    /// save is flushed and the group stashed *before* the tab is built, so the
    /// `view_state` it copies is the one the stash just wrote.
    pub(crate) fn split_beside(
        &mut self,
        path: Option<String>,
        direction: SplitDirection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.flush_save(cx);
        self.stash_active_group();
        let from = self.active_group_id();
        let (tab, mode_before_graph) = match path {
            None => (
                Tab {
                    path: self.current.clone(),
                    mode: self.mode,
                    history: self.history.clone(),
                    history_ix: self.history_ix,
                    pinned: false,
                    leaf: None,
                    view_state: self
                        .tabs
                        .get(self.active_tab)
                        .and_then(|t| t.view_state.clone()),
                },
                self.mode_before_graph,
            ),
            Some(path) => {
                let mode = self.default_mode();
                (
                    Tab {
                        path: Some(path),
                        mode,
                        history: Vec::new(),
                        history_ix: 0,
                        pinned: false,
                        leaf: None,
                        view_state: None,
                    },
                    mode,
                )
            }
        };
        let id = self.new_group(tab.mode, window, cx);
        self.place_beside(from, id, direction, true);
        let Some(ix) = self.group_ix(id) else {
            return;
        };
        self.groups[ix].tabs = vec![tab];
        self.groups[ix].active_tab = 0;
        self.groups[ix].mode_before_graph = mode_before_graph;
        self.load_group(ix, cx);
        // * The new group's views are empty: put the note in them.
        if let (Some(path), Some(state)) = (self.current.clone(), self.state.clone()) {
            if self.mode.is_editing() {
                let live = self.mode == ViewMode::LivePreview;
                self.editor.update(cx, |e, cx| e.set_live_preview(live, cx));
            }
            self.show_file(&path, &state, cx);
        }
    }

    /// Puts group `new` into the layout beside group `from`, after it or
    /// before it along `direction`, and settles the `dimension`: halved
    /// between the two when they became siblings, moved onto the split that
    /// now wraps them when they did not.
    pub(crate) fn place_beside(
        &mut self,
        from: u64,
        new: u64,
        direction: SplitDirection,
        after: bool,
    ) {
        let Some(from_ix) = self.group_ix(from) else {
            return;
        };
        let dimension = self.groups[from_ix].dimension();
        match self.layout_tree.insert_beside(from, new, direction, after) {
            Some(false) => {
                if let Some(d) = dimension {
                    self.groups[from_ix].set_dimension(Some(d / 2.0));
                    if let Some(new_ix) = self.group_ix(new) {
                        self.groups[new_ix].set_dimension(Some(d / 2.0));
                    }
                }
            }
            Some(true) => {
                self.groups[from_ix].set_dimension(None);
                if let Some(mut path) = self.layout_tree.path_to(from) {
                    path.pop();
                    if let Some(Layout::Split { node, .. }) = self.layout_tree.node_at_mut(&path) {
                        set_dimension(node, dimension);
                    }
                }
            }
            None => {}
        }
    }

    // ----- dragging tabs ---------------------------------------------------------

    /// A tab drag started: every group paints its drop zones from the next
    /// frame on. Called from the drag's own constructor, which runs with
    /// the app rather than this view, so it comes in through a weak handle.
    pub(crate) fn begin_tab_drag(&mut self, cx: &mut Context<Self>) {
        self.dragging_tab = true;
        self.tab_drop = None;
        self.drop_bar = None;
        self.tab_bounds.clear();
        cx.notify();
    }

    /// The dragged tab moved over (or off) group `group`'s strip: over it,
    /// the drop bar goes to the tab boundary nearest the pointer, the left
    /// edge of a tab or the right edge of the last one, sliding there from
    /// wherever it was. Off it, the bar goes if it was this strip's.
    pub(crate) fn strip_drag_moved(
        &mut self,
        group: u64,
        bounds: gpui::Bounds<gpui::Pixels>,
        position: gpui::Point<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) {
        if !bounds.contains(&position) {
            if self.drop_bar.is_some_and(|b| b.group == group) {
                self.drop_bar = None;
                cx.notify();
            }
            return;
        }
        let mut tabs: Vec<(usize, gpui::Bounds<gpui::Pixels>)> = self
            .tab_bounds
            .iter()
            .filter(|((g, _), _)| *g == group)
            .map(|((_, ix), b)| (*ix, *b))
            .collect();
        tabs.sort_by_key(|(ix, _)| *ix);
        let Some((last_ix, last)) = tabs.last().copied() else {
            return;
        };
        let pointer = f32::from(position.x);
        let edges = tabs
            .iter()
            .map(|(ix, b)| (*ix, f32::from(b.origin.x)))
            .chain(std::iter::once((last_ix + 1, f32::from(last.right()))));
        let Some((at, x)) =
            edges.min_by(|a, b| (a.1 - pointer).abs().total_cmp(&(b.1 - pointer).abs()))
        else {
            return;
        };
        let now = Instant::now();
        let strip_x = f32::from(bounds.origin.x);
        let next = match self.drop_bar {
            Some(bar) if bar.group == group && bar.at == at => {
                if bar.strip_x == strip_x {
                    return;
                }
                DropBar { strip_x, ..bar }
            }
            // * Within the strip the bar slides; arriving from elsewhere it
            //   appears where it is wanted.
            Some(bar) if bar.group == group => DropBar {
                group,
                at,
                x,
                from: bar.shown_x(now),
                since: now,
                strip_x,
            },
            _ => DropBar {
                group,
                at,
                x,
                from: x,
                since: now,
                strip_x,
            },
        };
        self.drop_bar = Some(next);
        cx.notify();
    }

    /// A tab dropped on group `to`'s strip lands where the drop bar says,
    /// or at `fallback` when the bar never found a boundary.
    pub(crate) fn drop_tab_at_bar(
        &mut self,
        drag: &TabDrag,
        to: u64,
        fallback: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        let at = match self.drop_bar {
            Some(bar) if bar.group == to => Some(bar.at),
            _ => fallback,
        };
        self.drop_tab_on_strip(drag, to, at, cx);
    }

    /// The dragged tab moved: over group `id`'s content at `position` it
    /// lights the zone it would land in, and leaving it puts the light out.
    /// Every group hears every move, so only the one under the pointer
    /// writes, and only the one that was lit clears.
    pub(crate) fn tab_drag_moved(
        &mut self,
        id: u64,
        bounds: gpui::Bounds<gpui::Pixels>,
        position: gpui::Point<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) {
        let next = if bounds.contains(&position) {
            Some((id, DropZone::at(bounds, position)))
        } else if self.tab_drop.is_some_and(|(g, _)| g == id) {
            None
        } else {
            return;
        };
        if self.tab_drop != next {
            self.tab_drop = next;
            cx.notify();
        }
    }

    /// A tab dropped on group `id`'s content: into its strip from the
    /// middle, into a new group beside it from a side.
    pub(crate) fn drop_tab_on_group(
        &mut self,
        drag: &TabDrag,
        id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let zone = match self.tab_drop.take() {
            Some((g, zone)) if g == id => zone,
            _ => DropZone::Center,
        };
        self.dragging_tab = false;
        self.drop_bar = None;
        self.tab_bounds.clear();
        match zone.split() {
            None => self.move_tab(drag, id, None, cx),
            Some((direction, after)) => self.split_with_tab(drag, id, direction, after, window, cx),
        }
    }

    /// A tab dropped on group `to`'s strip, before tab `at` or at the end:
    /// it joins that strip, there, and is the active tab.
    pub(crate) fn drop_tab_on_strip(
        &mut self,
        drag: &TabDrag,
        to: u64,
        at: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        self.tab_drop = None;
        self.dragging_tab = false;
        self.drop_bar = None;
        self.tab_bounds.clear();
        self.move_tab(drag, to, at, cx);
    }

    /// Takes tab `ix` out of group `from_ix` with every group stashed. A
    /// group whose active tab left shows the one that takes its place,
    /// which means making it live for a moment: its views only ever load
    /// through the live fields. Says whether the group has nothing left.
    pub(crate) fn take_tab(
        &mut self,
        from_ix: usize,
        ix: usize,
        cx: &mut Context<Self>,
    ) -> Option<Tab> {
        let group = &mut self.groups[from_ix];
        if ix >= group.tabs.len() {
            return None;
        }
        let tab = group.tabs.remove(ix);
        if group.tabs.is_empty() {
            return Some(tab);
        }
        if ix < group.active_tab {
            group.active_tab -= 1;
        } else if ix == group.active_tab {
            group.active_tab = ix.min(group.tabs.len() - 1);
            self.load_group(from_ix, cx);
            self.load_tab(cx);
            self.stash_active_group();
        }
        Some(tab)
    }

    /// Moves the dragged tab to group `to`, before its tab `at` or at the
    /// end, and makes it the active tab of the active group. In its own
    /// group that is a reorder. A group left with nothing closes, as one
    /// whose last tab closed does.
    pub(crate) fn move_tab(
        &mut self,
        drag: &TabDrag,
        to: u64,
        at: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        self.flush_save(cx);
        self.stash_active_group();
        let (Some(from_ix), Some(to_ix)) = (self.group_ix(drag.group), self.group_ix(to)) else {
            return;
        };
        let mut at = at.unwrap_or(usize::MAX);
        let mut empty = false;
        let tab = if from_ix == to_ix {
            // * A reorder: the strip's views keep showing what they show
            //   until the end, when the dragged tab takes over if it must.
            let group = &mut self.groups[to_ix];
            if drag.tab >= group.tabs.len() {
                return;
            }
            if drag.tab < at {
                at -= 1;
            }
            group.tabs.remove(drag.tab)
        } else {
            let Some(tab) = self.take_tab(from_ix, drag.tab, cx) else {
                return;
            };
            empty = self.groups[from_ix].tabs.is_empty();
            tab
        };
        // * What the target's views show now: the tab that was active there,
        //   or for a reorder whose active tab left, nothing in particular.
        let target = &mut self.groups[to_ix];
        let shown = (from_ix != to_ix || drag.tab != target.active_tab)
            .then(|| target.tabs.get(target.active_tab).cloned())
            .flatten();
        let at = at.min(target.tabs.len());
        target.tabs.insert(at, tab);
        target.active_tab = at;
        self.load_group(to_ix, cx);
        if shown.is_none() || self.tabs.get(self.active_tab) != shown.as_ref() {
            self.load_tab(cx);
        }
        if empty {
            self.close_group(drag.group, false, cx);
        }
        cx.notify();
    }

    /// Puts the dragged tab in a new group beside group `beside`, after it
    /// or before it along `direction`, the way "Split right" and "Split
    /// down" place theirs, and makes that group active. A group's only tab
    /// dropped on its own edge has nowhere else to be, so nothing happens.
    pub(crate) fn split_with_tab(
        &mut self,
        drag: &TabDrag,
        beside: u64,
        direction: SplitDirection,
        after: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.flush_save(cx);
        self.stash_active_group();
        let Some(from_ix) = self.group_ix(drag.group) else {
            return;
        };
        if drag.group == beside && self.groups[from_ix].tabs.len() <= 1 {
            return;
        }
        if self.group_ix(beside).is_none() {
            return;
        }
        let Some(tab) = self.take_tab(from_ix, drag.tab, cx) else {
            return;
        };
        let empty = self.groups[from_ix].tabs.is_empty();
        let id = self.new_group(tab.mode, window, cx);
        self.place_beside(beside, id, direction, after);
        let Some(ix) = self.group_ix(id) else {
            return;
        };
        self.groups[ix].tabs = vec![tab];
        self.groups[ix].active_tab = 0;
        self.load_group(ix, cx);
        self.load_tab(cx);
        if empty {
            self.close_group(drag.group, false, cx);
        }
        cx.notify();
    }

    /// `workspace:close-tab-group`, and what closing a group's last tab
    /// does: the group goes, its siblings take the room, and the keyboard
    /// goes to the group used most recently before it. The last group
    /// cannot go; it is left with an empty tab instead. `remember` puts the
    /// group's tabs where `workspace:undo-close-pane` finds them.
    pub(crate) fn close_group(&mut self, id: u64, remember: bool, cx: &mut Context<Self>) {
        let Some(ix) = self.group_ix(id) else {
            return;
        };
        if self.groups.len() == 1 {
            self.flush_save(cx);
            self.stash_active_tab();
            let tabs = std::mem::take(&mut self.tabs);
            if remember {
                for (i, tab) in tabs.into_iter().enumerate() {
                    self.remember_closed(i, tab);
                }
            }
            self.tabs.push(Tab::empty(self.default_mode()));
            self.active_tab = 0;
            self.load_tab(cx);
            return;
        }
        let was_active = ix == self.active_group;
        let active_id = self.active_group_id();
        // * Nothing paints this group's header again, so its title's shaped
        //   line goes with it.
        self.title_layout.remove(&id);
        if was_active {
            self.flush_save(cx);
            self.stash_active_group();
        }
        if remember {
            let tabs = self.groups[ix].tabs.clone();
            for (i, tab) in tabs.into_iter().enumerate() {
                self.remember_closed(i, tab);
            }
        }
        let mut parent = self.layout_tree.path_to(id).unwrap_or_default();
        parent.pop();
        self.layout_tree.remove(id);
        self.groups.remove(ix);
        self.group_mru.retain(|g| *g != id);
        // * The split the group left, or the nearest one still standing when
        //   it was the split's last child and went with it.
        loop {
            if matches!(
                self.layout_tree.node_at(&parent),
                Some(Layout::Split { .. })
            ) {
                self.renormalise_dimensions(&parent);
                break;
            }
            if parent.pop().is_none() {
                break;
            }
        }
        if was_active {
            let next = self
                .group_mru
                .first()
                .copied()
                .and_then(|g| self.group_ix(g))
                .unwrap_or(0);
            self.load_group(next, cx);
        } else {
            self.active_group = self.group_ix(active_id).unwrap_or(0);
            cx.notify();
        }
    }

    /// The `dimension` of a node of the layout: a group's lives on the
    /// group, a split's on the split.
    pub(crate) fn dimension_of(&self, node: &Layout) -> Option<f64> {
        match node {
            Layout::Group(id) => self.groups[self.group_ix(*id)?].dimension(),
            Layout::Split { node, .. } => node.get("dimension").and_then(Value::as_f64),
        }
    }

    pub(crate) fn set_dimension_at(&mut self, path: &[usize], dimension: Option<f64>) {
        match self.layout_tree.node_at_mut(path) {
            Some(Layout::Group(id)) => {
                let id = *id;
                if let Some(ix) = self.group_ix(id) {
                    self.groups[ix].set_dimension(dimension);
                }
            }
            Some(Layout::Split { node, .. }) => set_dimension(node, dimension),
            None => {}
        }
    }

    /// After a child left the split at `path`, the rest share its room: their
    /// `dimension`s scaled back to a hundred, or dropped when one is left,
    /// which is what Obsidian wrote in both cases.
    pub(crate) fn renormalise_dimensions(&mut self, path: &[usize]) {
        let Some(Layout::Split { children, .. }) = self.layout_tree.node_at(path) else {
            return;
        };
        let dims: Vec<Option<f64>> = children.iter().map(|c| self.dimension_of(c)).collect();
        let n = dims.len();
        let scaled: Vec<Option<f64>> = if n <= 1 {
            vec![None; n]
        } else if dims.iter().all(Option::is_some) {
            let sum: f64 = dims.iter().flatten().sum();
            if sum <= 0.0 {
                return;
            }
            dims.iter().map(|d| d.map(|d| d * 100.0 / sum)).collect()
        } else {
            return;
        };
        for (ix, d) in scaled.into_iter().enumerate() {
            let mut child = path.to_vec();
            child.push(ix);
            self.set_dimension_at(&child, d);
        }
    }

    /// A divider between two children of a split, dragged to `position`
    /// inside the split's `bounds`: the pair's room is redistributed and
    /// every child's share written as a `dimension`, as Obsidian does once
    /// a divider has moved.
    pub(crate) fn drag_divider(
        &mut self,
        drag: &DividerDrag,
        bounds: gpui::Bounds<gpui::Pixels>,
        position: gpui::Point<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(Layout::Split {
            direction,
            children,
            ..
        }) = self.layout_tree.node_at(&drag.path)
        else {
            return;
        };
        let n = children.len();
        if drag.before + 1 >= n {
            return;
        }
        let row = *direction == SplitDirection::Vertical;
        let (origin, size) = if row {
            (bounds.origin.x, bounds.size.width)
        } else {
            (bounds.origin.y, bounds.size.height)
        };
        let along = if row { position.x } else { position.y };
        let size = f32::from(size);
        if size <= 0.0 {
            return;
        }
        let at = (f32::from(along - origin) / size).clamp(0.0, 1.0) as f64 * 100.0;
        let mut dims: Vec<f64> = match children
            .iter()
            .map(|c| self.dimension_of(c))
            .collect::<Option<Vec<f64>>>()
        {
            Some(dims) => dims,
            None => vec![100.0 / n as f64; n],
        };
        let before: f64 = dims[..drag.before].iter().sum();
        let pair = dims[drag.before] + dims[drag.before + 1];
        const MIN: f64 = 5.0;
        if pair <= 2.0 * MIN {
            return;
        }
        let a = (at - before).clamp(MIN, pair - MIN);
        dims[drag.before] = a;
        dims[drag.before + 1] = pair - a;
        for (ix, d) in dims.into_iter().enumerate() {
            let mut child = drag.path.clone();
            child.push(ix);
            self.set_dimension_at(&child, Some(d));
        }
        cx.notify();
    }

    /// `editor:focus-left` and its three siblings: the keyboard moves to the
    /// group on that side — of the groups there, the one used most recently,
    /// which is where Obsidian's went.
    pub(crate) fn focus_neighbor(
        &mut self,
        direction: SplitDirection,
        forward: bool,
        cx: &mut Context<Self>,
    ) {
        let candidates = self
            .layout_tree
            .neighbors(self.active_group_id(), direction, forward);
        let target = self
            .group_mru
            .iter()
            .find(|g| candidates.contains(g))
            .copied()
            .or_else(|| candidates.first().copied());
        if let Some(target) = target {
            self.activate_group(target, cx);
        }
    }

    /// Every tab but the live one: the active group's other tabs and all of
    /// the other groups'. What the live one needs is done on the workspace's
    /// own fields.
    pub(crate) fn stashed_tabs_mut(&mut self) -> impl Iterator<Item = &mut Tab> {
        let active_tab = self.active_tab;
        let active_group = self.active_group;
        let own = self
            .tabs
            .iter_mut()
            .enumerate()
            .filter(move |(ix, _)| *ix != active_tab)
            .map(|(_, t)| t);
        let others = self
            .groups
            .iter_mut()
            .enumerate()
            .filter(move |(gx, _)| *gx != active_group)
            .flat_map(|(_, g)| g.tabs.iter_mut());
        own.chain(others)
    }

    /// The group whose area the one graph view paints in: the active group
    /// while it shows the graph, else the first group that does.
    pub(crate) fn graph_group(&self) -> Option<u64> {
        if self.mode == ViewMode::Graph {
            return Some(self.active_group_id());
        }
        let mut order = Vec::new();
        self.layout_tree.groups(&mut order);
        order.into_iter().find(|id| {
            self.group_ix(*id).is_some_and(|ix| {
                let g = &self.groups[ix];
                g.tabs
                    .get(g.active_tab)
                    .is_some_and(|t| t.mode == ViewMode::Graph)
            })
        })
    }

    /// A sidebar's collapse toggle: it stays on screen when the sidebar is
    /// collapsed, which is what brings the sidebar back.
    pub(crate) fn sidebar_toggle(
        &self,
        left: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let visible = if left {
            self.left_visible
        } else {
            self.right_visible
        };
        let command = if left {
            "app:toggle-left-sidebar"
        } else {
            "app:toggle-right-sidebar"
        };
        let _ = visible;
        icon_button(
            if left { "toggle-left" } else { "toggle-right" },
            if left { "panel-left" } else { "panel-right" },
            theme,
        )
        .on_click(cx.listener(move |this, _, window, cx| this.run_command(command, window, cx)))
    }

    /// Whether `appearance.json` keeps the ribbon on screen.
    pub(crate) fn show_ribbon(&self) -> bool {
        self.state
            .as_ref()
            .map(|s| s.appearance().show_ribbon.unwrap_or(true))
            .unwrap_or(true)
    }

    /// The per-file header above each leaf: the path crumbs and the history
    /// arrows, off when `showViewHeader` is false. The tab strip stays — it
    /// is what Obsidian calls the tab title bar.
    pub(crate) fn show_view_header(&self) -> bool {
        self.state
            .as_ref()
            .map(|s| s.appearance().show_view_header.unwrap_or(true))
            .unwrap_or(true)
    }

    /// Whether a link opened in a new tab takes the screen: Obsidian's
    /// `alwaysFocusNewTabs`, on by default.
    pub(crate) fn focus_new_tab(&self) -> bool {
        self.state
            .as_ref()
            .map(|s| s.app().focus_new_tab.unwrap_or(true))
            .unwrap_or(true)
    }

    /// Whether the status bar carries the mode toggle: the `editor-status`
    /// core plugin, on by default. It is not in `CORE_PLUGINS` — Obsidian
    /// writes it in `core-plugins.json` but gives it no settings tab — so
    /// the map is read directly rather than through
    /// [`Self::core_plugin_enabled`], which would answer `true` for an id
    /// it does not know.
    pub(crate) fn editor_status_shown(&self) -> bool {
        let Some(state) = self.state.as_ref() else {
            return true;
        };
        match state.core_plugins().as_ref() {
            Some(CorePlugins::Map(m)) => m.get("editor-status").copied().unwrap_or(true),
            Some(CorePlugins::List(l)) => l.iter().any(|p| p == "editor-status"),
            None => true,
        }
    }

    /// Whether the window has macOS's hidden frame: the desktop's traffic
    /// lights in the title row's top-left corner, no title and no window
    /// buttons of Silica's. Obsidian keeps the sidebar toggles in the
    /// headers there — the left one in the left dock's header, or at the
    /// head of the root tab strip while that dock is collapsed, never above
    /// the ribbon — and the right one in the window's top-right corner,
    /// inside the right dock's own header while that dock is open.
    pub(crate) fn mac_frame(&self) -> bool {
        cfg!(target_os = "macos") && self.frameless
    }

    /// What the header in the window's top-left corner is padded by past
    /// its usual 8 px: Obsidian's `--frame-left-space`, room for the
    /// traffic lights on macOS and nothing elsewhere or in fullscreen,
    /// where the lights are gone.
    pub(crate) fn frame_left_space(&self) -> f32 {
        if !self.mac_frame() || self.fullscreen {
            return 0.;
        }
        MAC_FRAME_LEFT_SPACE - if self.show_ribbon() { RIBBON_WIDTH } else { 0. }
    }

    /// A sidebar's resize handle: the 3 px strip on its inner edge, which
    /// Obsidian colours with the accent while the pointer is over it.
    pub(crate) fn dock_handle(&self, left: bool, theme: &Theme) -> gpui::Stateful<gpui::Div> {
        div()
            .id(if left {
                "left-dock-handle"
            } else {
                "right-dock-handle"
            })
            .absolute()
            .top_0()
            .h_full()
            .w(px(3.))
            .when(left, |d| d.right(px(0.)).border_r_1())
            .when(!left, |d| d.left(px(0.)).border_l_1())
            .border_color(theme.border)
            .cursor_col_resize()
            .occlude()
            .hover(|s| s.bg(theme.accent).border_color(theme.accent))
            .on_drag(DockDrag { left }, |_, _, _, cx| cx.new(|_| NoDragPreview))
    }

    /// A sidebar handle dragged to `position` across the row of `bounds`:
    /// the dock follows the pointer, between a header row's worth and what
    /// leaves the main area room.
    pub(crate) fn drag_dock(
        &mut self,
        left: bool,
        bounds: gpui::Bounds<Pixels>,
        position: gpui::Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let x = f64::from(f32::from(position.x));
        let row = f64::from(f32::from(bounds.size.width));
        let max = (row - 2. * MIN_SIDEBAR_WIDTH).max(MIN_SIDEBAR_WIDTH);
        if left {
            let ribbon = if self.show_ribbon() {
                RIBBON_WIDTH as f64
            } else {
                0.
            };
            let origin = f64::from(f32::from(bounds.origin.x)) + ribbon;
            self.left_width = (x - origin).clamp(MIN_SIDEBAR_WIDTH, max);
        } else {
            let right = f64::from(f32::from(bounds.origin.x + bounds.size.width));
            self.right_width = (right - x).clamp(MIN_SIDEBAR_WIDTH, max);
        }
        cx.notify();
    }

    /// The window's own buttons in its top right corner, where Obsidian's
    /// hidden frame keeps them: minimise, maximise or restore, and close,
    /// each 44 by 40.
    /// The minimise, maximise and close buttons in the window's right
    /// corner, drawn only when the frame is Silica's.
    ///
    /// Windows wants these three named to the hit test rather than clicked:
    /// the system then performs them itself, and hovering the maximise
    /// button opens Windows 11's snap layouts, which an ordinary click
    /// handler can never offer. `occlude` is what keeps the press from
    /// reaching the caption area underneath and dragging the window instead.
    pub(crate) fn render_window_controls(&self, theme: &Theme, window: &Window) -> gpui::Div {
        let maximized = window.is_maximized();
        let system = cfg!(target_os = "windows");
        let corners = self.corners;
        let button = |id: &'static str,
                      name: &'static str,
                      close: bool,
                      area: WindowControlArea,
                      act: fn(&mut Window)| {
            let hover_bg: Hsla = if close {
                rgb(0xe93147).into()
            } else {
                theme.bg_alt
            };
            let color = theme.text_muted;
            div()
                .id(id)
                .group(id)
                .w(px(RIBBON_WIDTH))
                .h(px(HEADER_HEIGHT))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .when(close && corners.tr, |d| d.rounded_tr(WINDOW_RADIUS))
                .hover(move |s| s.bg(hover_bg))
                .when(system, |d| d.occlude().window_control_area(area))
                .when(!system, |d| d.on_click(move |_, window, _| act(window)))
                .child(
                    div()
                        .when(close, |d| {
                            d.group_hover(id, |s| s.text_color(gpui::white()))
                        })
                        .text_color(color)
                        .child(icon(name, 14., color)),
                )
        };
        div()
            .absolute()
            .top_0()
            .right_0()
            .flex()
            .flex_row()
            .child(button(
                "win-minimize",
                "minus",
                false,
                WindowControlArea::Min,
                |w| w.minimize_window(),
            ))
            .child(button(
                "win-maximize",
                if maximized { "copy" } else { "square" },
                false,
                WindowControlArea::Max,
                |w| w.zoom_window(),
            ))
            .child(button(
                "win-close",
                "x",
                true,
                WindowControlArea::Close,
                |w| w.remove_window(),
            ))
    }

    /// The ribbon down the left edge: the quick switcher, the graph, a new
    /// canvas, today's daily note, a template and the palette. `workspace.json`'s
    /// `left-ribbon.hiddenItems` hides an entry by the plugin id its key starts
    /// with. Settings is not here: Obsidian 1.14 leaves `.side-dock-settings`
    /// empty and puts the gear in the sidebar's footer, the vault profile.
    pub(crate) fn render_ribbon(&self, theme: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let hidden = |plugin: &str| -> bool {
            self.layout
                .as_ref()
                .and_then(|l| l.get("left-ribbon")?.get("hiddenItems")?.as_object())
                .is_some_and(|items| {
                    items.iter().any(|(key, hidden)| {
                        key.split(':').next() == Some(plugin) && hidden.as_bool() == Some(true)
                    })
                })
        };
        let enabled = |plugin: &str| -> bool {
            self.state
                .as_ref()
                .is_some_and(|s| settings::plugin_enabled(s, plugin))
        };
        // * The corner above the ribbon is part of the title row: it carries
        //   the headers' bottom line across, and the ribbon's own divider
        //   starts below it, since Obsidian's ribbon begins under the title
        //   row. The left sidebar's toggle sits in the corner, which is where
        //   Obsidian's hidden frame puts it on Linux and Windows. On macOS the
        //   corner is the traffic lights' and only drags the window; the
        //   toggle is in the headers instead.
        let toggle: gpui::AnyElement = if self.mac_frame() {
            window_handle(
                div()
                    .id("ribbon-corner")
                    .w(px(RIBBON_WIDTH))
                    .h(px(HEADER_HEIGHT))
                    .border_b_1()
                    .border_color(theme.border),
                true,
            )
            .into_any_element()
        } else {
            div()
                .w(px(RIBBON_WIDTH))
                .h(px(HEADER_HEIGHT))
                .flex()
                .items_center()
                .justify_center()
                .border_b_1()
                .border_color(theme.border)
                .child(self.sidebar_toggle(true, theme, cx))
                .into_any_element()
        };
        let button = |id: &'static str, name: &'static str, command: &'static str| {
            icon_button(id, name, theme).on_click(
                cx.listener(move |this, _, window, cx| this.run_command(command, window, cx)),
            )
        };
        // * The icons are the ones Obsidian's ribbon names: `lucide-file-search`
        //   for the switcher, `git-fork` for the graph, `layout-dashboard` for
        //   a new canvas, `calendar`, `files` for a template, `terminal` for
        //   the palette and `layout-list` for a new base -- in that order,
        //   which is the order the DOM was read in.
        let items: [(&'static str, &'static str, &'static str, &'static str); 7] = [
            (
                "ribbon-switcher",
                "file-search",
                "switcher:open",
                "switcher",
            ),
            ("ribbon-graph", "git-fork", "graph:open", "graph"),
            (
                "ribbon-canvas",
                "layout-dashboard",
                "canvas:new-file",
                "canvas",
            ),
            ("ribbon-daily", "calendar", "daily-notes", "daily-notes"),
            ("ribbon-template", "files", "insert-template", "templates"),
            (
                "ribbon-palette",
                "terminal",
                "command-palette:open",
                "command-palette",
            ),
            ("ribbon-base", "layout-list", "bases:new-file", "bases"),
        ];
        let buttons: Vec<_> = items
            .into_iter()
            .filter(|(_, _, _, plugin)| enabled(plugin) && !hidden(plugin))
            .map(|(id, name, command, _)| button(id, name, command))
            .collect();
        div()
            .flex()
            .flex_col()
            .items_center()
            .flex_shrink_0()
            .w(px(RIBBON_WIDTH))
            .h_full()
            .bg(theme.bg_secondary)
            .map(|d| self.corners.left().apply(d))
            .child(toggle)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .flex_1()
                    .w_full()
                    .border_r_1()
                    .border_color(theme.border)
                    .pt(px(8.))
                    .pb(px(12.))
                    .gap(px(6.))
                    .children(buttons),
            )
    }

    /// The left sidebar's footer, Obsidian's vault profile: the vault's
    /// name behind a `chevrons-up-down` switcher on the left, Help and
    /// Settings on the right, above a hairline. It is part of the sidebar,
    /// so it collapses with it, and it stays whether the ribbon shows or not.
    pub(crate) fn render_vault_profile(
        &self,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let menu = self.render_vault_menu(theme, window, cx);
        let button = |id: &'static str, name: &'static str, command: &'static str| {
            icon_button(id, name, theme).on_click(
                cx.listener(move |this, _, window, cx| this.run_command(command, window, cx)),
            )
        };
        div()
            .flex()
            .flex_row()
            .items_center()
            .flex_shrink_0()
            .h(px(HEADER_HEIGHT))
            .px(px(8.))
            .gap(px(2.))
            .border_t_1()
            .border_color(theme.border)
            .child(
                div()
                    .id("vault-switcher")
                    .flex()
                    .flex_row()
                    .items_center()
                    .min_w_0()
                    .flex_1()
                    .h(px(24.))
                    .pr(px(6.))
                    .rounded(px(4.))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.bg_alt))
                    .when(self.vault_menu_open, |d| d.bg(theme.bg_alt))
                    .on_hover(cx.listener(|this, over: &bool, _, _| {
                        this.vault_switcher_hovered = *over;
                    }))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_vault_menu(cx)))
                    // * The menu's anchor is this box, pinned at the row's
                    //   top-left corner: an absolute element left to the
                    //   row's own centring would hang from its middle.
                    .child(div().absolute().top_0().left_0().size_0().children(menu))
                    .child(
                        div()
                            .flex_shrink_0()
                            .w(px(28.))
                            .h(px(24.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(icon("chevrons-up-down", ICON_SIZE, theme.text_muted)),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .text_size(px(FONT_UI_SMALL))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(theme.text)
                            .child(self.vault_name.clone()),
                    ),
            )
            .child(button("vault-help", "circle-help", "app:open-help"))
            .child(button("vault-settings", "settings", "app:open-settings"))
    }

    /// The vault switcher's menu, dropped up from the footer: every vault
    /// this machine has opened whose folder is still there, in the
    /// registry's own order with a check on the one showing, then
    /// "Manage vaults...". A folder that is gone is left out rather than
    /// listed faint, and the row is the folder's name alone however many
    /// vaults share it — both are what Obsidian's own switcher does
    /// (probe of 2026-10-29); the path is the row's tooltip. A click
    /// outside closes it, and so does a second click on the footer's own
    /// row.
    pub(crate) fn render_vault_menu(
        &self,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        if !self.vault_menu_open {
            return None;
        }
        let current = self.state.as_ref().map(|s| s.root.clone());
        let registry = VaultRegistry::open().ok().flatten();
        let known: Vec<(String, PathBuf)> = registry
            .as_ref()
            .map(|r| {
                r.vaults()
                    .into_iter()
                    .filter(|(_, e)| e.path.is_dir())
                    .map(|(_, e)| (e.name(), e.path.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let item = |id: gpui::ElementId, theme: &Theme| {
            div()
                .id(id)
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .px_3()
                .py(px(5.))
                .rounded(px(4.))
                .mx_1()
                .cursor_pointer()
                .hover(|s| s.bg(theme.bg_alt))
        };
        let check = |on: bool, theme: &Theme| {
            div()
                .w(px(16.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .children(on.then(|| icon("check", 14., theme.text_muted)))
        };
        // * A machine that has opened dozens of vaults has a menu taller than
        //   the window, and the rows under the hairline are then off the
        //   bottom edge however the menu is snapped: cap it at the window and
        //   scroll, as Obsidian's own menus do inside their `.menu-scroll`.
        let tallest = (window.viewport_size().height - px(96.)).max(px(160.));
        let mut list = div()
            .id("vault-menu")
            .flex()
            .flex_col()
            .min_w(px(240.))
            .max_w(px(360.))
            .max_h(tallest)
            .overflow_y_scroll()
            .py_1()
            .bg(theme.bg_secondary)
            .border_1()
            .border_color(theme.border)
            .rounded_md()
            .shadow_lg()
            .text_size(px(FONT_UI_SMALL))
            .text_color(theme.text)
            .occlude()
            // * A press on the switcher row is a press outside the menu, and
            //   the click that ends it toggles: closing here as well would
            //   close and reopen the menu in one press. The row's own
            //   handler does the closing when the press is its.
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                if this.vault_switcher_hovered {
                    return;
                }
                this.vault_menu_open = false;
                cx.notify();
            }));
        if known.is_empty() {
            list = list.child(
                div()
                    .px_3()
                    .py(px(5.))
                    .text_color(theme.text_faint)
                    .child("No vaults opened yet"),
            );
        }
        for (ix, (name, path)) in known.into_iter().enumerate() {
            let showing = current.as_deref() == Some(path.as_path());
            let tooltip = crate::canvas::menu::tooltip(path.display().to_string());
            list = list.child(
                item(("vault-menu-item", ix).into(), theme)
                    .tooltip(tooltip)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.vault_menu_open = false;
                        if showing {
                            cx.notify();
                        } else if path.is_dir() {
                            this.load_vault(path.clone(), window, cx);
                        } else {
                            this.report(format!("{} is not a folder any more", path.display()), cx);
                        }
                    }))
                    .child(check(showing, theme))
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .child(name),
                    ),
            );
        }
        // * Obsidian's tail is a single row under the separator, "Manage
        //   vaults...", which raises the vault manager (probe of
        //   2026-10-29). Silica's own "Open vault..." and "Open sandbox
        //   vault" moved into that window, where Obsidian keeps the same
        //   two; they are still commands, and still in the palette.
        list =
            list.child(div().my_1().h(px(1.)).bg(theme.border)).child(
                item("vault-menu-manage".into(), theme)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.run_command("app:open-vault", window, cx)
                    }))
                    .child(
                        div()
                            .w(px(16.))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .child(icon("folder-open", 14., theme.text_muted)),
                    )
                    .child("Manage vaults..."),
            );
        Some(
            deferred(
                anchored()
                    .anchor(gpui::Anchor::BottomLeft)
                    .offset(point(px(0.), px(-6.)))
                    .snap_to_window_with_margin(px(8.))
                    .child(list),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }

    /// A group's tab strip. The sidebar toggles sit at the window's edges:
    /// the left one in the first group's strip, the right one in the last's.
    /// `first` and `last` say the group holds the main area's top left and
    /// top right corner: those strips carry the collapsed sidebars' toggles
    /// and clear the window controls.
    pub(crate) fn render_tab_strip(
        &self,
        view: &GroupView,
        first: bool,
        last: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let tabs = &view.tabs;
        let active_ix = view.active_tab;
        let group = view.id;
        let group_active = view.active;
        let show_ribbon = self.show_ribbon();
        let frameless = self.frameless;
        let mac = self.mac_frame();
        // * The window buttons overlay the right end of whichever header is
        //   in the window's top right corner: this strip's, when the right
        //   sidebar is collapsed and this is the last group. macOS has none.
        let under_controls = frameless && !mac && last && !self.right_visible;
        // * The strip in the window's top-left corner, with the left sidebar
        //   collapsed, keeps clear of the traffic lights on macOS.
        let left_space = if first && !self.left_visible {
            self.frame_left_space()
        } else {
            0.
        };
        let menu_open = self.tab_list_menu == Some(group);
        let at_left = first && !self.show_ribbon() && !self.left_visible;
        let at_right = last && !self.right_visible;
        div()
            .flex()
            .flex_row()
            .items_end()
            .when(at_left && self.corners.tl, |d| d.rounded_tl(WINDOW_RADIUS))
            .when(at_right && self.corners.tr, |d| d.rounded_tr(WINDOW_RADIUS))
            .h(px(HEADER_HEIGHT))
            .pt(px(6.))
            .pl(px(8. + left_space))
            .pr(if under_controls {
                px(3. * RIBBON_WIDTH)
            } else {
                px(8.)
            })
            .bg(theme.bg_secondary)
            .relative()
            .overflow_hidden()
            // * A dragged tab over the strip moves the drop bar to the
            //   nearest tab boundary; dropped, it lands there (at the end,
            //   should the bar never have found one).
            .on_drag_move(cx.listener(move |this, e: &DragMoveEvent<TabDrag>, _, cx| {
                this.strip_drag_moved(group, e.bounds, e.event.position, cx)
            }))
            .on_drop(cx.listener(move |this, drag: &TabDrag, _, cx| {
                this.drop_tab_at_bar(drag, group, None, cx)
            }))
            // * The strip's outline is a hairline along its bottom edge rather
            //   than a border, so the active tab, which reaches the edge, paints
            //   over it and joins the view header below without a seam. The
            //   other tabs stop a pixel short and keep the line under them.
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .h(px(1.))
                    .bg(theme.border),
            )
            // * Without the ribbon a collapsed left sidebar would have no
            //   way back from the window; macOS keeps the toggle here, after
            //   the traffic lights, ribbon or not.
            .when(first && !self.left_visible && (!show_ribbon || mac), |d| {
                d.child(self.sidebar_toggle(true, theme, cx).mb(px(8.)).mr(px(2.)))
            })
            .children(
                tabs.iter()
                    .enumerate()
                    .filter(|(ix, _)| tab_in_strip(*ix, active_ix, view.stacked))
                    .map(|(ix, tab)| {
                        let active = ix == active_ix;
                        let divider = !active && ix + 1 != active_ix;
                        let title = tab.title();
                        let drag = TabDrag {
                            group,
                            tab: ix,
                            title: title.clone(),
                        };
                        let this = cx.weak_entity();
                        // * Obsidian's tab: 34 px tall, 200 px wide, the active one
                        //   in the content's colour, its top corners as round as its
                        //   feet, its title in the small UI size and its close button a
                        //   20 px disc. It is padded `1px 4px 4px` around an inner box
                        //   with a 4 px radius, and that inner box, not the tab, is
                        //   what the hover wash paints.
                        div()
                            .id(("tab", ix))
                            .group("tab")
                            .flex()
                            .flex_row()
                            .h(px(34.))
                            .w(px(200.))
                            .min_w(px(60.))
                            .flex_shrink()
                            .pt(px(1.))
                            .px(px(4.))
                            .pb(px(4.))
                            .rounded_t(px(TAB_CURVE))
                            .relative()
                            .when(!active, |d| d.mb(px(1.)))
                            // * The divider on an inactive tab's right edge, as
                            //   Obsidian's `::after`: a 1 px line, 15 px tall and 8 px
                            //   down, astride the edge (the tabs touch, so it is
                            //   centred between two inner boxes, 3.5 px from each),
                            //   and after the last tab it parts the strip from the
                            //   new-tab button. The active tab has none and paints
                            //   over its left neighbour's, so the gaps around it are
                            //   bare. Hover leaves it alone: the wash stops 4 px short
                            //   of the edge, so the line stays beside the hovered box.
                            //   Deferred, like the feet: the next tab paints after
                            //   this one with an opaque background, and the half of
                            //   the line astride its edge would otherwise be lost
                            //   under it, whole or in part depending on where the
                            //   edge falls between device pixels.
                            .when(divider, |d| {
                                d.child(deferred(
                                    div()
                                        .absolute()
                                        .right(px(-0.5))
                                        .top(px(8.))
                                        .w(px(1.))
                                        .h(px(15.))
                                        .bg(theme.tab_divider),
                                ))
                            })
                            // * The active tab's feet: the concave curves that carry
                            //   its bottom corners out into the view header. Painted
                            //   late so a neighbour's hover wash stays underneath.
                            .when(active, |d| {
                                d.child(deferred(tab_foot(true, theme)))
                                    .child(deferred(tab_foot(false, theme)))
                            })
                            .cursor_pointer()
                            .text_size(px(FONT_UI_SMALL))
                            .bg(if active { theme.bg } else { theme.bg_secondary })
                            .text_color(if active && group_active {
                                theme.text
                            } else if active {
                                theme.text_muted
                            } else {
                                theme.text_faint
                            })
                            .when(!active, |d| d.hover(|s| s.text_color(theme.text_muted)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.activate_group(group, cx);
                                this.activate_tab(ix, cx)
                            }))
                            // * Picking the tab up: the ghost is its title, and the
                            //   workspace is told so every group shows its zones.
                            .on_drag(drag, move |drag, grab, _, cx| {
                                this.update(cx, |w, cx| w.begin_tab_drag(cx)).ok();
                                let title = drag.title.clone();
                                cx.new(|_| TabDragPreview { title, grab })
                            })
                            // * Where the tab is, for the strip's drop bar; a tab
                            //   dropped on it lands at the bar, else in front of it.
                            .on_drag_move(cx.listener(
                                move |this, e: &DragMoveEvent<TabDrag>, _, _| {
                                    if this.dragging_tab {
                                        this.tab_bounds.insert((group, ix), e.bounds);
                                    }
                                },
                            ))
                            .on_drop(cx.listener(move |this, drag: &TabDrag, _, cx| {
                                this.drop_tab_at_bar(drag, group, Some(ix), cx)
                            }))
                            .on_mouse_down(
                                gpui::MouseButton::Middle,
                                cx.listener(move |this, _, _, cx| {
                                    this.activate_group(group, cx);
                                    this.close_tab(ix, cx)
                                }),
                            )
                            // * The inner box: the title and the close button, padded
                            //   `0 3px 0 6px`, washed on hover when the tab is not the
                            //   active one (the active tab shows nothing on hover).
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .h_full()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap(px(2.))
                                    .pl(px(6.))
                                    .pr(px(3.))
                                    .rounded(px(4.))
                                    .when(!active, |d| d.group_hover("tab", |s| s.bg(theme.bg_alt)))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .whitespace_nowrap()
                                            .overflow_hidden()
                                            .text_ellipsis()
                                            .child(title),
                                    )
                                    // * The close button: no wash of its own, its cross
                                    //   brightens from muted to normal under the pointer.
                                    .child(
                                        div()
                                            .id(("tab-close", ix))
                                            .group("tab-close")
                                            .flex_shrink_0()
                                            .w(px(20.))
                                            .h(px(20.))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .rounded(px(8.))
                                            .when(!active, |d| {
                                                d.invisible().group_hover("tab", |s| s.visible())
                                            })
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                cx.stop_propagation();
                                                this.activate_group(group, cx);
                                                this.close_tab(ix, cx);
                                            }))
                                            .child(
                                                icon("x", 14., theme.text_muted)
                                                    .group_hover("tab-close", |s| {
                                                        s.text_color(theme.text)
                                                    }),
                                            ),
                                    ),
                            )
                    }),
            )
            // * Stacked tabs: only the active tab is in the strip, so a
            //   badge beside it steps through the rest of the stack.
            .when(view.stacked && tabs.len() > 1, |d| {
                d.child(
                    div()
                        .id(("stack-badge", group))
                        .flex_shrink_0()
                        .ml(px(4.))
                        .mb(px(7.))
                        .w(px(24.))
                        .h(px(24.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(4.))
                        .cursor_pointer()
                        .hover(|s| s.bg(theme.bg_alt))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.cycle_stack(cx);
                        }))
                        .child(icon("layers", 14., theme.text_muted)),
                )
            })
            .child(
                div()
                    .id("new-tab")
                    .flex_shrink_0()
                    .ml(px(8.))
                    .mr(px(4.))
                    .mb(px(7.))
                    .w(px(24.))
                    .h(px(24.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.bg_alt))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.activate_group(group, cx);
                        this.new_tab(cx)
                    }))
                    .child(icon("plus", ICON_SIZE, theme.text)),
            )
            .child(window_handle(
                div().id(("tab-strip-space", group)).flex_1().h_full(),
                frameless,
            ))
            // * The tab-list button at the strip's right end: Obsidian's
            //   `chevron-down`, 24 px square at y 8.66 with a 2 px right
            //   margin. The menu hangs off the wrapper, so it opens under
            //   the button's top-left corner, 2 px below the button.
            .child(
                div()
                    .relative()
                    .flex_shrink_0()
                    .mr(px(2.))
                    .mb(px(7.))
                    .child(
                        div()
                            .id(("tab-list", group))
                            .w(px(24.))
                            .h(px(24.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(4.))
                            .cursor_pointer()
                            .hover(|s| s.bg(theme.bg_alt))
                            .when(menu_open, |d| d.bg(theme.bg_alt))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.toggle_tab_list_menu(group, cx)
                            }))
                            .child(icon("chevron-down", ICON_SIZE, theme.text)),
                    )
                    .children(self.render_tab_list_menu(view, theme, cx)),
            )
            // * The right sidebar's toggle sits at the root header's right
            //   end in Obsidian's title row whether the sidebar is open (left
            //   of its header) or collapsed (at the window's edge, before the
            //   window buttons): it never moves into the sidebar's own header.
            //   On macOS it keeps to the window's top-right corner, which is
            //   the right dock's own header while that dock is open.
            .when(last && !(mac && self.right_visible), |d| {
                d.child(self.sidebar_toggle(false, theme, cx).mb(px(8.)))
            })
            // * The drop bar: an accent bar astride the boundary the dragged
            //   tab would land at, the height of the tabs' inner boxes.
            //   Deferred past the dividers and feet, which paint late too.
            .when_some(self.drop_bar.filter(|b| b.group == group), |d, bar| {
                let left = bar.shown_x(Instant::now()) - bar.strip_x - DROP_BAR_WIDTH / 2.;
                d.child(deferred(
                    div()
                        .absolute()
                        .left(px(left))
                        .bottom(px(6.))
                        .w(px(DROP_BAR_WIDTH))
                        .h(px(25.))
                        .rounded(px(DROP_BAR_WIDTH / 2.))
                        .bg(theme.accent),
                ))
            })
    }

    pub(crate) fn toggle_tab_list_menu(&mut self, group: u64, cx: &mut Context<Self>) {
        self.tab_list_menu = if self.tab_list_menu == Some(group) {
            None
        } else {
            Some(group)
        };
        cx.notify();
    }

    /// The tab-list menu, as Obsidian 1.14 drops it from the strip's
    /// `chevron-down` (probed 2026-10-22): "Stack tabs" (`layers`), "Bookmark
    /// N tabs..." (`bookmark`, only when a tab has a file), a separator,
    /// "Close all" (`x`), a separator, then one row per tab (`file`) with a
    /// check on the active one. Rows are 25 px, padded `4px 8px` with an
    /// 8 px gap, in a menu padded 6 px whose separators run its full width.
    pub(crate) fn render_tab_list_menu(
        &self,
        view: &GroupView,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        if self.tab_list_menu != Some(view.id) {
            return None;
        }
        let group = view.id;
        let active_ix = view.active_tab;
        let paths: Vec<String> = view.tabs.iter().filter_map(|t| t.path.clone()).collect();
        let row = |id: gpui::ElementId, name: &'static str, theme: &Theme| {
            div()
                .id(id)
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.))
                .px(px(8.))
                .h(px(25.))
                .rounded(px(4.))
                .cursor_pointer()
                .hover(|s| s.bg(theme.bg_alt))
                .child(icon(name, ICON_SIZE, theme.text_muted))
        };
        let separator = |theme: &Theme| div().my(px(6.)).mx(px(-6.)).h(px(1.)).bg(theme.border);
        let mut list = div()
            .id(("tab-list-menu", group))
            .flex()
            .flex_col()
            .min_w(px(167.))
            .max_w(px(360.))
            .p(px(6.))
            .bg(theme.bg_secondary)
            .border_1()
            .border_color(theme.border)
            .rounded(px(8.))
            .shadow_lg()
            .text_size(px(FONT_UI_SMALL))
            .text_color(theme.text)
            .occlude()
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.tab_list_menu = None;
                cx.notify();
            }))
            .child(
                row("tab-list-stack".into(), "layers", theme)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.tab_list_menu = None;
                        this.run_command("workspace:toggle-stacked-tabs", window, cx);
                    }))
                    .child("Stack tabs"),
            );
        if !paths.is_empty() {
            let label = if paths.len() == 1 {
                "Bookmark 1 tab...".to_string()
            } else {
                format!("Bookmark {} tabs...", paths.len())
            };
            list = list.child(
                row("tab-list-bookmark".into(), "bookmark", theme)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.tab_list_menu = None;
                        this.start_bookmark(BookmarkDraft::Tabs(paths.clone()), window, cx);
                    }))
                    .child(label),
            );
        }
        list = list
            .child(separator(theme))
            .child(
                row("tab-list-close-all".into(), "x", theme)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.tab_list_menu = None;
                        this.close_group(group, true, cx);
                    }))
                    .child("Close all"),
            )
            .child(separator(theme));
        for (ix, tab) in view.tabs.iter().enumerate() {
            let checked = ix == active_ix;
            list = list.child(
                row(("tab-list-tab", ix).into(), "file", theme)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.tab_list_menu = None;
                        this.activate_group(group, cx);
                        this.activate_tab(ix, cx);
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .child(tab.title()),
                    )
                    .children(checked.then(|| icon("check", ICON_SIZE, theme.text_muted))),
            );
        }
        // * The wrapper lays the menu out below the button, so the offset
        //   is the gap under it.
        Some(
            deferred(
                anchored()
                    .offset(point(px(0.), px(2.)))
                    .snap_to_window_with_margin(px(8.))
                    .child(list),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }
}
