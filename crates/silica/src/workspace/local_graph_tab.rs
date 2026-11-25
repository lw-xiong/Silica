//! The local graph as a tab: `graph:open-local` splits the main area to
//! the right with a group of its own holding a local-graph tab, linked
//! with the tab it came from, so the graph follows the note it is about.
//!
//! **The leaf.** A local-graph tab is one with no file and no history
//! whose leaf is a `localgraph` leaf: the file it centres on is in the
//! leaf's own `state`, and the link it shares with the note's tab is the
//! leaf's `group`, as for every other tab.
//!
//! **The title.** `Graph of <name>` while it has a note of its own,
//! `Local graph` when it has none.
//!
//! **The follow.** The graph centres on the file of the tab it is linked
//! with, so the note the reader moves to is the note the graph is about.
//! With no link left, the graph follows the note the main area shows.

use super::*;

/// The leaf's view type: a local graph of its own.
pub(crate) const VIEW_TYPE: &str = "localgraph";

/// The title a local graph carries with no note of its own.
pub(crate) const UNLINKED_TITLE: &str = "Local graph";

/// Whether `tab` is a local graph's.
pub(crate) fn is_local_graph(tab: &Tab) -> bool {
    tab.path.is_none()
        && tab.history.is_empty()
        && tab.mode != ViewMode::Graph
        && tab
            .leaf
            .as_ref()
            .and_then(|l| l.pointer("/state/type"))
            .and_then(Value::as_str)
            == Some(VIEW_TYPE)
}

/// The file a local graph's leaf centres on.
pub(crate) fn file_of(tab: &Tab) -> Option<String> {
    tab.leaf
        .as_ref()?
        .pointer("/state/state/file")?
        .as_str()
        .map(str::to_string)
}

/// The tab's title: `Graph of <name>` while it is linked to a note,
/// `Local graph` when it is not.
pub(crate) fn title(tab: &Tab) -> String {
    match (&tab.link, file_of(tab)) {
        (Some(_), Some(file)) => format!("Graph of {}", stem(&file)),
        _ => UNLINKED_TITLE.to_string(),
    }
}

/// A path's file name without its extension.
fn stem(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    match name.rfind('.') {
        Some(dot) if dot > 0 => &name[..dot],
        _ => name,
    }
}

/// A new local-graph leaf, centred on `file`.
pub(crate) fn new_leaf(file: Option<&str>) -> Value {
    let mut state = Map::new();
    if let Some(file) = file {
        state.insert("file".into(), json!(file));
    }
    json!({
        "id": leaf_id(),
        "type": "leaf",
        "state": {
            "type": VIEW_TYPE,
            "state": Value::Object(state),
            "icon": "lucide-git-fork",
            "title": UNLINKED_TITLE,
        },
    })
}

impl Workspace {
    /// `graph:open-local`: the local graph of the open note, in a group
    /// of its own beside it. The graph's tab is linked with the note's
    /// tab, so the graph follows the note; every run makes another group.
    pub(crate) fn open_linked_local_graph(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(file) = self.current.clone() else {
            return;
        };
        self.flush_save(cx);
        self.stash_active_group();
        let from = self.active_group_id();
        // * The note's tab joins a link, or keeps the one it has: the
        //   graph's tab takes the same id.
        let link = match self.tabs.get_mut(self.active_tab) {
            Some(tab) => match tab.link.clone() {
                Some(link) => link,
                None => {
                    let link = leaf_id();
                    tab.link = Some(link.clone());
                    link
                }
            },
            None => return,
        };
        let tab = Tab {
            path: None,
            mode: ViewMode::Reading,
            history: Vec::new(),
            history_ix: 0,
            pinned: false,
            leaf: Some(new_leaf(Some(&file))),
            link: Some(link),
            view_state: None,
        };
        // * The keyboard stays where it was: the reader keeps typing in
        //   the note, with the graph beside it.
        let focused = window.focused(cx);
        let id = self.new_group(ViewMode::Reading, window, cx);
        self.place_beside(from, id, SplitDirection::Vertical, true);
        let Some(ix) = self.group_ix(id) else {
            return;
        };
        self.groups[ix].tabs = vec![tab];
        self.groups[ix].active_tab = 0;
        self.load_group(ix, cx);
        self.sync_local_graph_tabs(cx);
        if let Some(handle) = focused {
            window.focus(&handle, cx);
            self.refocus_main = false;
        }
        cx.notify();
    }

    /// A node clicked in a local graph's tab: the file opens in the tab
    /// the graph is linked with, so the graph stays on screen.
    pub(crate) fn open_from_local_graph_tab(
        &mut self,
        group: u64,
        path: String,
        cx: &mut Context<Self>,
    ) {
        let link = self.group_ix(group).and_then(|gi| {
            self.groups[gi]
                .tabs
                .iter()
                .find(|tab| is_local_graph(tab))
                .and_then(|tab| tab.link.clone())
        });
        let target = self
            .layout_tabs()
            .into_iter()
            .find(|(_, _, tab)| tab.link == link && tab.path.is_some() && !is_local_graph(tab));
        if let Some((id, ix, _)) = target {
            self.activate_group(id, cx);
            self.activate_tab(ix, cx);
        }
        self.open_file(path, None, cx);
    }

    /// Brings every local-graph tab up to date: the file it centres on
    /// (the file of the tab it is linked with) and its title.
    pub(crate) fn sync_local_graph_tabs(&mut self, cx: &mut Context<Self>) {
        let tabs = self.layout_tabs();
        if !tabs.iter().any(|(_, _, t)| is_local_graph(t)) {
            return;
        }
        // * The file of each link: the note tab's file, not the graph's.
        let mut sources: HashMap<String, String> = HashMap::new();
        for (_, _, tab) in &tabs {
            if is_local_graph(tab) {
                continue;
            }
            if let (Some(link), Some(path)) = (&tab.link, &tab.path) {
                sources.entry(link.clone()).or_insert_with(|| path.clone());
            }
        }
        for (group, ix, tab) in &tabs {
            if !is_local_graph(tab) {
                continue;
            }
            let linked = tab
                .link
                .as_ref()
                .and_then(|link| sources.get(link))
                .cloned();
            // * A graph with no note of its own follows the note the main
            //   area shows; with none there either, it keeps what it had.
            let file = linked
                .clone()
                .or_else(|| self.current.clone())
                .or_else(|| file_of(tab));
            let title = match (&tab.link, &file) {
                (Some(_), Some(file)) => format!("Graph of {}", stem(file)),
                _ => UNLINKED_TITLE.to_string(),
            };
            if let Some(tab) = self.tab_slot(*group, *ix) {
                if let Some(leaf) = tab.leaf.as_mut() {
                    if let Some(state) = leaf
                        .pointer_mut("/state/state")
                        .and_then(Value::as_object_mut)
                    {
                        match &file {
                            Some(file) => {
                                state.insert("file".into(), json!(file));
                            }
                            None => {
                                state.shift_remove("file");
                            }
                        }
                    }
                    if let Some(state) = leaf.get_mut("state").and_then(Value::as_object_mut) {
                        state.insert("title".into(), json!(title));
                    }
                }
            }
            // * The graph in the group shows the file the tab is about.
            let live = self.groups[self.active_group].id == *group;
            let view = if live {
                self.graph_tab.clone()
            } else {
                match self.group_ix(*group) {
                    Some(gi) => self.groups[gi].graph_tab.clone(),
                    None => continue,
                }
            };
            view.update(cx, |g, cx| g.set_current(file, cx));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_local_graph_tab_is_told_by_its_leaf() {
        let tab = Tab {
            path: None,
            mode: ViewMode::Reading,
            history: Vec::new(),
            history_ix: 0,
            pinned: false,
            leaf: Some(new_leaf(Some("Notes/Board.md"))),
            link: Some("g".into()),
            view_state: None,
        };
        assert!(is_local_graph(&tab));
        assert_eq!(file_of(&tab).as_deref(), Some("Notes/Board.md"));
        assert_eq!(title(&tab), "Graph of Board");
    }

    #[test]
    fn a_tab_with_a_file_is_not_a_local_graph() {
        let tab = Tab {
            path: Some("Notes/Board.md".into()),
            mode: ViewMode::Reading,
            history: Vec::new(),
            history_ix: 0,
            pinned: false,
            leaf: Some(new_leaf(None)),
            link: None,
            view_state: None,
        };
        assert!(!is_local_graph(&tab));
    }

    #[test]
    fn a_graph_with_no_note_of_its_own_is_only_local_graph() {
        let tab = Tab {
            path: None,
            mode: ViewMode::Reading,
            history: Vec::new(),
            history_ix: 0,
            pinned: false,
            leaf: Some(new_leaf(Some("Notes/Board.md"))),
            link: None,
            view_state: None,
        };
        assert!(is_local_graph(&tab));
        assert_eq!(title(&tab), "Local graph");
    }
}
