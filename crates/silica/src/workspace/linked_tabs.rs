//! **Link with tab...** and **Unlink tab**: tabs that share a link follow
//! each other to the file one of them opens, as Obsidian 1.15 does them.
//!
//! **The id.** The link is the leaf's `group`: tabs whose leaf carries the
//! same id are linked, and the id is written back into `workspace.json`, so
//! the links survive a restart. A source joins the target's link, or both
//! take a new id when the target has none ([`joined_link`]).
//!
//! **The pick.** "Link with tab..." on a tab's menu starts a mode: the
//! source tab is marked, and the next tab clicked is linked with it. A tab
//! already linked offers "Unlink tab" instead, so a source is never
//! linked. Linking opens nothing.
//!
//! **Following.** When a linked tab opens a file, every other tab in its
//! link opens it too, pushing it onto its own history and keeping its own
//! mode ([`followers`]). An empty tab is left alone. A link left with one
//! tab is dissolved ([`lone_links`]), which is also what closing a tab of a
//! pair does.

use super::*;

use std::collections::{HashMap, HashSet};

/// The link a source joins: the target's, or a new id when it has none.
pub(crate) fn joined_link(target: Option<&str>, fresh: impl FnOnce() -> String) -> String {
    target.map(str::to_string).unwrap_or_else(fresh)
}

/// The links only one tab holds, which Obsidian dissolves: the tab left
/// behind loses its `group`.
pub(crate) fn lone_links<'a>(links: impl IntoIterator<Item = &'a str>) -> HashSet<String> {
    let mut count: HashMap<&str, usize> = HashMap::new();
    for link in links {
        *count.entry(link).or_default() += 1;
    }
    count
        .into_iter()
        .filter(|(_, n)| *n == 1)
        .map(|(l, _)| l.to_string())
        .collect()
}

/// The members of one link that follow `leader`, by index: every other
/// member showing a file (an empty tab does not follow) and not on the
/// leader's file already.
pub(crate) fn followers(files: &[Option<&str>], leader: usize) -> Vec<usize> {
    let Some(Some(to)) = files.get(leader) else {
        return Vec::new();
    };
    files
        .iter()
        .enumerate()
        .filter(|(ix, f)| *ix != leader && f.is_some_and(|f| f != *to))
        .map(|(ix, _)| ix)
        .collect()
}

/// The member whose file a restored link settles on, when its members
/// showing a file disagree: the last of them in layout order.
pub(crate) fn settled_leader(files: &[Option<&str>]) -> Option<usize> {
    let shown: Vec<(usize, &str)> = files
        .iter()
        .enumerate()
        .filter_map(|(ix, f)| f.map(|f| (ix, f)))
        .collect();
    let (last, file) = *shown.last()?;
    shown.iter().any(|(_, f)| *f != file).then_some(last)
}

impl Workspace {
    /// Every tab in the window's groups, with its group's id and index.
    pub(crate) fn layout_tabs(&self) -> Vec<(u64, usize, Tab)> {
        let mut out = Vec::new();
        for group in &self.groups {
            let tabs = if group.id == self.groups[self.active_group].id {
                self.tabs.clone()
            } else {
                group.tabs.clone()
            };
            out.extend(
                tabs.into_iter()
                    .enumerate()
                    .map(|(ix, tab)| (group.id, ix, tab)),
            );
        }
        out
    }

    /// The tab at `ix` of group `group`, live or stashed.
    pub(crate) fn tab_slot(&mut self, group: u64, ix: usize) -> Option<&mut Tab> {
        let active = self.groups[self.active_group].id == group;
        if active {
            self.tabs.get_mut(ix)
        } else {
            let gi = self.group_ix(group)?;
            self.groups[gi].tabs.get_mut(ix)
        }
    }

    /// "Link with tab...": the pick begins, with this tab as its source.
    pub(crate) fn start_link_pick(&mut self, group: u64, ix: usize, cx: &mut Context<Self>) {
        self.stash_active_group();
        self.linking = Some((group, ix));
        self.tab_context_menu = None;
        cx.notify();
    }

    /// "Unlink tab": the one tab out of its link. A link left with one tab
    /// is dissolved at the next sync.
    pub(crate) fn unlink_tab(&mut self, group: u64, ix: usize, cx: &mut Context<Self>) {
        self.stash_active_group();
        if let Some(tab) = self.tab_slot(group, ix) {
            tab.link = None;
        }
        // * The link may be left with one tab, which is dissolved.
        self.dissolve_lone_links();
        self.tab_context_menu = None;
        cx.notify();
    }

    /// The pick: the tab clicked is linked with the source. The source
    /// joins the target's link, or both take a new one.
    pub(crate) fn link_pick(&mut self, group: u64, ix: usize, cx: &mut Context<Self>) {
        let Some((source_group, source_ix)) = self.linking.take() else {
            return;
        };
        self.stash_active_group();
        if (source_group, source_ix) == (group, ix) {
            cx.notify();
            return;
        }
        let target_link = self.tab_slot(group, ix).and_then(|t| t.link.clone());
        let link = joined_link(target_link.as_deref(), leaf_id);
        if let Some(tab) = self.tab_slot(group, ix) {
            tab.link = Some(link.clone());
        }
        if let Some(tab) = self.tab_slot(source_group, source_ix) {
            tab.link = Some(link);
        }
        // * The tab picked is the one the reader meant: it is the one that
        //   comes to the front.
        self.activate_group(group, cx);
        self.activate_tab(ix, cx);
        cx.notify();
    }

    /// Links left with one tab are dissolved: the tab left behind loses
    /// its `group`.
    pub(crate) fn dissolve_lone_links(&mut self) {
        let tabs = self.layout_tabs();
        let lone = lone_links(tabs.iter().filter_map(|(_, _, t)| t.link.as_deref()));
        if lone.is_empty() {
            return;
        }
        for (group, ix, tab) in &tabs {
            if tab.link.as_ref().is_some_and(|l| lone.contains(l)) {
                if let Some(slot) = self.tab_slot(*group, *ix) {
                    slot.link = None;
                }
            }
        }
    }

    /// A restored layout whose linked tabs disagree is settled once, the
    /// later tab in the layout winning, as Obsidian's relaunch did.
    pub(crate) fn settle_restored_links(&mut self, cx: &mut Context<Self>) {
        let tabs = self.layout_tabs();
        let mut links: Vec<String> = Vec::new();
        for (_, _, tab) in &tabs {
            if let Some(link) = &tab.link {
                if !links.contains(link) {
                    links.push(link.clone());
                }
            }
        }
        let active_group = self.groups[self.active_group].id;
        let active_ix = self.active_tab;
        for link in links {
            let members: Vec<(u64, usize, Option<&str>)> = tabs
                .iter()
                .filter(|(_, _, t)| t.link.as_deref() == Some(link.as_str()))
                .map(|(g, i, t)| (*g, *i, t.path.as_deref()))
                .collect();
            let files: Vec<Option<&str>> = members.iter().map(|(_, _, f)| *f).collect();
            let Some(leader) = settled_leader(&files) else {
                continue;
            };
            let Some(to) = files[leader].map(str::to_string) else {
                continue;
            };
            for (ix, (group, tab_ix, _)) in members.iter().enumerate() {
                if ix == leader {
                    continue;
                }
                if *group == active_group && *tab_ix == active_ix {
                    self.open_file(to.clone(), None, cx);
                } else {
                    self.follow_in(*group, *tab_ix, &to, cx);
                }
            }
        }
    }

    /// The linked tabs that follow the active tab to the file it has just
    /// opened: every other tab of the link, in any group of the window,
    /// opens the same file in its own mode.
    pub(crate) fn follow_linked_tabs(&mut self, cx: &mut Context<Self>) {
        let Some(link) = self
            .tabs
            .get(self.active_tab)
            .and_then(|t| t.link.clone())
            .filter(|_| !self.linking.is_some())
        else {
            return;
        };
        let Some(path) = self.current.clone() else {
            return;
        };
        if self.mode == ViewMode::Graph {
            return;
        }
        let active_group = self.groups[self.active_group].id;
        let active_ix = self.active_tab;
        let tabs = self.layout_tabs();
        let members: Vec<(u64, usize, Option<&str>)> = tabs
            .iter()
            .filter(|(_, _, t)| t.link.as_deref() == Some(link.as_str()))
            .map(|(g, i, t)| (*g, *i, t.path.as_deref()))
            .collect();
        let files: Vec<Option<&str>> = members.iter().map(|(_, _, f)| *f).collect();
        let Some(at) = members
            .iter()
            .position(|(g, i, _)| *g == active_group && *i == active_ix)
        else {
            return;
        };
        for follower in followers(&files, at) {
            let (group, ix, _) = members[follower];
            self.follow_in(group, ix, &path, cx);
        }
    }

    /// Tab `ix` of group `group` opens `path` because a tab linked to it
    /// did: pushed onto its own history in its own mode. A tab not on
    /// screen moves on in its slot; the live one is left to the open that
    /// brought the workspace here.
    fn follow_in(&mut self, group: u64, ix: usize, path: &str, cx: &mut Context<Self>) {
        let active = self.groups[self.active_group].id == group;
        if active && ix == self.active_tab {
            return;
        }
        if let Some(tab) = self.tab_slot(group, ix) {
            tab.history.truncate(tab.history_ix);
            tab.history
                .push(HistoryEntry::new(path.to_string(), tab.mode));
            tab.history_ix = tab.history.len();
            tab.path = Some(path.to_string());
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_source_joins_the_targets_link_or_both_take_a_new_one() {
        assert_eq!(
            joined_link(Some("e47f88ecd2cab5de"), || unreachable!()),
            "e47f88ecd2cab5de"
        );
        assert_eq!(joined_link(None, || "fresh".into()), "fresh");
    }

    #[test]
    fn a_link_left_with_one_tab_is_dissolved() {
        let lone = lone_links(["a", "a", "b", "c", "c", "c"]);
        assert_eq!(lone, HashSet::from(["b".to_string()]));
        assert!(lone_links(["a", "a"]).is_empty());
    }

    #[test]
    fn a_restored_link_settles_on_its_last_tab() {
        // Obsidian's relaunch: Alpha before Gamma in the layout, both came
        // back on Gamma, whichever was active.
        assert_eq!(
            settled_leader(&[Some("Alpha.md"), Some("Gamma.md")]),
            Some(1)
        );
        assert_eq!(
            settled_leader(&[Some("Alpha.md"), Some("Gamma.md"), None]),
            Some(1)
        );
        assert_eq!(settled_leader(&[Some("A.md"), None, Some("A.md")]), None);
        assert_eq!(settled_leader(&[None, None]), None);
    }

    #[test]
    fn the_others_follow_the_tab_that_moved_but_an_empty_tab_does_not() {
        // Alpha moved to Beta; Gamma follows, the empty tab and a tab on
        // Beta already do not.
        let files = [Some("Beta.md"), Some("Gamma.md"), None, Some("Beta.md")];
        assert_eq!(followers(&files, 0), vec![1]);
        // Either way round.
        let files = [Some("Alpha.md"), Some("Delta.md")];
        assert_eq!(followers(&files, 1), vec![0]);
        // An empty leader leads nowhere.
        assert!(followers(&[None, Some("A.md")], 0).is_empty());
    }
}
