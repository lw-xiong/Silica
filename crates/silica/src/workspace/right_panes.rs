//! The right-dock panes: Tags, All properties, Outline, plus the filter
//! field, scrolling and context menus they share — the methods of the
//! split-out section, unchanged apart from living in their own module.
//! Split from `workspace.rs` per the audit's module-size finding.

use super::calendar::render_calendar_pane;
use super::*;

actions!(
    pane_keys,
    [
        PaneUp,
        PaneDown,
        PaneActivate,
        PaneLeaveFocus,
        PaneFoldLeft,
        PaneUnfoldRight
    ]
);

impl Workspace {
    /// Obsidian's `nav-header`: 40 px, 8 px of padding, the buttons
    /// centred 2 px apart.
    pub(crate) fn pane_header() -> gpui::Div {
        div()
            .flex_shrink_0()
            .h(px(HEADER_HEIGHT))
            .p(px(8.))
            .flex()
            .flex_row()
            .items_center()
            .justify_center()
            .gap(px(2.))
    }

    /// The filter field of a pane, by pane.
    pub(crate) fn pane_filter(&self, which: RightTab) -> &Entity<TextInput> {
        match which {
            RightTab::Tags => &self.tag_filter,
            RightTab::Properties => &self.property_filter,
            RightTab::Outline => &self.outline_filter,
            RightTab::Tasks => &self.task_filter,
            _ => &self.backlink_filter,
        }
    }

    pub(crate) fn pane_show_search(&mut self, which: RightTab) -> &mut bool {
        match which {
            RightTab::Tags => &mut self.tag_state.show_search,
            RightTab::Properties => &mut self.property_state.show_search,
            RightTab::Outline => &mut self.outline_state.show_search,
            RightTab::Tasks => &mut self.task_state.show_search,
            _ => &mut self.backlink_state.show_search,
        }
    }

    /// "Show search filter": shows the box and puts the keyboard in it, or
    /// hides it again, leaving the query (and the rows) as they were.
    pub(crate) fn toggle_pane_search(
        &mut self,
        which: RightTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let flag = self.pane_show_search(which);
        *flag = !*flag;
        if *flag {
            let handle = self.pane_filter(which).focus_handle(cx);
            window.focus(&handle, cx);
        }
        cx.notify();
    }

    /// Escape in the box: hides it and clears the query, so every row is
    /// back, and the keyboard goes to the note.
    pub(crate) fn hide_pane_search(
        &mut self,
        which: RightTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        *self.pane_show_search(which) = false;
        self.pane_filter(which)
            .update(cx, |input, cx| input.set_text("", cx));
        self.focus_main(window, cx);
        cx.notify();
    }

    /// The box's clear button: empties it and keeps the keyboard there.
    pub(crate) fn clear_pane_filter(
        &mut self,
        which: RightTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = self.pane_filter(which).clone();
        input.update(cx, |input, cx| input.set_text("", cx));
        window.focus(&input.focus_handle(cx), cx);
        cx.notify();
    }

    pub(crate) fn toggle_tag_hierarchy(&mut self, cx: &mut Context<Self>) {
        self.tag_state.use_hierarchy = !self.tag_state.use_hierarchy;
        // * Obsidian's tree comes back with every folder open.
        self.tag_state.collapsed.clear();
        self.tag_state.tag_fold_next = false;
        cx.notify();
    }

    /// "Expand all" (whose label never changes in Obsidian): folds every
    /// tag, leaves included, or opens them all again.
    pub(crate) fn toggle_tag_folds(&mut self, cx: &mut Context<Self>) {
        if self.tag_state.tag_fold_next {
            self.tag_state.collapsed = panes::tag_keys(&self.tag_tree).into_iter().collect();
        } else {
            self.tag_state.collapsed.clear();
        }
        self.tag_state.tag_fold_next = !self.tag_state.tag_fold_next;
        cx.notify();
    }

    pub(crate) fn toggle_tag_fold(&mut self, key: String, cx: &mut Context<Self>) {
        if !self.tag_state.collapsed.remove(&key) {
            self.tag_state.collapsed.insert(key);
        }
        cx.notify();
    }

    pub(crate) fn set_pane_sort(
        &mut self,
        which: RightTab,
        order: PaneSort,
        cx: &mut Context<Self>,
    ) {
        match which {
            RightTab::Tags => {
                self.tag_state.sort = order;
                self.tag_state.sort_menu_open = false;
                panes::sort_tags(&mut self.tag_tree, order);
            }
            _ => {
                self.property_state.sort = order;
                self.property_state.sort_menu_open = false;
            }
        }
        cx.notify();
    }

    pub(crate) fn toggle_outline_fold(&mut self, ix: usize, cx: &mut Context<Self>) {
        if !self.outline_state.collapsed.remove(&ix) {
            self.outline_state.collapsed.insert(ix);
        }
        cx.notify();
    }

    /// The headings that have children: what "Collapse all" folds.
    pub(crate) fn outline_parents(&self) -> Vec<usize> {
        let depths = panes::outline_depths(&self.outline);
        (0..depths.len())
            .filter(|&ix| depths.get(ix + 1).is_some_and(|&d| d > depths[ix]))
            .collect()
    }

    /// The top-level headings with children: what "Collapse all" folds,
    /// leaving the folds under them as they were, and what "Expand all"
    /// counts.
    pub(crate) fn outline_top_parents(&self) -> Vec<usize> {
        let depths = panes::outline_depths(&self.outline);
        self.outline_parents()
            .into_iter()
            .filter(|&ix| depths[ix] == 0)
            .collect()
    }

    /// Whether every top-level heading with children is folded, which is
    /// when the button reads "Expand all".
    pub(crate) fn outline_all_folded(&self) -> bool {
        let parents = self.outline_top_parents();
        !parents.is_empty()
            && parents
                .iter()
                .all(|ix| self.outline_state.collapsed.contains(ix))
    }

    /// "Collapse all" folds the top-level headings and leaves the deeper
    /// folds alone; "Expand all" opens everything.
    pub(crate) fn toggle_outline_folds(&mut self, cx: &mut Context<Self>) {
        if self.outline_all_folded() {
            self.outline_state.collapsed.clear();
        } else {
            let top = self.outline_top_parents();
            self.outline_state.collapsed.extend(top);
        }
        cx.notify();
    }

    pub(crate) fn toggle_follow_cursor(&mut self, cx: &mut Context<Self>) {
        self.outline_state.follow_cursor = !self.outline_state.follow_cursor;
        self.outline_state.scroll_pending = true;
        cx.notify();
    }

    /// An Outline row's click: in an editing mode the cursor goes to the
    /// heading's line, column 0, scrolled to the vertical centre, and the
    /// editor takes the keyboard; in reading mode the heading scrolls to
    /// the top edge.
    pub(crate) fn reveal_heading(&mut self, offset: usize, cx: &mut Context<Self>) {
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

    /// The heading whose section holds the cursor (editing) or the block at
    /// the top of the view (reading); none above the first heading.
    pub(crate) fn current_heading(&self, cx: &App) -> Option<usize> {
        let path = self.current.as_deref()?;
        let offset = match self.mode {
            ViewMode::Reading => self.reading.read(cx).top_offset()?,
            mode if mode.is_editing() => {
                let editor = self.editor.read(cx);
                if editor.name.as_ref() != path {
                    return None;
                }
                editor.selection().start
            }
            _ => return None,
        };
        panes::heading_at(&self.outline, offset)
    }

    /// Brings the Outline's mark up to date when the cursor or the reading
    /// view moved, and asks for a draw only when the mark moved.
    pub(crate) fn follow_heading(&mut self, cx: &mut Context<Self>) {
        if !self.right_visible || self.right_tab != RightTab::Outline {
            return;
        }
        let active = self.current_heading(cx);
        if active != self.outline_state.active {
            self.outline_state.active = active;
            self.outline_state.scroll_pending = true;
            cx.notify();
        }
    }

    /// The Tags pane's rows as drawn now.
    pub(crate) fn tag_rows(&self, cx: &App) -> Vec<panes::TagRow> {
        panes::tag_rows(
            &self.tag_tree,
            self.tag_state.sort,
            self.tag_state.use_hierarchy,
            &self.tag_state.collapsed,
            self.tag_filter.read(cx).text(),
        )
    }

    pub(crate) fn property_rows(&self, cx: &App) -> Vec<panes::PropertyRow> {
        panes::property_rows(
            &self.property_entries,
            self.property_state.sort,
            self.property_filter.read(cx).text(),
        )
    }

    pub(crate) fn outline_rows(&self, cx: &App) -> Vec<panes::OutlineRow> {
        panes::outline_rows(
            &self.outline,
            &self.outline_state.collapsed,
            self.outline_filter.read(cx).text(),
            self.outline_state.active,
        )
    }

    /// The search box under a pane's header: Obsidian's
    /// `search-input-container`, 30 px tall with 4 px above and below it
    /// inside the header, so the rows move down 38 px, and a round clear
    /// button at its right end once there is text.
    pub(crate) fn render_pane_filter(
        &self,
        which: RightTab,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let input = self.pane_filter(which).clone();
        let has_text = !input.read(cx).text().is_empty();
        // * Obsidian's `search-input-container`: the field's text starts
        //   30 px in, behind a magnifier at the left.
        div().flex_shrink_0().px(px(8.)).pb(px(8.)).child(
            div()
                .relative()
                .w_full()
                .h(px(30.))
                .child(input)
                .child(
                    div()
                        .absolute()
                        .left(px(8.))
                        .top(px(0.))
                        .h(px(30.))
                        .flex()
                        .items_center()
                        .child(icon("search", 14., theme.text_muted)),
                )
                .children(has_text.then(|| {
                    div()
                        .id(("pane-filter-clear", which as usize))
                        .absolute()
                        .right(px(2.))
                        .top(px(0.))
                        .w(px(28.))
                        .h(px(30.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .rounded_full()
                        .tooltip(tooltip("Clear search"))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.clear_pane_filter(which, window, cx)
                        }))
                        .child(icon("x", 14., theme.text_muted))
                })),
        )
    }

    /// "Change sort order" for the Tags and All properties panes: the two
    /// name orders, a separator, the two frequency orders, a check against
    /// the one in force.
    pub(crate) fn render_pane_sort_menu(
        &mut self,
        which: RightTab,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let (open, current, noun, id) = match which {
            RightTab::Tags => (
                self.tag_state.sort_menu_open,
                self.tag_state.sort,
                "Tag name",
                "tags-sort-menu",
            ),
            _ => (
                self.property_state.sort_menu_open,
                self.property_state.sort,
                "Property name",
                "properties-sort-menu",
            ),
        };
        if !open {
            return None;
        }
        let mut list = div()
            .id(id)
            .flex()
            .flex_col()
            .min_w(px(196.))
            .py_1()
            .bg(theme.bg_secondary)
            .border_1()
            .border_color(theme.border)
            .rounded_md()
            .shadow_lg()
            .text_size(px(13.))
            .text_color(theme.text)
            .occlude()
            .on_mouse_down_out(cx.listener(move |this, _, _, cx| {
                match which {
                    RightTab::Tags => this.tag_state.sort_menu_open = false,
                    _ => this.property_state.sort_menu_open = false,
                }
                cx.notify();
            }));
        for (ix, order) in PaneSort::ALL.into_iter().enumerate() {
            if ix == 2 {
                list = list.child(div().my_1().h(px(1.)).bg(theme.border));
            }
            let checked = order == current;
            list = list.child(
                div()
                    .id((id, ix))
                    .h(px(25.))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.bg_alt))
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.set_pane_sort(which, order, cx)),
                    )
                    .child(
                        div()
                            .w(px(16.))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .children(checked.then(|| icon("check", 14., theme.text_muted))),
                    )
                    .child(SharedString::from(order.label(noun))),
            );
        }
        Some(
            deferred(
                anchored()
                    .position(self.sort_menu_at)
                    .anchor(Anchor::TopRight)
                    .snap_to_window_with_margin(px(8.))
                    .child(list),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }

    /// Opens the side-pane context menu at the pointer's position, as
    /// Obsidian's right-click on a Tags/All properties/Outline row does.
    pub(crate) fn open_side_menu(
        &mut self,
        position: gpui::Point<Pixels>,
        target: SidePaneTarget,
        cx: &mut Context<Self>,
    ) {
        self.side_menu_at = position;
        self.side_menu = Some(SidePaneMenu { position, target });
        cx.notify();
    }

    /// Closes the open context menu, as clicking outside one does.
    pub(crate) fn close_side_menu(&mut self, cx: &mut Context<Self>) {
        if self.side_menu.take().is_some() {
            cx.notify();
        }
    }

    /// The right-click context menu over a side-pane row.
    pub(crate) fn render_side_menu(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let menu = self.side_menu.as_ref()?;
        let items = side_menu_items(&menu.target);
        let mut list = div()
            .id("side-menu")
            .flex()
            .flex_col()
            .min_w(px(196.))
            .py_1()
            .bg(theme.bg_secondary)
            .border_1()
            .border_color(theme.border)
            .rounded_md()
            .shadow_lg()
            .text_size(px(13.))
            .text_color(theme.text)
            .occlude()
            .on_mouse_down_out(cx.listener(move |this, _, _, cx| {
                this.close_side_menu(cx);
            }));
        for (ix, item) in items.into_iter().enumerate() {
            if ix > 0 {
                list = list.child(div().my_1().h(px(1.)).bg(theme.border));
            }
            list = list.child(
                div()
                    .id(("side-menu-item", ix))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_1()
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.bg_alt))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.close_side_menu(cx);
                        this.handle_side_menu_action(&item.action, window, cx);
                        cx.notify();
                    }))
                    .child(crate::assets::icon(item.icon, 16., theme.text_muted))
                    .child(item.label),
            );
        }
        Some(
            deferred(
                anchored()
                    .position(menu.position)
                    .anchor(Anchor::TopLeft)
                    .snap_to_window_with_margin(px(8.))
                    .child(list),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }

    /// Handles a side-pane context menu action: searches for tags/properties,
    /// copies paths, or opens outline headings.
    pub(crate) fn handle_side_menu_action(
        &mut self,
        action: &SidePaneAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            SidePaneAction::SearchTag(tag) => {
                self.show_search(&format!("tag:#{tag}"), None, cx);
            }
            SidePaneAction::RenameTag(tag) => {
                // * The prompt asks for the new tag name; the rename itself
                //   rewrites every occurrence across the vault on submit.
                let Some(state) = self.state.clone() else {
                    return;
                };
                let tag_lower = tag.to_lowercase();
                let count = state
                    .cache()
                    .tags()
                    .get(&tag_lower)
                    .map(|e| e.count)
                    .unwrap_or(0);
                self.open_prompt(
                    PendingOp::RenameTag { from: tag.clone() },
                    PromptSpec {
                        title: "Rename tag".into(),
                        detail: Some(
                            format!("Rename \"#{tag}\" to what? ({count} occurrence{} across the vault)", if count == 1 { "" } else { "s" }).into(),
                        ),
                        input: None,
                        placeholder: "Tag name".into(),
                        confirm: "Rename".into(),
                        ..PromptSpec::default()
                    },
                    window,
                    cx,
                );
            }
            SidePaneAction::CopyTag(tag) => {
                self.copy_path(tag, cx);
            }
            SidePaneAction::SearchProperty(key) => {
                self.show_search(&format!("[\"{key}\"]"), None, cx);
            }
            SidePaneAction::CopyProperty(key) => {
                self.copy_path(key, cx);
            }
            SidePaneAction::RenameProperty(key) => {
                let count = self
                    .property_entries
                    .iter()
                    .find(|e| e.name == *key)
                    .map(|e| e.count)
                    .unwrap_or(0);
                let s = if count == 1 { "" } else { "s" };
                self.open_prompt(
                    PendingOp::RenameProperty { from: key.clone() },
                    PromptSpec {
                        title: crate::i18n::t("Rename property").into(),
                        detail: Some(
                            format!(
                                "Rename \"{key}\" to what? ({count} note{s} with this property)",
                                s = s
                            )
                            .into(),
                        ),
                        input: None,
                        placeholder: crate::i18n::t("Property name").into(),
                        confirm: "Rename".into(),
                        ..PromptSpec::default()
                    },
                    window,
                    cx,
                );
            }
            SidePaneAction::SetPropertyType(key) => {
                let Some(state) = self.state.clone() else {
                    return;
                };
                let current = state
                    .property_types()
                    .types
                    .get(key)
                    .cloned()
                    .unwrap_or_default();
                let suggestions: Vec<String> =
                    silica_formats::properties::PropertyType::selectable()
                        .iter()
                        .map(|ty| ty.as_str().to_string())
                        .collect();
                self.open_prompt(
                    PendingOp::SetPropertyType { key: key.clone() },
                    PromptSpec {
                        title: crate::i18n::t("Property type").into(),
                        detail: Some(format!("Set the type of \"{key}\"").into()),
                        input: None,
                        placeholder: current.into(),
                        suggestions,
                        confirm: "Set".into(),
                        ..PromptSpec::default()
                    },
                    window,
                    cx,
                );
            }
            SidePaneAction::DeleteProperty(key) => {
                let count = self
                    .property_entries
                    .iter()
                    .find(|e| e.name == *key)
                    .map(|e| e.count)
                    .unwrap_or(0);
                let s = if count == 1 { "" } else { "s" };
                self.open_prompt(
                    PendingOp::DeleteProperty { key: key.clone() },
                    PromptSpec {
                        title: crate::i18n::t("Delete property").into(),
                        detail: Some(
                            format!("Delete \"{key}\" from {count} note{s}", s = s).into(),
                        ),
                        input: None,
                        placeholder: "".into(),
                        confirm: crate::i18n::t("Delete").into(),
                        ..PromptSpec::default()
                    },
                    window,
                    cx,
                );
            }
            SidePaneAction::OpenHeading(path, offset) => {
                self.open_file(path.clone(), None, cx);
                self.reveal_offset(*offset, cx);
            }
            SidePaneAction::OpenHeadingInNewTab(path) => {
                self.open_in_new_tab(path.clone(), None, cx);
            }
            SidePaneAction::OpenHeadingToRight(path) => {
                self.open_in_new_tab(path.clone(), None, cx);
            }
            SidePaneAction::OpenLink(path) => {
                self.open_file(path.clone(), None, cx);
            }
            SidePaneAction::OpenLinkInNewTab(path) => {
                self.open_in_new_tab(path.clone(), None, cx);
            }
            SidePaneAction::OpenLinkToRight(path) => {
                self.open_in_new_tab(path.clone(), None, cx);
            }
            SidePaneAction::CopyLinkPath(path) => {
                self.copy_path(path, cx);
            }
            // * The file menu's own items: each one opens the prompt the
            //   workspace already asks with, so a row's Rename... is the
            //   same rename with the same link rewriting the command
            //   behind it would do.
            SidePaneAction::RenameFile(path) => {
                self.start_rename(path, window, cx);
            }
            SidePaneAction::MoveFile(path) => {
                self.start_move(path, window, cx);
            }
            SidePaneAction::BookmarkFile(path) => {
                let draft = BookmarkDraft::File {
                    path: path.clone(),
                    subpath: None,
                };
                self.start_bookmark(draft, window, cx);
            }
            SidePaneAction::DeleteFile(path) => {
                self.start_delete(path, window, cx);
            }
        }
    }

    /// Obsidian's `tree-item-self`: 25 px tall at a 27 px pitch,
    /// `padding: 4px 8px 4px 24px` with 17 px more on the left per level of
    /// depth, an 8 px radius, 13 px muted text that brightens under the
    /// pointer (or when `active`, the Outline's `is-active` mark), a faint
    /// flair at the right edge, a fold triangle or an icon in the gutter
    /// 20 px before the text, and an indentation guide for every level.
    /// A `wrap` label wraps at 17 px per line; the others are cut with an
    /// ellipsis.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn tree_row(
        id: (&'static str, usize),
        depth: usize,
        fold: Option<(bool, FoldAction)>,
        icon_name: Option<&'static str>,
        label: gpui::AnyElement,
        flair: Option<SharedString>,
        active: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
        on_right_click: Option<RightClickHandler<Self>>,
    ) -> gpui::Stateful<gpui::Div> {
        let text_x = 24. + 17. * depth as f32;
        let gutter = match (fold, icon_name) {
            (Some((collapsed, on_fold)), _) => Some(
                div()
                    .id((id.0, id.1 + 1_000_000))
                    .absolute()
                    .left(px(text_x - 20.))
                    .top(px(4.))
                    .w(px(16.))
                    .h(px(17.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .opacity(0.85)
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        on_fold(this, cx);
                        cx.stop_propagation();
                    }))
                    .child(
                        div()
                            .text_color(theme.text_faint)
                            .group_hover("tree-row", |s| s.text_color(theme.text_muted))
                            .child(icon(
                                if collapsed {
                                    "chevron-right"
                                } else {
                                    "chevron-down"
                                },
                                10.,
                                theme.text_faint,
                            )),
                    ),
            ),
            (None, Some(name)) => Some(
                div()
                    .id((id.0, id.1 + 1_000_000))
                    .absolute()
                    .left(px(text_x - 20.))
                    .top(px(4.))
                    .w(px(16.))
                    .h(px(17.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .opacity(0.85)
                    .child(icon(name, 14., theme.text_muted)),
            ),
            (None, None) => None,
        };
        div()
            .id(id)
            .group("tree-row")
            .relative()
            .w_full()
            .flex_shrink_0()
            .min_h(px(25.))
            .mb(px(2.))
            .pl(px(text_x))
            .pr(px(8.))
            .py(px(4.))
            .flex()
            .flex_row()
            .items_start()
            .rounded(px(8.))
            .cursor_pointer()
            .when(active, |d| d.bg(theme.bg_alt))
            .hover(|s| s.bg(theme.bg_alt))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                    if let Some(on_right_click) = &on_right_click {
                        cx.stop_propagation();
                        on_right_click(this, event.position, cx);
                    }
                }),
            )
            // * Obsidian's `tree-item-children` guides: a hairline 12 px in
            //   from each ancestor's text, the row's full pitch tall.
            .children((1..=depth).map(|k| {
                div()
                    .absolute()
                    .left(px(17. * k as f32 - 5.))
                    .top(px(0.))
                    .bottom(px(-2.))
                    .w(px(1.))
                    .bg(theme.border)
            }))
            .children(gutter)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(FONT_UI_SMALL))
                    .line_height(px(17.))
                    .text_color(if active { theme.text } else { theme.text_muted })
                    .group_hover("tree-row", |s| s.text_color(theme.text))
                    .child(label),
            )
            .children(flair.map(|count| {
                div()
                    .flex_shrink_0()
                    .pl(px(4.))
                    .pt(px(3.))
                    .text_size(px(FONT_UI_SMALLER))
                    .line_height(px(12.))
                    .text_color(theme.text_faint)
                    .group_hover("tree-row", |s| s.text_color(theme.text_muted))
                    .child(count)
            }))
    }

    /// A one-line label cut with an ellipsis.
    pub(crate) fn tree_label(text: impl Into<SharedString>) -> gpui::AnyElement {
        div()
            .whitespace_nowrap()
            .overflow_hidden()
            .text_ellipsis()
            .child(text.into())
            .into_any_element()
    }

    /// The Tags pane: sort, nested, expand-all and search buttons, the
    /// filter when shown, then the tags as a tree (or a flat list) with
    /// their counts; a row's click searches `tag:#path`.
    pub(crate) fn render_tags_pane(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let (hierarchy, show_search) = (self.tag_state.use_hierarchy, self.tag_state.show_search);
        let sort_menu = self.render_pane_sort_menu(RightTab::Tags, theme, cx);
        let header = Self::pane_header()
            .child(
                div()
                    .relative()
                    .child(
                        Self::nav_button(
                            "tags-sort",
                            "arrow-up-narrow-wide",
                            "Change sort order",
                            false,
                            theme,
                        )
                        .on_click(cx.listener(
                            |this, event: &ClickEvent, _, cx| {
                                this.sort_menu_at = event.position();
                                this.tag_state.sort_menu_open = !this.tag_state.sort_menu_open;
                                cx.notify();
                            },
                        )),
                    )
                    .children(sort_menu),
            )
            .child(
                Self::nav_button(
                    "tags-nested",
                    "folder-tree",
                    "Show nested tags",
                    hierarchy,
                    theme,
                )
                .on_click(cx.listener(|this, _, _, cx| this.toggle_tag_hierarchy(cx))),
            )
            .child(
                Self::nav_button(
                    "tags-expand",
                    "chevrons-up-down",
                    "Expand all",
                    false,
                    theme,
                )
                .on_click(cx.listener(|this, _, _, cx| this.toggle_tag_folds(cx))),
            )
            .child(
                Self::nav_button(
                    "tags-search",
                    "search",
                    "Show search filter",
                    show_search,
                    theme,
                )
                .on_click(cx.listener(|this, _, window, cx| {
                    this.toggle_pane_search(RightTab::Tags, window, cx)
                })),
            );
        let filter = show_search.then(|| self.render_pane_filter(RightTab::Tags, theme, cx));
        let rows = self.tag_rows(cx);
        // * Obsidian's `.tag-container`: padding 12px 12px 32px.
        let mut list = div()
            .id("tags")
            .flex()
            .flex_col()
            .flex_1()
            .overflow_y_scroll()
            .px(px(12.))
            .pt(px(12.))
            .pb(px(32.));
        for (ix, row) in rows.into_iter().enumerate() {
            let path = row.path.clone();
            let path_for_menu = path.clone();
            let fold = row.has_children.then(|| {
                let key = row.key.clone();
                let on_fold: FoldAction =
                    Box::new(move |this, cx| this.toggle_tag_fold(key.clone(), cx));
                (row.collapsed, on_fold)
            });
            // * The parent part shows only in flat mode, in the same colour.
            let label = if hierarchy {
                row.name.clone()
            } else {
                row.path.clone()
            };
            list = list.child(
                Self::tree_row(
                    ("tag-row", ix),
                    row.depth,
                    fold,
                    None,
                    Self::tree_label(label),
                    Some(row.count.to_string().into()),
                    self.tag_state.selected == Some(ix),
                    theme,
                    cx,
                    Some(Box::new(move |this, position, cx| {
                        this.open_side_menu(
                            position,
                            SidePaneTarget::Tag(path_for_menu.clone()),
                            cx,
                        );
                    })),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.show_search(&format!("tag:#{path}"), Some(window), cx);
                    this.focus_panes(window, cx);
                })),
            );
        }
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(header)
            .children(filter)
            .child(list)
            .into_any_element()
    }

    /// The All properties pane: sort and search buttons, the filter when
    /// shown, then one row per key with its widget's icon and the notes
    /// carrying it; a row's click searches `["key"]`.
    pub(crate) fn render_properties_pane(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let show_search = self.property_state.show_search;
        let sort_menu = self.render_pane_sort_menu(RightTab::Properties, theme, cx);
        let header = Self::pane_header()
            .child(
                div()
                    .relative()
                    .child(
                        Self::nav_button(
                            "properties-sort",
                            "arrow-up-narrow-wide",
                            "Change sort order",
                            false,
                            theme,
                        )
                        .on_click(cx.listener(
                            |this, event: &ClickEvent, _, cx| {
                                this.sort_menu_at = event.position();
                                this.property_state.sort_menu_open =
                                    !this.property_state.sort_menu_open;
                                cx.notify();
                            },
                        )),
                    )
                    .children(sort_menu),
            )
            .child(
                Self::nav_button(
                    "properties-search",
                    "search",
                    "Show search filter",
                    show_search,
                    theme,
                )
                .on_click(cx.listener(|this, _, window, cx| {
                    this.toggle_pane_search(RightTab::Properties, window, cx)
                })),
            );
        let filter = show_search.then(|| self.render_pane_filter(RightTab::Properties, theme, cx));
        let rows = self.property_rows(cx);
        // * Obsidian's `.view-content`: padding 4px 12px 32px.
        let mut list = div()
            .id("properties")
            .flex()
            .flex_col()
            .flex_1()
            .overflow_y_scroll()
            .px(px(12.))
            .pt(px(4.))
            .pb(px(32.));
        for (ix, row) in rows.into_iter().enumerate() {
            let name = row.name.clone();
            let name_for_menu = name.clone();
            list = list.child(
                Self::tree_row(
                    ("property-row", ix),
                    0,
                    None,
                    Some(row.icon),
                    Self::tree_label(name.clone()),
                    Some(row.count.to_string().into()),
                    self.property_state.selected == Some(ix),
                    theme,
                    cx,
                    Some(Box::new(move |this, position, cx| {
                        this.open_side_menu(
                            position,
                            SidePaneTarget::Property(name_for_menu.clone()),
                            cx,
                        );
                    })),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.show_search(&format!("[\"{name}\"]"), Some(window), cx);
                    this.focus_panes(window, cx);
                })),
            );
        }
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(header)
            .children(filter)
            .child(list)
            .into_any_element()
    }

    /// The Outline pane: search, auto-scroll and collapse-all buttons, the
    /// filter when shown, then the headings as a structural tree with the
    /// one holding the cursor marked, or "No headings found.".
    pub(crate) fn render_outline_pane(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        // * The mark follows the cursor and the reading view; a mode or note
        //   switch lands here before anything else can notice it.
        let active = self.current_heading(cx);
        if active != self.outline_state.active {
            self.outline_state.active = active;
            self.outline_state.scroll_pending = true;
        }
        let (follow, show_search) = (
            self.outline_state.follow_cursor,
            self.outline_state.show_search,
        );
        let all_folded = self.outline_all_folded();
        let header = Self::pane_header()
            .child(
                Self::nav_button(
                    "outline-search",
                    "search",
                    "Show search filter",
                    show_search,
                    theme,
                )
                .on_click(cx.listener(|this, _, window, cx| {
                    this.toggle_pane_search(RightTab::Outline, window, cx)
                })),
            )
            .child(
                Self::nav_button(
                    "outline-follow",
                    "gallery-vertical",
                    "Auto-scroll to current section",
                    follow,
                    theme,
                )
                .on_click(cx.listener(|this, _, _, cx| this.toggle_follow_cursor(cx))),
            )
            .child(
                if all_folded {
                    Self::nav_button(
                        "outline-fold",
                        "chevrons-up-down",
                        "Expand all",
                        false,
                        theme,
                    )
                } else {
                    Self::nav_button(
                        "outline-fold",
                        "chevrons-down-up",
                        "Collapse all",
                        false,
                        theme,
                    )
                }
                .on_click(cx.listener(|this, _, _, cx| this.toggle_outline_folds(cx))),
            );
        let filter = show_search.then(|| self.render_pane_filter(RightTab::Outline, theme, cx));
        let body: gpui::AnyElement = if self.outline.is_empty() {
            // * Obsidian's `.pane-empty`: 13 px faint, padding 24px 16px,
            //   margin 8px 0, centred.
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
                        .child("No headings found."),
                )
                .into_any_element()
        } else {
            let rows = self.outline_rows(cx);
            if follow && self.outline_state.scroll_pending {
                self.outline_state.scroll_pending = match rows.iter().position(|r| r.active) {
                    Some(ix) => !scroll_outline_to(&self.outline_state.scroll, ix),
                    None => false,
                };
            }
            let mut list = div()
                .id("outline")
                .flex()
                .flex_col()
                .flex_1()
                .overflow_y_scroll()
                .track_scroll(&self.outline_state.scroll)
                .px(px(12.))
                .pt(px(4.))
                .pb(px(32.));
            for (ix, row) in rows.into_iter().enumerate() {
                let offset = row.offset;
                let fold = row.has_children.then(|| {
                    let ix = row.ix;
                    let on_fold: FoldAction =
                        Box::new(move |this, cx| this.toggle_outline_fold(ix, cx));
                    (row.collapsed, on_fold)
                });
                // * The text wraps; a search match is Obsidian's
                //   `suggestion-highlight`, drawn in the accent.
                let highlights: Vec<(Range<usize>, HighlightStyle)> = row
                    .highlights
                    .iter()
                    .filter(|r| r.end <= row.text.len())
                    .map(|r| {
                        (
                            r.clone(),
                            HighlightStyle {
                                color: Some(theme.accent),
                                ..HighlightStyle::default()
                            },
                        )
                    })
                    .collect();
                let label = StyledText::new(SharedString::from(row.text.clone()))
                    .with_highlights(highlights)
                    .into_any_element();
                list = list.child(
                    Self::tree_row(
                        ("outline-row", row.ix),
                        row.depth,
                        fold,
                        None,
                        label,
                        None,
                        row.active || self.outline_state.selected == Some(ix),
                        theme,
                        cx,
                        Some(Box::new(move |this, position, cx| {
                            let path = this.current.clone().unwrap_or_default();
                            this.open_side_menu(
                                position,
                                SidePaneTarget::Heading {
                                    path,
                                    offset: row.offset,
                                },
                                cx,
                            );
                        })),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.reveal_heading(offset, cx))),
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

    /// The File recovery pane: every markdown note that has at least one
    /// snapshot, newest snapshot first, each with a button that restores
    /// that snapshot through the vault's own write path.
    pub(crate) fn render_file_recovery_pane(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let Some(state) = self.state.clone() else {
            return div()
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
                        .child("No vault is open.")
                        .into_any_element(),
                )
                .into_any_element();
        };
        let root = state.root.clone();
        let vault = state.vault();
        let entries =
            crate::file_recovery::collect(&root, vault.markdown_files().map(|f| f.path.as_str()));
        drop(vault);

        if entries.is_empty() {
            return div()
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
                        .child("No snapshots yet. Save a note to create one.")
                        .into_any_element(),
                )
                .into_any_element();
        }

        let header = Self::pane_header().child(SharedString::from("File recovery"));

        let mut list = div()
            .id("file-recovery")
            .flex()
            .flex_col()
            .flex_1()
            .overflow_y_scroll()
            .px(px(12.))
            .pt(px(12.))
            .pb(px(32.));
        for entry in &entries {
            let path = entry.path.clone();
            list = list.child(
                div()
                    .text_size(px(FONT_UI_SMALL))
                    .text_color(theme.text_muted)
                    .mb(px(6.))
                    .child(SharedString::from(path.clone())),
            );
            for version in &entry.versions {
                let path = path.clone();
                let at = version.at;
                let label = version.label();
                let bytes = version.bytes;
                list = list.child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.))
                        .px(px(8.))
                        .py(px(4.))
                        .mb(px(2.))
                        .rounded(px(6.))
                        .hover(|s| s.bg(theme.bg_alt))
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(FONT_UI_SMALL))
                                .child(SharedString::from(format!("{label}  ·  {bytes} bytes"))),
                        )
                        .child(
                            Self::nav_button(
                                "file-recovery-restore",
                                "clock",
                                "Restore",
                                false,
                                theme,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| this.file_recovery_restore(&path, at, cx),
                            )),
                        ),
                );
            }
        }
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(header)
            .child(list)
            .into_any_element()
    }

    /// Whether a left-dock pane's core plugin is on, by its id.
    pub(crate) fn left_pane_enabled(&self, id: &str) -> bool {
        self.state
            .as_ref()
            .is_none_or(|state| state.core_plugin_enabled(id))
    }

    /// Whether a right-dock pane's core plugin is on. Each of the five is
    /// its own plugin in `core-plugins.json`, and a vault with no file has
    /// them all, so the default is what `CORE_PLUGINS` says.
    pub(crate) fn pane_enabled(&self, tab: RightTab) -> bool {
        // * File recovery is Silica's own pane, not an Obsidian core plugin,
        //   so it is always on.
        if matches!(tab, RightTab::FileRecovery) {
            return true;
        }
        let id = match tab {
            RightTab::Backlinks => "backlink",
            RightTab::Outgoing => "outgoing-link",
            RightTab::Tags => "tag-pane",
            RightTab::Properties => "properties",
            RightTab::Outline => "outline",
            RightTab::Footnotes => "footnotes",
            // * The graph's own plugin id, so a vault with the graph core
            //   plugin off takes the tab away with it.
            RightTab::Graph => "graph",
            // * Kalender bukan core plugin dengan id di `core-plugins.json`;
            //   ia dihidupkan oleh kunci `calendar` di `daily-notes.json`
            //   (default on, berkas lama tetap).
            RightTab::Calendar => {
                return self
                    .state
                    .as_ref()
                    .is_none_or(|state| state.daily_notes().calendar.unwrap_or(true));
            }
            RightTab::FileRecovery => unreachable!("handled above"),
            // * The local graph rides on the graph's own plugin id: it is
            //   the same core plugin's second pane, so a vault with the
            //   graph off loses this tab with the other one.
            RightTab::LocalGraph => "graph",
            // * The inbox is this app's own pane, not a core plugin's
            //   leaf: an id no core plugin owns always answers enabled.
            RightTab::Tasks => "tasks",
            RightTab::Flashcards => "flashcards",
            RightTab::AiChat => "ai-chat",
        };
        self.state
            .as_ref()
            .is_none_or(|state| state.core_plugin_enabled(id))
    }

    pub(crate) fn render_right_dock(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        // * The pane whose plugin was just turned off has no tab any more,
        //   so it cannot be the one on screen either: the first that is
        //   still on takes over, as Obsidian moves to a neighbouring leaf.
        if !self.pane_enabled(self.right_tab) {
            if let Some(next) = [
                RightTab::Backlinks,
                RightTab::Outgoing,
                RightTab::Tags,
                RightTab::Properties,
                RightTab::Outline,
                RightTab::Footnotes,
                RightTab::Graph,
                RightTab::Calendar,
                RightTab::LocalGraph,
                RightTab::Tasks,
                RightTab::Flashcards,
                RightTab::AiChat,
            ]
            .into_iter()
            .find(|t| self.pane_enabled(*t))
            {
                self.right_tab = next;
            }
        }
        let tab = self.right_tab;
        let body: gpui::AnyElement = match tab {
            RightTab::Backlinks => self.render_backlinks_pane(theme, cx),
            RightTab::Outgoing => self.render_outgoing_pane(theme, cx),
            RightTab::Outline => self.render_outline_pane(theme, cx),
            RightTab::Tags => self.render_tags_pane(theme, cx),
            RightTab::Properties => self.render_properties_pane(theme, cx),
            RightTab::Footnotes => self.render_footnotes_pane(theme, cx),
            // * Kalender: grid bulan di samping catatan harian.
            RightTab::Calendar => render_calendar_pane(self, theme, cx).into_any_element(),
            RightTab::Tasks => self.render_tasks_pane(theme, cx),
            RightTab::Flashcards => self.render_flashcards_pane(theme, cx),
            RightTab::AiChat => self.render_ai_pane(theme, cx),
            // * The graph side pane: the GraphView entity the ribbon opens
            //   full screen, kept in the dock. While a group's area already
            //   paints it, the dock says so rather than painting it twice.
            RightTab::Graph if self.graph_group().is_some() => div()
                .flex()
                .items_center()
                .justify_center()
                .size_full()
                .min_h_0()
                .px(px(12.))
                .py(px(12.))
                .text_size(px(12.))
                .text_color(theme.text_muted)
                .child("The graph is already open in the main area")
                .into_any_element(),
            RightTab::Graph => div()
                .size_full()
                .min_h_0()
                .child(self.graph.clone())
                .into_any_element(),
            RightTab::FileRecovery => self.render_file_recovery_pane(theme, cx),
            // * The local graph side pane: the open note at the centre with
            //   the notes one hop out. It has no pan or zoom of its own, so
            //   nothing but the pane itself is offered.
            RightTab::LocalGraph => div()
                .size_full()
                .min_h_0()
                .child(self.local_graph.clone())
                .into_any_element(),
        };
        // * A pane whose core plugin is off is not offered: Obsidian takes
        //   the view away with the plugin.
        let tab_button = |this: &mut Self,
                          label: &'static str,
                          icon_name: &'static str,
                          which: RightTab,
                          cx: &mut Context<Self>| {
            this.pane_enabled(which).then(|| {
                this.dock_tab(
                    label,
                    icon_name,
                    tab == which,
                    theme,
                    move |this, window, cx| {
                        this.select_right_tab(which, cx);
                        match which {
                            RightTab::Calendar => window.focus(&this.calendar_focus, cx),
                            // * The dock tab takes the keyboard, the way a row
                            //   click does: the arrows then walk the pane.
                            _ => this.focus_panes(window, cx),
                        }
                    },
                    cx,
                )
            })
        };
        let frameless = self.frameless;
        let mac = self.mac_frame();
        // * Obsidian's tab order and icon names (`links-coming-in` and
        //   `links-going-out` are its own drawings; Silica's are in `assets`).
        div()
            .relative()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .w(px(self.right_width as f32))
            .h_full()
            .bg(theme.bg_secondary)
            // * The pane actions live on this container: the panes' own
            //   focus handle makes them reachable, and each handler checks
            //   that handle is focused before it acts.
            .on_action(cx.listener(Self::pane_up))
            .on_action(cx.listener(Self::pane_down))
            .on_action(cx.listener(Self::pane_on_enter))
            .on_action(cx.listener(Self::pane_on_left))
            .on_action(cx.listener(Self::pane_on_right))
            .on_action(cx.listener(Self::pane_on_escape))
            .on_action(cx.listener(Self::on_flashcard_grade))
            .map(|d| self.corners.right().apply(d))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .h(px(HEADER_HEIGHT))
                    .pl(px(8.))
                    .pr(if frameless && !mac {
                        px(3. * RIBBON_WIDTH)
                    } else {
                        px(8.)
                    })
                    .gap(px(3.))
                    .border_b_1()
                    .border_color(theme.border)
                    .children(tab_button(
                        self,
                        "Backlinks",
                        "links-coming-in",
                        RightTab::Backlinks,
                        cx,
                    ))
                    .children(tab_button(
                        self,
                        "Outgoing links",
                        "links-going-out",
                        RightTab::Outgoing,
                        cx,
                    ))
                    .children(tab_button(self, "Tags", "tags", RightTab::Tags, cx))
                    .children(tab_button(
                        self,
                        "All properties",
                        "archive",
                        RightTab::Properties,
                        cx,
                    ))
                    .children(tab_button(self, "Outline", "list", RightTab::Outline, cx))
                    .children(tab_button(
                        self,
                        "Footnotes",
                        "list",
                        RightTab::Footnotes,
                        cx,
                    ))
                    .children(tab_button(
                        self,
                        "Kalender",
                        "calendar",
                        RightTab::Calendar,
                        cx,
                    ))
                    .children(tab_button(
                        self,
                        "Tasks",
                        "square-check",
                        RightTab::Tasks,
                        cx,
                    ))
                    .children(tab_button(
                        self,
                        "Flashcards",
                        "layers",
                        RightTab::Flashcards,
                        cx,
                    ))
                    .children(tab_button(self, "AI chat", "zap", RightTab::AiChat, cx))
                    .children(tab_button(self, "Graph", "git-fork", RightTab::Graph, cx))
                    .children(tab_button(
                        self,
                        "File recovery",
                        "clock",
                        RightTab::FileRecovery,
                        cx,
                    ))
                    .children(tab_button(
                        self,
                        crate::i18n::t("Local graph"),
                        "binary",
                        RightTab::LocalGraph,
                        cx,
                    ))
                    .child(window_handle(
                        div().id("right-dock-space").flex_1().h_full(),
                        frameless,
                    ))
                    // * The window's top-right corner on macOS: the right
                    //   sidebar's toggle, which Obsidian fixes there.
                    .when(mac, |d| d.child(self.sidebar_toggle(false, theme, cx))),
            )
            .child(div().flex_1().overflow_hidden().child(body))
            .children(self.render_side_menu(theme, cx))
            .child(self.dock_handle(false, theme))
            .children(self.render_right_dock_context_menu(theme, cx))
    }

    /// Renders the right-dock context menu over backlinks/outgoing links/tags/outline rows.
    pub(crate) fn render_right_dock_context_menu(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let Some(path) = &self.right_dock_context_menu else {
            return None;
        };
        let entity = cx.entity();
        let path_clone = path.clone();
        let mut list = div()
            .id("right-dock-menu")
            .absolute()
            .left(px(10.))
            .top(px(40.))
            .flex()
            .flex_col()
            .min_w(px(180.))
            .py_1()
            .bg(theme.bg_secondary)
            .border_1()
            .border_color(theme.border)
            .rounded_md()
            .shadow_lg()
            .text_size(px(13.))
            .text_color(theme.text)
            .occlude();
        let items: Vec<(&'static str, &'static str)> = vec![
            ("Open in new tab", "file-plus"),
            ("Open to the right", "separator-vertical"),
            ("Copy path", "clipboard"),
        ];
        for (label, icon) in items {
            let path_for_item = path_clone.clone();
            let entity_for_item = entity.clone();
            list = list.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_0p5()
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.bg_active))
                    .child(
                        div()
                            .w(px(16.))
                            .flex_shrink_0()
                            .text_color(theme.text_muted)
                            .child(crate::assets::icon(icon, 14., theme.text_muted)),
                    )
                    .child(label)
                    .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                        let path_for_item = path_for_item.clone();
                        entity_for_item.update(cx, |ws, cx| {
                            ws.right_dock_context_menu = None;
                            match label {
                                "Open in new tab" => {
                                    ws.open_file(path_for_item.clone(), None, cx);
                                }
                                "Open to the right" => {
                                    ws.open_file(path_for_item.clone(), None, cx);
                                }
                                "Copy path" => {
                                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                        path_for_item,
                                    ));
                                }
                                _ => {}
                            }
                        });
                    }),
            );
        }
        Some(list.into_any_element())
    }

    // ----- keyboard navigation -------------------------------------------------

    /// Focuses the right-dock panes, the way clicking a row or its dock tab
    /// does: the arrow keys move within the pane from then on.
    pub(crate) fn focus_panes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let handle = self.pane_focus.clone();
        window.focus(&handle, cx);
        cx.notify();
    }

    /// Whether the panes hold the keyboard. The four pane actions are
    /// registered on the workspace's element tree, so without this guard
    /// they would also move the mark while the main area is focused.
    fn panes_focused(&self, window: &Window) -> bool {
        self.pane_focus.clone().is_focused(window)
    }

    fn pane_up(&mut self, _: &PaneUp, window: &mut Window, cx: &mut Context<Self>) {
        if self.panes_focused(window) {
            self.pane_move(-1, cx);
        }
    }

    fn pane_down(&mut self, _: &PaneDown, window: &mut Window, cx: &mut Context<Self>) {
        if self.panes_focused(window) {
            self.pane_move(1, cx);
        }
    }

    fn pane_on_enter(&mut self, _: &PaneActivate, window: &mut Window, cx: &mut Context<Self>) {
        if self.panes_focused(window) {
            self.pane_activate(window, cx);
        }
    }

    fn pane_on_escape(&mut self, _: &PaneLeaveFocus, window: &mut Window, cx: &mut Context<Self>) {
        if self.panes_focused(window) {
            self.pane_leave_focus(window, cx);
        }
    }

    fn pane_on_left(&mut self, _: &PaneFoldLeft, window: &mut Window, cx: &mut Context<Self>) {
        if self.panes_focused(window) {
            self.pane_fold_left(cx);
        }
    }

    fn pane_on_right(&mut self, _: &PaneUnfoldRight, window: &mut Window, cx: &mut Context<Self>) {
        if self.panes_focused(window) {
            self.pane_fold_right(cx);
        }
    }

    /// How many rows the visible pane has, for clamping the arrow keys.
    pub(crate) fn pane_row_count(&self, cx: &App) -> usize {
        match self.right_tab {
            RightTab::Tags => self.tag_rows(cx).len(),
            RightTab::Properties => self.property_rows(cx).len(),
            RightTab::Outline => self.outline_rows(cx).len(),
            RightTab::Tasks => self.visible_task_rows(cx).len(),
            // * Backlinks and Outgoing are card lists without a row model
            //   the arrows could walk; their rows stay click-only.
            _ => 0,
        }
    }

    /// The selection of the pane that has the keyboard, moved by `delta`
    /// rows and clamped to the pane's row count.
    pub(crate) fn pane_move(&mut self, delta: isize, cx: &mut Context<Self>) {
        let current = match self.right_tab {
            RightTab::Tags => self.tag_state.selected,
            RightTab::Properties => self.property_state.selected,
            RightTab::Outline => self.outline_state.selected,
            RightTab::Tasks => self.task_state.selected,
            _ => None,
        };
        let count = self.pane_row_count(cx);
        if count == 0 {
            return;
        }
        let Some(next) = next_row_selection(current, self.pane_row_count(cx), delta) else {
            return;
        };
        match self.right_tab {
            RightTab::Tags => self.tag_state.selected = Some(next),
            RightTab::Properties => self.property_state.selected = Some(next),
            RightTab::Outline => self.outline_state.selected = Some(next),
            _ => {}
        }
        cx.notify();
    }

    /// Left on the marked row: folds the group it sits on, or — a group
    /// that is already folded hands the mark to its parent, as Obsidian's
    /// `tree-item-self` keyboard contract has it. Properties have no
    /// groups to fold, so Left does nothing there.
    pub(crate) fn pane_fold_left(&mut self, cx: &mut Context<Self>) {
        match self.right_tab {
            RightTab::Tags => {
                let rows = self.tag_rows(cx);
                let Some(ix) = self.tag_state.selected else {
                    return;
                };
                let Some(row) = rows.get(ix) else {
                    return;
                };
                let depths: Vec<usize> = rows.iter().map(|row| row.depth).collect();
                if row.has_children && !row.collapsed {
                    let key = row.key.clone();
                    self.toggle_tag_fold(key, cx);
                } else if let Some(parent) = parent_row(&depths, ix) {
                    self.tag_state.selected = Some(parent);
                    cx.notify();
                }
            }
            RightTab::Outline => {
                let rows = self.outline_rows(cx);
                let Some(ix) = self.outline_state.selected else {
                    return;
                };
                let Some(row) = rows.get(ix) else {
                    return;
                };
                let depths: Vec<usize> = rows.iter().map(|row| row.depth).collect();
                if row.has_children && !row.collapsed {
                    self.toggle_outline_fold(row.ix, cx);
                } else if let Some(parent) = parent_row(&depths, ix) {
                    self.outline_state.selected = Some(parent);
                    cx.notify();
                }
            }
            _ => {}
        }
    }

    /// Right on the marked row: opens a folded group, or hands the mark to
    /// the first child of one already open. A leaf has nothing to open,
    /// so Right does nothing there.
    pub(crate) fn pane_fold_right(&mut self, cx: &mut Context<Self>) {
        match self.right_tab {
            RightTab::Tags => {
                let rows = self.tag_rows(cx);
                let Some(ix) = self.tag_state.selected else {
                    return;
                };
                let Some(row) = rows.get(ix) else {
                    return;
                };
                let depths: Vec<usize> = rows.iter().map(|row| row.depth).collect();
                if !row.has_children {
                    return;
                }
                if row.collapsed {
                    let key = row.key.clone();
                    self.toggle_tag_fold(key, cx);
                } else if let Some(child) = first_child_row(&depths, ix) {
                    self.tag_state.selected = Some(child);
                    cx.notify();
                }
            }
            RightTab::Outline => {
                let rows = self.outline_rows(cx);
                let Some(ix) = self.outline_state.selected else {
                    return;
                };
                let Some(row) = rows.get(ix) else {
                    return;
                };
                let depths: Vec<usize> = rows.iter().map(|row| row.depth).collect();
                if !row.has_children {
                    return;
                }
                if row.collapsed {
                    self.toggle_outline_fold(row.ix, cx);
                } else if let Some(child) = first_child_row(&depths, ix) {
                    self.outline_state.selected = Some(child);
                    cx.notify();
                }
            }
            _ => {}
        }
    }

    /// Enter on the selected row: the same thing a click on it does — a tag
    /// or property row searches for it, an outline row reveals its heading
    /// in the note, an inbox row opens the note its task came from.
    pub(crate) fn pane_activate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let action = match self.right_tab {
            RightTab::Tags => self
                .tag_state
                .selected
                .and_then(|ix| self.tag_rows(cx).get(ix).map(tag_row_action)),
            RightTab::Properties => self
                .property_state
                .selected
                .and_then(|ix| self.property_rows(cx).get(ix).map(property_row_action)),
            RightTab::Outline => self
                .outline_state
                .selected
                .and_then(|ix| self.outline_rows(cx).get(ix).map(outline_row_action)),
            _ => None,
        };
        match action {
            Some(PaneRowAction::Search(query)) => self.show_search(&query, Some(window), cx),
            Some(PaneRowAction::Reveal(offset)) => self.reveal_heading(offset, cx),
            None => {}
        }
        // * Tags/Properties/Outline dispatch through the row-action helpers
        //   above; only Tasks needs its own arm (open_task).
        if let RightTab::Tasks = self.right_tab {
            let Some(ix) = self.task_state.selected else {
                return;
            };
            let rows = self.visible_task_rows(cx);
            if let Some(row) = rows.get(ix) {
                self.open_task(row, cx);
            }
        }
    }

    /// Escape hands the keyboard back to the main area and drops the mark.
    pub(crate) fn pane_leave_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.right_tab {
            RightTab::Tags => self.tag_state.selected = None,
            RightTab::Properties => self.property_state.selected = None,
            RightTab::Outline => self.outline_state.selected = None,
            RightTab::Tasks => self.task_state.selected = None,
            _ => {}
        }
        self.focus_main(window, cx);
    }

    /// The Footnotes pane: one row per footnote in the active note, numbered
    /// in section order; clicking a row reveals its definition's line with
    /// the same `reveal_offset` a footnote reference click uses.
    fn render_footnotes_pane(&mut self, theme: &Theme, cx: &mut Context<Self>) -> gpui::AnyElement {
        let model = self.reading.read(cx).model();
        let rows = footnote_rows(model);
        let empty = rows.is_empty();
        let header = Self::pane_header();
        let mut list = div()
            .id("footnotes")
            .flex()
            .flex_col()
            .flex_1()
            .overflow_y_scroll()
            .px(px(12.))
            .pt(px(4.))
            .pb(px(32.));
        for row in rows {
            let offset = row.offset;
            list = list.child(
                Self::tree_row(
                    ("footnote-row", row.n),
                    0,
                    None,
                    None,
                    Self::tree_label(format!("[{}]", row.n)),
                    None,
                    false,
                    theme,
                    cx,
                    None,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.reveal_offset(offset, cx);
                    this.focus_panes(window, cx);
                })),
            );
        }
        let body: gpui::AnyElement = if empty {
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
                        .child("No footnotes."),
                )
                .into_any_element()
        } else {
            list.into_any_element()
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(header)
            .child(body)
            .into_any_element()
    }
}

/// Where the arrow keys put the pane's mark next: `current` moved by
/// `delta` rows, clamped to the `count` rows on screen. Nothing is
/// selected before the first press, so a downward stroke marks the first
/// row and an upward one the last, as Obsidian's list navigation does.
/// A pane with no rows has nothing to mark.
pub(crate) fn next_row_selection(
    current: Option<usize>,
    count: usize,
    delta: isize,
) -> Option<usize> {
    if count == 0 {
        return None;
    }
    Some(match current {
        Some(ix) => (ix as isize + delta).clamp(0, count as isize - 1) as usize,
        None => {
            if delta >= 0 {
                0
            } else {
                count - 1
            }
        }
    })
}

/// The nearest row before `ix` at a shallower depth: the group `ix` sits
/// in, or none for a row at the root.
pub(crate) fn parent_row(depths: &[usize], ix: usize) -> Option<usize> {
    let depth = *depths.get(ix)?;
    (0..ix).rev().find(|&k| depths[k] < depth)
}

/// The row just after `ix` when it is one level deeper: `ix`'s first
/// child, or none when `ix` is a leaf or the last row.
pub(crate) fn first_child_row(depths: &[usize], ix: usize) -> Option<usize> {
    let depth = *depths.get(ix)?;
    let next = ix + 1;
    (depths.get(next)? > &depth).then_some(next)
}

/// What Enter does on a pane's marked row: the same thing a click on it
/// does.
pub(crate) enum PaneRowAction {
    /// Search the vault for this query, the pane's own field's text set to
    /// it, as a tag or property row's click does.
    Search(String),
    /// Reveal this byte offset in the note, as an outline row's click does.
    Reveal(usize),
}

pub(crate) fn tag_row_action(row: &panes::TagRow) -> PaneRowAction {
    PaneRowAction::Search(format!("tag:#{}", row.path))
}

pub(crate) fn property_row_action(row: &panes::PropertyRow) -> PaneRowAction {
    PaneRowAction::Search(format!("[\"{}\"]", row.name))
}

pub(crate) fn outline_row_action(row: &panes::OutlineRow) -> PaneRowAction {
    PaneRowAction::Reveal(row.offset)
}

/// Keys for the side panes, bound to the panes' own context beneath the
/// workspace's: a row click (or the dock tab) focuses the panes, the
/// arrows move the mark, Enter runs the row, Left/Right fold and open the
/// group the mark is on, Escape goes back to the note.
pub fn bind_pane_keys(cx: &mut App) {
    let ctx = Some("SidePane");
    cx.bind_keys([
        KeyBinding::new("up", PaneUp, ctx),
        KeyBinding::new("down", PaneDown, ctx),
        KeyBinding::new("enter", PaneActivate, ctx),
        KeyBinding::new("left", PaneFoldLeft, ctx),
        KeyBinding::new("right", PaneUnfoldRight, ctx),
        KeyBinding::new("escape", PaneLeaveFocus, ctx),
    ]);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag_row(path: &str, depth: usize, has_children: bool, collapsed: bool) -> panes::TagRow {
        panes::TagRow {
            name: path.rsplit('/').next().unwrap_or(path).to_string(),
            parent: path
                .rsplit_once('/')
                .map(|(head, _)| format!("{head}/"))
                .unwrap_or_default(),
            path: path.to_string(),
            key: path.to_lowercase(),
            count: 1,
            depth,
            has_children,
            collapsed,
        }
    }

    fn property_row(name: &str) -> panes::PropertyRow {
        panes::PropertyRow {
            name: name.to_string(),
            count: 1,
            ty: None,
            icon: "file-question",
        }
    }

    fn outline_row(ix: usize, offset: usize, depth: usize) -> panes::OutlineRow {
        panes::OutlineRow {
            ix,
            depth,
            text: format!("Heading {ix}"),
            level: depth as u8 + 1,
            line: ix,
            offset,
            has_children: false,
            collapsed: false,
            active: false,
            highlights: Vec::new(),
        }
    }

    /// A downward key event on a pane with rows moves the mark down the
    /// rows, one per stroke, and stops at the last one.
    #[test]
    fn a_down_key_moves_the_pane_mark_one_row_at_a_time() {
        // * Nothing marked yet: the first downward stroke lands on the
        //   first row, the way a list's first arrow does.
        assert_eq!(next_row_selection(None, 3, 1), Some(0));
        assert_eq!(next_row_selection(Some(0), 3, 1), Some(1));
        assert_eq!(next_row_selection(Some(1), 3, 1), Some(2));
        // * Clamped at the end: more strokes change nothing.
        assert_eq!(next_row_selection(Some(2), 3, 1), Some(2));
        assert_eq!(next_row_selection(Some(2), 5, 9), Some(4));
    }

    /// An upward key event walks the other way, and with nothing marked
    /// yet lands on the last row.
    #[test]
    fn an_up_key_moves_the_pane_mark_the_other_way() {
        assert_eq!(next_row_selection(None, 3, -1), Some(2));
        assert_eq!(next_row_selection(Some(2), 3, -1), Some(1));
        assert_eq!(next_row_selection(Some(1), 3, -1), Some(0));
        // * Clamped at the top.
        assert_eq!(next_row_selection(Some(0), 3, -1), Some(0));
    }

    /// A pane whose filter left no rows has no mark to move.
    #[test]
    fn an_arrow_on_a_pane_with_no_rows_moves_nothing() {
        assert_eq!(next_row_selection(None, 0, 1), None);
        assert_eq!(next_row_selection(Some(0), 0, -1), None);
    }

    /// Enter on a tag row searches `tag:#path`, the query a click on it
    /// runs.
    #[test]
    fn enter_on_a_tag_row_searches_for_it() {
        match tag_row_action(&tag_row("project/alpha", 1, true, false)) {
            PaneRowAction::Search(query) => assert_eq!(query, "tag:#project/alpha"),
            PaneRowAction::Reveal(_) => panic!("a tag row searches, it does not reveal"),
        }
    }

    /// Enter on a property row searches `["key"]`, the query a click on it
    /// runs.
    #[test]
    fn enter_on_a_property_row_searches_for_it() {
        match property_row_action(&property_row("status")) {
            PaneRowAction::Search(query) => assert_eq!(query, "[\"status\"]"),
            PaneRowAction::Reveal(_) => panic!("a property row searches, it does not reveal"),
        }
    }

    /// Enter on an outline row reveals the heading's offset in the note,
    /// the jump a click on it makes.
    #[test]
    fn enter_on_an_outline_row_reveals_its_heading() {
        match outline_row_action(&outline_row(2, 128, 1)) {
            PaneRowAction::Reveal(offset) => assert_eq!(offset, 128),
            PaneRowAction::Search(_) => panic!("an outline row reveals, it does not search"),
        }
    }

    /// Left on a folded group hands the mark to the group it sits in, and
    /// a root row has none to go to.
    #[test]
    fn left_hands_the_mark_to_the_parent_group() {
        let depths = [0, 1, 2, 1];
        assert_eq!(parent_row(&depths, 2), Some(1));
        assert_eq!(parent_row(&depths, 3), Some(0));
        // * A root row's parent is not on the list.
        assert_eq!(parent_row(&depths, 0), None);
        assert_eq!(parent_row(&depths, 9), None);
    }

    /// Right on an open group hands the mark to its first child; a leaf
    /// and the last row have no child to go to.
    #[test]
    fn right_hands_the_mark_to_the_first_child() {
        let depths = [0, 1, 1];
        assert_eq!(first_child_row(&depths, 0), Some(1));
        assert_eq!(first_child_row(&depths, 1), None);
        assert_eq!(first_child_row(&depths, 2), None);
        assert_eq!(first_child_row(&[], 0), None);
    }
}
