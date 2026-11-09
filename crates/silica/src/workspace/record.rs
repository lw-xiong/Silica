//! The `workspace.json` record: saving and restoring which tabs, groups
//! and layout were open, and the closed-tabs history used by reopen — the
//! methods of the split-out section, unchanged apart from living in their
//! own module. Split from `workspace.rs` per the audit's module-size
//! finding.

use super::*;

impl Workspace {
    /// Whether a `workspace.json` was read at load. The startup flags then
    /// leave the first note alone: with nothing recorded Obsidian shows the
    /// empty state rather than picking a note.
    pub fn layout_restored(&self) -> bool {
        self.layout_restored
    }

    /// Restores the layout `workspace.json` records: the main area's tab
    /// groups and how they are split, the active leaf, each sidebar's
    /// collapse state and its tab.
    pub(crate) fn restore_layout(
        &mut self,
        state: &Arc<VaultState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let layout = match state.vault().workspace_json() {
            Ok(Some(layout)) => layout,
            Ok(None) => return,
            Err(err) => {
                self.report(format!("workspace.json could not be read: {err}"), cx);
                return;
            }
        };
        self.restore_layout_value(layout, state, window, cx);
    }

    /// Puts a layout document on screen: the groups and their tabs, the
    /// sidebars, the open note. The vault's own `workspace.json` comes
    /// through here at startup; a named layout loaded out of
    /// `workspaces.json` comes through here the same way, whichever file
    /// it was read from.
    pub(crate) fn restore_layout_value(
        &mut self,
        layout: Value,
        state: &Arc<VaultState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.layout_restored = true;
        if let Some((sort, auto_reveal)) = explorer_leaf_state(&layout) {
            self.explorer
                .update(cx, |e, cx| e.set_leaf_state(sort, auto_reveal, cx));
        }
        self.restore_pane_states(&layout, cx);
        self.last_open_files = layout
            .get("lastOpenFiles")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let default_mode = self.default_mode();
        let spec = layout.get("main").and_then(|main| {
            // * A leaf naming a file the vault no longer has is dropped, the
            //   way Obsidian drops one it cannot open.
            read_main(main, default_mode, &|file| {
                state.vault().absolute(file).is_file()
            })
        });
        if let Some(spec) = spec {
            // * The groups the file describes replace the one the window
            //   opened with; its views were empty.
            let placeholder = self.active_group_id();
            let mut leaf_ids: Vec<(String, u64, usize)> = Vec::new();
            let tree = self.materialize(spec, &mut leaf_ids, window, cx);
            self.layout_tree = tree;
            if let Some(ix) = self.group_ix(placeholder) {
                self.groups.remove(ix);
                self.group_mru.retain(|g| *g != placeholder);
            }
            let active_id = layout.get("active").and_then(Value::as_str);
            let (group, tab) = leaf_ids
                .iter()
                .find(|(id, _, _)| Some(id.as_str()) == active_id)
                .map(|(_, g, t)| (*g, Some(*t)))
                .or_else(|| {
                    // * No active leaf on record: the first with a file.
                    leaf_ids
                        .iter()
                        .find(|(_, g, t)| {
                            self.group_ix(*g)
                                .is_some_and(|ix| self.groups[ix].tabs[*t].path.is_some())
                        })
                        .map(|(_, g, t)| (*g, Some(*t)))
                })
                .or_else(|| leaf_ids.first().map(|(_, g, _)| (*g, None)))
                .unwrap_or((self.groups[0].id, None));
            if let (Some(ix), Some(tab)) = (self.group_ix(group), tab) {
                self.groups[ix].active_tab = tab;
            }
            // * Fill every group's views, the active one last so its note is
            //   the one the explorer, graph and right dock follow.
            let mut order = Vec::new();
            self.layout_tree.groups(&mut order);
            for id in order.into_iter().filter(|id| *id != group).chain([group]) {
                if let Some(ix) = self.group_ix(id) {
                    self.active_group = ix;
                    self.load_group(ix, cx);
                    self.load_tab(cx);
                    self.stash_active_group();
                }
            }
            // * The most recently used order Obsidian would have: the active
            //   group first, the rest in layout order.
            self.group_mru = {
                let mut mru = vec![group];
                let mut order = Vec::new();
                self.layout_tree.groups(&mut order);
                mru.extend(order.into_iter().filter(|g| *g != group));
                mru
            };
        }
        let collapsed = |dock: &str| {
            layout
                .get(dock)
                .and_then(|d| d.get("collapsed"))
                .and_then(Value::as_bool)
                .unwrap_or(false)
        };
        self.left_visible = !collapsed("left");
        self.right_visible = !collapsed("right");
        // * A dock's `width` is where its handle was last dragged to; a
        //   fresh vault has 300, and nothing narrower than a header row's
        //   worth of buttons is believed.
        let width = |dock: &str| {
            layout
                .get(dock)
                .and_then(|d| d.get("width"))
                .and_then(Value::as_f64)
                .filter(|w| *w >= MIN_SIDEBAR_WIDTH)
        };
        if let Some(w) = width("left") {
            self.left_width = w;
        }
        if let Some(w) = width("right") {
            self.right_width = w;
        }
        match dock_current_view(&layout, "left").as_deref() {
            Some("search") => self.left_tab = LeftTab::Search,
            Some("bookmarks") => self.left_tab = LeftTab::Bookmarks,
            Some(_) => self.left_tab = LeftTab::Files,
            None => {}
        }
        match dock_current_view(&layout, "right").as_deref() {
            Some("backlink") => self.right_tab = RightTab::Backlinks,
            Some("outgoing-link") => self.right_tab = RightTab::Outgoing,
            Some("outline") => self.right_tab = RightTab::Outline,
            Some("graph") => self.right_tab = RightTab::Graph,
            Some("local-graph") => self.right_tab = RightTab::LocalGraph,
            Some("tag") => self.right_tab = RightTab::Tags,
            Some("all-properties") => self.right_tab = RightTab::Properties,
            Some("calendar") => self.right_tab = RightTab::Calendar,
            _ => {}
        }
        self.layout = Some(layout);
    }

    /// Makes the groups a `main` node describes: each with its own views,
    /// its tabs in place, not yet shown. `leaf_ids` collects every restored
    /// leaf's id with the group and tab it became, for `active`.
    pub(crate) fn materialize(
        &mut self,
        spec: LayoutSpec,
        leaf_ids: &mut Vec<(String, u64, usize)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Layout {
        match spec {
            LayoutSpec::Group(group) => {
                let mode = group
                    .tabs
                    .first()
                    .map(|(_, t)| t.mode)
                    .unwrap_or(ViewMode::Reading);
                let id = self.new_group(mode, window, cx);
                let ix = self.group_ix(id).expect("the group was just made");
                for (tab_ix, (leaf, _)) in group.tabs.iter().enumerate() {
                    leaf_ids.push((leaf.clone(), id, tab_ix));
                }
                let g = &mut self.groups[ix];
                g.tabs = group.tabs.into_iter().map(|(_, t)| t).collect();
                g.active_tab = group.active_tab.min(g.tabs.len().saturating_sub(1));
                g.node = group.node;
                Layout::Group(id)
            }
            LayoutSpec::Split {
                direction,
                children,
                node,
            } => Layout::Split {
                direction,
                children: children
                    .into_iter()
                    .map(|c| self.materialize(c, leaf_ids, window, cx))
                    .collect(),
                node,
            },
        }
    }

    /// The Backlinks and Outgoing links leaves' own state from
    /// `workspace.json`'s right dock, each key at Obsidian's default when
    /// it is missing or unreadable.
    pub(crate) fn restore_pane_states(&mut self, layout: &Value, cx: &mut Context<Self>) {
        let flag =
            |state: &Value, key: &str| state.get(key).and_then(Value::as_bool).unwrap_or(false);
        if let Some(state) = leaf_state(layout, "left", "search") {
            self.search.update(cx, |s, cx| s.restore(state, cx));
        }
        if let Some(state) = leaf_state(layout, "right", "backlink") {
            let b = &mut self.backlink_state;
            b.collapse_all = flag(state, "collapseAll");
            b.extra_context = flag(state, "extraContext");
            b.sort = state
                .get("sortOrder")
                .and_then(Value::as_str)
                .and_then(SortOrder::parse)
                .unwrap_or_default();
            b.show_search = flag(state, "showSearch");
            b.linked_collapsed = flag(state, "backlinkCollapsed");
            b.unlinked_collapsed = flag(state, "unlinkedCollapsed");
            let query = state
                .get("searchQuery")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            self.backlink_filter
                .update(cx, |input, cx| input.set_text(query, cx));
        }
        if let Some(state) = leaf_state(layout, "right", "outgoing-link") {
            self.outgoing_state.links_collapsed = flag(state, "linksCollapsed");
            self.outgoing_state.unlinked_collapsed = flag(state, "unlinkedCollapsed");
        }
        let sort = |state: &Value| {
            state
                .get("sortOrder")
                .and_then(Value::as_str)
                .and_then(PaneSort::parse)
                .unwrap_or_default()
        };
        let query = |state: &Value| {
            state
                .get("searchQuery")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        if let Some(state) = leaf_state(layout, "right", "tag") {
            self.tag_state.sort = sort(state);
            self.tag_state.use_hierarchy = state
                .get("useHierarchy")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            self.tag_state.show_search = flag(state, "showSearch");
            let q = query(state);
            self.tag_filter
                .update(cx, |input, cx| input.set_text(q, cx));
        }
        if let Some(state) = leaf_state(layout, "right", "all-properties") {
            self.property_state.sort = sort(state);
            self.property_state.show_search = flag(state, "showSearch");
            let q = query(state);
            self.property_filter
                .update(cx, |input, cx| input.set_text(q, cx));
        }
        if let Some(state) = leaf_state(layout, "right", "outline") {
            self.outline_state.follow_cursor = flag(state, "followCursor");
            self.outline_state.show_search = flag(state, "showSearch");
            let q = query(state);
            self.outline_filter
                .update(cx, |input, cx| input.set_text(q, cx));
        }
    }

    /// `workspace.json` as this session would leave it: the file that was
    /// read with the tab groups, the active leaf, the sidebars' collapse
    /// state and `lastOpenFiles` brought up to date, and every other key as
    /// it was.
    pub(crate) fn layout_json(&self, cx: &App) -> Value {
        let mut layout = self.layout.clone().unwrap_or_else(|| json!({}));
        let Some(object) = layout.as_object_mut() else {
            return layout;
        };
        // * A vault Obsidian never laid out has no docks in the file, or no
        //   file at all. Obsidian writes its default ones; so does Silica, so
        //   the panes' own state has a leaf to live in rather than being
        //   lost, with `main` first the way Obsidian orders the keys.
        if !object.contains_key("main") {
            object.insert("main".into(), Value::Null);
        }
        let (left_tab, right_tab) = (self.left_tab as usize, self.right_tab as usize);
        for (dock, tab) in [("left", left_tab), ("right", right_tab)] {
            if !object.get(dock).is_some_and(Value::is_object) {
                object.insert(
                    dock.into(),
                    default_dock_json(dock, self.current.as_deref(), tab),
                );
            }
        }
        // * The Files leaf's own state, where the file has that leaf.
        {
            let explorer = self.explorer.read(cx);
            let (sort, auto_reveal) = (explorer.sort_order(), explorer.auto_reveal());
            if let Some(left) = object.get_mut("left") {
                set_explorer_leaf_state(left, sort, auto_reveal);
                set_leaf_state(left, "search", &self.search.read(cx).leaf_state(cx));
            }
        }
        // * The Backlinks and Outgoing links leaves' own state, likewise.
        if let Some(right) = object.get_mut("right") {
            let b = &self.backlink_state;
            set_leaf_state(
                right,
                "backlink",
                &[
                    ("collapseAll", json!(b.collapse_all)),
                    ("extraContext", json!(b.extra_context)),
                    ("sortOrder", json!(b.sort.as_str())),
                    ("showSearch", json!(b.show_search)),
                    ("searchQuery", json!(self.backlink_filter.read(cx).text())),
                    ("backlinkCollapsed", json!(b.linked_collapsed)),
                    ("unlinkedCollapsed", json!(b.unlinked_collapsed)),
                ],
            );
            let o = &self.outgoing_state;
            set_leaf_state(
                right,
                "outgoing-link",
                &[
                    ("linksCollapsed", json!(o.links_collapsed)),
                    ("unlinkedCollapsed", json!(o.unlinked_collapsed)),
                ],
            );
            // * The Tags, All properties and Outline leaves' own state.
            let t = &self.tag_state;
            set_leaf_state(
                right,
                "tag",
                &[
                    ("sortOrder", json!(t.sort.as_str())),
                    ("useHierarchy", json!(t.use_hierarchy)),
                    ("showSearch", json!(t.show_search)),
                    ("searchQuery", json!(self.tag_filter.read(cx).text())),
                ],
            );
            let p = &self.property_state;
            set_leaf_state(
                right,
                "all-properties",
                &[
                    ("sortOrder", json!(p.sort.as_str())),
                    ("showSearch", json!(p.show_search)),
                    ("searchQuery", json!(self.property_filter.read(cx).text())),
                ],
            );
            // * `file` is the open note; Obsidian leaves the key out for an
            //   empty tab, and a `null` here removes it.
            let o = &self.outline_state;
            set_leaf_state(
                right,
                "outline",
                &[
                    ("file", json!(self.current)),
                    ("followCursor", json!(o.follow_cursor)),
                    ("showSearch", json!(o.show_search)),
                    ("searchQuery", json!(self.outline_filter.read(cx).text())),
                ],
            );
        }
        let mut active_id = Value::Null;
        let main = self.layout_node_json(&self.layout_tree, &mut active_id);
        // * `main` keeps its place in a file that had one.
        match object.get_mut("main") {
            Some(slot) => *slot = main,
            None => {
                object.insert("main".into(), main);
            }
        }
        object.insert("active".into(), active_id);
        for (dock, visible, width) in [
            ("left", self.left_visible, self.left_width),
            ("right", self.right_visible, self.right_width),
        ] {
            if let Some(d) = object.get_mut(dock).and_then(Value::as_object_mut) {
                if visible {
                    d.shift_remove("collapsed");
                } else {
                    d.insert("collapsed".into(), json!(true));
                }
                // * The width is rewritten only once a drag moved it, so the
                //   `300` Obsidian wrote stays the integer it was and a file
                //   without the key gains it only for a sidebar that moved.
                let same = d
                    .get("width")
                    .and_then(Value::as_f64)
                    .is_some_and(|w| (w - width).abs() < 0.5);
                if !same && (d.contains_key("width") || width != SIDEBAR_WIDTH as f64) {
                    d.insert("width".into(), json!(width));
                }
            }
        }
        object.insert("lastOpenFiles".into(), json!(self.last_open_files));
        layout
    }

    /// The layout tree for the socket's state dump: group ids at the leaves.
    pub(crate) fn layout_tree_json(&self, node: &Layout) -> Value {
        match node {
            Layout::Group(id) => json!({"group": id, "dimension": self.dimension_of(node)}),
            Layout::Split {
                direction,
                children,
                ..
            } => json!({
                "direction": direction.as_str(),
                "dimension": self.dimension_of(node),
                "children": children.iter().map(|c| self.layout_tree_json(c)).collect::<Vec<Value>>(),
            }),
        }
    }

    /// One node of `main`: a group as a `tabs` node with its leaves and
    /// `currentTab`, a split with its children and `direction`, each over
    /// the keys it was read with. The active group's active leaf id lands in
    /// `active_id` on the way, since a new tab's id exists only in the leaf
    /// written here.
    pub(crate) fn layout_node_json(&self, node: &Layout, active_id: &mut Value) -> Value {
        match node {
            Layout::Group(id) => {
                let Some(ix) = self.group_ix(*id) else {
                    return Value::Null;
                };
                let active = ix == self.active_group;
                let (tabs, active_tab) = if active {
                    (self.tab_snapshots(), self.active_tab)
                } else {
                    (self.groups[ix].tabs.clone(), self.groups[ix].active_tab)
                };
                let mut map = self.groups[ix].node.clone();
                if !map.contains_key("id") {
                    map.shift_insert(0, "id".into(), json!(leaf_id()));
                }
                map.insert("type".into(), json!("tabs"));
                let leaves: Vec<Value> = tabs.iter().map(leaf_json).collect();
                if active {
                    *active_id = leaves
                        .get(active_tab)
                        .and_then(|l| l.get("id"))
                        .cloned()
                        .unwrap_or(Value::Null);
                }
                map.shift_remove("children");
                map.shift_remove("currentTab");
                map.insert("children".into(), Value::Array(leaves));
                if active_tab > 0 {
                    map.insert("currentTab".into(), json!(active_tab));
                }
                Value::Object(map)
            }
            Layout::Split {
                direction,
                children,
                node,
            } => {
                let mut map = node.clone();
                if !map.contains_key("id") {
                    map.shift_insert(0, "id".into(), json!(leaf_id()));
                }
                map.insert("type".into(), json!("split"));
                map.shift_remove("children");
                map.shift_remove("direction");
                map.insert(
                    "children".into(),
                    Value::Array(
                        children
                            .iter()
                            .map(|c| self.layout_node_json(c, active_id))
                            .collect(),
                    ),
                );
                map.insert("direction".into(), json!(direction.as_str()));
                Value::Object(map)
            }
        }
    }

    /// Writes `workspace.json` back if the session changed the layout.
    /// Called on the way out, where Obsidian writes it.
    pub fn save_layout(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let fresh = self.layout_json(cx);
        if self.layout.as_ref() == Some(&fresh) {
            return;
        }
        if let Err(err) = state.vault().write_config("workspace.json", &fresh) {
            self.report(format!("workspace.json could not be written: {err}"), cx);
            return;
        }
        self.layout = Some(fresh);
    }

    /// Everything that has to reach the disk before the app goes away: the
    /// note's unsaved edits and the layout.
    pub fn on_quit(&mut self, cx: &mut Context<Self>) {
        self.flush_save(cx);
        if let Some(path) = self.current.clone() {
            self.save_folds(&path, cx);
        }
        self.save_layout(cx);
    }

    /// `silica:workspace-save`: asks what to call the layout on screen.
    /// The field has no suggestion list on purpose: Enter with a list
    /// open picks the marked row instead of what was typed, and this one
    /// takes the name as typed.
    pub(crate) fn save_workspace_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.is_none() {
            self.report("Workspace layouts: no vault is open", cx);
            return;
        }
        self.open_prompt(
            PendingOp::SaveWorkspace,
            PromptSpec {
                title: "Save workspace layout".into(),
                detail: Some(
                    "The panes and tabs open now are recorded under the name you type.".into(),
                ),
                input: Some(String::new()),
                placeholder: "Workspace name".into(),
                confirm: "Save".into(),
                ..PromptSpec::default()
            },
            window,
            cx,
        );
    }

    /// The name the prompt collected: record the layout the session would
    /// have written under it, or say why the name will not do and keep the
    /// prompt open.
    pub(crate) fn commit_workspace_save(
        &mut self,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(name) = crate::workspaces::normalized_name(name) else {
            self.prompt
                .update(cx, |p, cx| p.set_error("Give the workspace a name.", cx));
            return;
        };
        let Some(state) = self.state.clone() else {
            self.close_modal(window, cx);
            return;
        };
        let layout = self.layout_json(cx);
        match state.save_workspace(&name, &layout) {
            Ok(()) => {
                self.pending = None;
                self.close_modal(window, cx);
                self.report(format!("Saved workspace layout {name}"), cx);
            }
            Err(err) => self.prompt.update(cx, |p, cx| p.set_error(err, cx)),
        }
    }

    /// The prompt over the saved names: `silica:workspace-load` with Load
    /// as the answer, `silica:workspace-delete` with Delete. Enter takes
    /// the marked row, the way every other prompt with a list does.
    fn workspace_names_prompt(
        &mut self,
        title: &str,
        detail: &str,
        confirm: &str,
        op: PendingOp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.state.clone() else {
            self.report("Workspace layouts: no vault is open", cx);
            return;
        };
        let names: Vec<String> = match state.workspaces() {
            Ok(store) => store.names().iter().map(|n| n.to_string()).collect(),
            Err(err) => {
                self.report(format!("Workspace layouts could not be read: {err}"), cx);
                return;
            }
        };
        if names.is_empty() {
            self.report("No workspace layouts saved yet.", cx);
            return;
        }
        self.open_prompt(
            op,
            PromptSpec {
                title: title.into(),
                detail: Some(detail.into()),
                input: Some(String::new()),
                placeholder: "Workspace name".into(),
                confirm: confirm.into(),
                suggestions: names,
                ..PromptSpec::default()
            },
            window,
            cx,
        );
    }

    /// `silica:workspace-load`: the saved layouts, one Enter away.
    pub(crate) fn load_workspace_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.workspace_names_prompt(
            "Load workspace layout",
            "The panes and tabs switch to the layout you pick.",
            "Load",
            PendingOp::LoadWorkspace,
            window,
            cx,
        );
    }

    /// `silica:workspace-delete`: the saved layouts, with Delete as the
    /// answer.
    pub(crate) fn delete_workspace_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.workspace_names_prompt(
            "Delete workspace layout",
            "The layout you pick is taken out of the vault's workspaces.json.",
            "Delete",
            PendingOp::DeleteWorkspace,
            window,
            cx,
        );
    }

    /// The name the prompt picked: switch the session to that layout.
    pub(crate) fn commit_workspace_load(
        &mut self,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.state.clone() else {
            self.close_modal(window, cx);
            return;
        };
        let name = name.trim();
        let layout = match state.workspaces() {
            Ok(store) => match store.layout(name) {
                Some(layout) => layout,
                None => {
                    self.prompt.update(cx, |p, cx| {
                        p.set_error(format!("No saved workspace layout named {name}"), cx)
                    });
                    return;
                }
            },
            Err(err) => {
                self.prompt.update(cx, |p, cx| p.set_error(err, cx));
                return;
            }
        };
        self.pending = None;
        self.close_modal(window, cx);
        // * The note on screen keeps its edits: its buffer is flushed the
        //   way a quit would, before the leaves that hold it are replaced.
        self.flush_save(cx);
        self.restore_layout_value(layout, &state, window, cx);
        self.report(format!("Loaded workspace layout {name}"), cx);
    }

    /// The name the prompt picked: take that layout out of the vault.
    pub(crate) fn commit_workspace_delete(
        &mut self,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.state.clone() else {
            self.close_modal(window, cx);
            return;
        };
        match state.delete_workspace(name.trim()) {
            Ok(()) => {
                self.pending = None;
                self.close_modal(window, cx);
                self.report(format!("Deleted workspace layout {}", name.trim()), cx);
            }
            Err(err) => self.prompt.update(cx, |p, cx| p.set_error(err, cx)),
        }
    }
}
