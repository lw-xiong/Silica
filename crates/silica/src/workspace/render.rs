//! The window frame: drawing the layout tree, the tab strip, side panes,
//! status bar, and overlays — the methods of the split-out rendering
//! section, unchanged apart from living in their own module. Split from
//! `workspace.rs` per the audit's module-size finding.

use super::*;

impl Workspace {
    /// A sidebar's tab: Obsidian shows the view's icon alone, 16 px in a
    /// 28 by 25 button with a 4 px radius, the active one on a wash.
    pub(crate) fn dock_tab(
        &self,
        label: &'static str,
        icon_name: &'static str,
        active: bool,
        theme: &Theme,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id(label)
            .flex_shrink_0()
            .w(px(28.))
            .h(px(25.))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .rounded(px(4.))
            .when(active, |d| d.bg(theme.bg_active))
            .hover(|s| s.bg(theme.bg_alt))
            .on_click(cx.listener(move |this, _, window, cx| on_click(this, window, cx)))
            .child(icon(
                icon_name,
                ICON_SIZE,
                if active { theme.text } else { theme.text_muted },
            ))
    }

    /// A pane's own header button: Obsidian's `nav-action-button`, 28 by
    /// 24 with a 4 px radius, the icon muted until the pointer is over it;
    /// `active` is the wash and accent of an `is-active` one.
    pub(crate) fn nav_button(
        id: &'static str,
        icon_name: &'static str,
        label: &'static str,
        active: bool,
        theme: &Theme,
    ) -> gpui::Stateful<gpui::Div> {
        div()
            .id(id)
            .flex_shrink_0()
            .w(px(28.))
            .h(px(24.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .cursor_pointer()
            .when(active, |d| d.bg(theme.bg_alt))
            .hover(|s| s.bg(theme.bg_alt))
            .tooltip(tooltip(label))
            .child(icon(
                icon_name,
                ICON_SIZE,
                if active {
                    theme.accent
                } else {
                    theme.text_muted
                },
            ))
    }

    /// The Backlinks pane: Obsidian's four header actions (collapse,
    /// more context, sort, filter), the filter field when it is shown,
    /// then the two sections.
    pub(crate) fn render_backlinks_pane(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let (collapse_all, extra, show_search) = (
            self.backlink_state.collapse_all,
            self.backlink_state.extra_context,
            self.backlink_state.show_search,
        );
        let sort_menu = self.render_backlink_sort_menu(theme, cx);
        let header = div()
            .flex_shrink_0()
            .h(px(HEADER_HEIGHT))
            .p(px(8.))
            .flex()
            .flex_row()
            .items_center()
            .justify_center()
            .gap(px(2.))
            .child(
                Self::nav_button(
                    "backlinks-collapse",
                    "list",
                    "Collapse results",
                    collapse_all,
                    theme,
                )
                .on_click(cx.listener(|this, _, _, cx| this.toggle_backlink_collapse_all(cx))),
            )
            .child(
                Self::nav_button(
                    "backlinks-context",
                    "move-vertical",
                    "Show more context",
                    extra,
                    theme,
                )
                .on_click(cx.listener(|this, _, _, cx| this.toggle_backlink_context(cx))),
            )
            .child(
                // * The menu hangs off this wrapper, so it opens under the
                //   button wherever the dock is.
                div()
                    .relative()
                    .child(
                        Self::nav_button(
                            "backlinks-sort",
                            "arrow-up-narrow-wide",
                            "Change sort order",
                            false,
                            theme,
                        )
                        .on_click(cx.listener(
                            |this, event: &ClickEvent, _, cx| {
                                this.sort_menu_at = event.position();
                                this.backlink_state.sort_menu_open =
                                    !this.backlink_state.sort_menu_open;
                                cx.notify();
                            },
                        )),
                    )
                    .children(sort_menu),
            )
            .child(
                Self::nav_button(
                    "backlinks-search",
                    "search",
                    "Show search filter",
                    show_search,
                    theme,
                )
                .on_click(
                    cx.listener(|this, _, window, cx| this.toggle_backlink_search(window, cx)),
                ),
            );
        let query = self.backlink_filter.read(cx).text().to_string();
        let filter = show_search.then(|| {
            div()
                .flex_shrink_0()
                .px_2()
                .pb_2()
                .child(self.backlink_filter.clone())
        });
        let body = self.render_sections(RightTab::Backlinks, &query, theme, cx);
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(header)
            .children(filter)
            .child(body)
            .into_any_element()
    }

    /// The Outgoing links pane has no header of its own: Links, then
    /// Unlinked mentions.
    pub(crate) fn render_outgoing_pane(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        self.render_sections(RightTab::Outgoing, "", theme, cx)
    }

    /// "Change sort order" for the Backlinks pane: the six orders in three
    /// pairs, a check against the one in force.
    pub(crate) fn render_backlink_sort_menu(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        if !self.backlink_state.sort_menu_open {
            return None;
        }
        let current = self.backlink_state.sort;
        let mut list = div()
            .id("backlinks-sort-menu")
            .flex()
            .flex_col()
            .min_w(px(220.))
            .py_1()
            .bg(theme.bg_secondary)
            .border_1()
            .border_color(theme.border)
            .rounded_md()
            .shadow_lg()
            .text_size(px(13.))
            .text_color(theme.text)
            // * Keeps a click on an item from reaching the row under it.
            .occlude()
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.backlink_state.sort_menu_open = false;
                cx.notify();
            }));
        for (ix, order) in SortOrder::ALL.into_iter().enumerate() {
            if ix > 0 && ix % 2 == 0 {
                list = list.child(div().my_1().h(px(1.)).bg(theme.border));
            }
            let checked = order == current;
            list = list.child(
                div()
                    .id(("backlinks-sort-item", ix))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_1()
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.bg_alt))
                    .on_click(cx.listener(move |this, _, _, cx| this.set_backlink_sort(order, cx)))
                    .child(
                        div()
                            .w(px(16.))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .children(checked.then(|| icon("check", 14., theme.text_muted))),
                    )
                    .child(order.label()),
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

    /// A pane's sections, drawn the way Obsidian's are: a section row with
    /// its count (no fold glyph; the row itself folds), a collapsible row
    /// per file with a triangle and its count, and the file's matches in
    /// one card with the matched text highlighted. The Links section's rows
    /// stand alone with a glyph for what they point at; the Outgoing pane's
    /// unlinked mentions are one card for the note itself, each match
    /// naming the note it mentions. A match wraps, so this is a scrolling
    /// column rather than a uniform list.
    pub(crate) fn render_sections(
        &mut self,
        which: RightTab,
        query: &str,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let outgoing = which == RightTab::Outgoing;
        let sections = if outgoing {
            self.outgoing_sections.clone()
        } else {
            self.backlink_sections.clone()
        };
        let prefix = if outgoing { "outgoing" } else { "backlinks" };
        let current = self.current.clone().unwrap_or_default();
        // * Obsidian's `.outgoing-link-pane` is padded 12px 12px 32px; the
        //   backlink pane 4px 12px 32px, its nav header carrying the rest.
        let mut list = div()
            .id(prefix)
            .flex()
            .flex_col()
            .flex_1()
            .overflow_y_scroll()
            .px(px(12.))
            .pt(px(if outgoing { 12. } else { 4. }))
            .pb(px(32.));
        let mut ix = 0usize;
        let mut next = || {
            ix += 1;
            ix
        };
        for (s_ix, section) in sections.iter().enumerate() {
            let shown = if outgoing {
                section.clone()
            } else {
                backlinks::filter(section, query)
            };
            let folded = self.section_collapsed(which, s_ix);
            let count: SharedString = if section.capped {
                format!("{}+", shown.count).into()
            } else {
                shown.count.to_string().into()
            };
            // * A folded section is drawn faint throughout; an open one's
            //   count brightens from faint to muted under the pointer.
            list = list.child(
                div()
                    .id(("pane-section", next()))
                    .group("pane-section")
                    .w_full()
                    .flex_shrink_0()
                    .h(px(25.))
                    .mb(px(2.))
                    .px(px(8.))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(4.))
                    .rounded(px(8.))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.bg_alt))
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.toggle_section(which, s_ix, cx)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(FONT_UI_SMALL))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(if folded { theme.text_faint } else { theme.text })
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(section.label),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(FONT_UI_SMALLER))
                            .text_color(theme.text_faint)
                            .when(!folded, |d| {
                                d.group_hover("pane-section", |s| s.text_color(theme.text_muted))
                            })
                            .child(count),
                    ),
            );
            if folded {
                continue;
            }
            // * Obsidian's `.search-result-container`: 4px 1px 16px.
            let mut body = div()
                .w_full()
                .flex_shrink_0()
                .flex()
                .flex_col()
                .pt(px(4.))
                .pb(px(16.))
                .px(px(1.));
            if !outgoing && s_ix == 0 && shown.groups.is_empty() {
                body = body.child(
                    div()
                        .pl(px(8.))
                        .mb(px(12.))
                        .text_size(px(FONT_UI_SMALL))
                        .text_color(theme.text_faint)
                        .child("No backlinks found."),
                );
            }
            // * The Outgoing pane's unlinked mentions: one card, no title row.
            let one_card = outgoing && s_ix == 1;
            if one_card {
                let mut card = Self::match_card(theme);
                for group in &shown.groups {
                    for m in &group.matches {
                        card = card.child(self.render_match(
                            which,
                            next(),
                            &current,
                            group,
                            m,
                            theme,
                            cx,
                        ));
                    }
                }
                body = body.child(card);
                list = list.child(body);
                continue;
            }
            for group in &shown.groups {
                let key = shown.group_key(group);
                let flat = group.matches.is_empty();
                let collapsed = !flat && self.group_collapsed(which, &key);
                let path = group.path.clone();
                let link = group.link.clone();
                let resolved = path.is_some();
                // * The fold triangle (a mentions group) or the link's glyph
                //   (a Links row) sits in the row's 24px left gutter.
                let gutter: gpui::AnyElement = if flat {
                    let name = group.icon.unwrap_or("link");
                    div()
                        .w(px(16.))
                        .h(px(17.))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .opacity(0.85)
                        .child(icon(name, 14., theme.text_muted))
                        .into_any_element()
                } else {
                    let key = key.clone();
                    div()
                        .id(("pane-chevron", next()))
                        .w(px(16.))
                        .h(px(17.))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .opacity(0.85)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.toggle_group(which, key.clone(), cx);
                            cx.stop_propagation();
                        }))
                        .child(icon(
                            if collapsed {
                                "chevron-right"
                            } else {
                                "chevron-down"
                            },
                            12.,
                            theme.text_muted,
                        ))
                        .into_any_element()
                };
                let flair: Option<SharedString> =
                    (!flat).then(|| group.matches.len().to_string().into());
                let subtext: Option<SharedString> = group.subtext.clone().map(Into::into);
                let unresolved = flat && !resolved;
                list = list.child(
                    div()
                        .id(("pane-group", next()))
                        .group("pane-group")
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
                        .gap(px(4.))
                        .rounded(px(8.))
                        .cursor_pointer()
                        .hover(|s| s.bg(theme.bg_alt))
                        .when(unresolved, |d| d.tooltip(tooltip("Not created yet")))
                        .on_click({
                            // * A Links row follows the link the way the note
                            //   would, subpath included, and creates the note
                            //   nothing stands behind; a mentions row opens
                            //   the file.
                            let path = path.clone();
                            let link = link.clone();
                            cx.listener(move |this, _, _, cx| {
                                if let Some(link) = link.clone() {
                                    this.follow_link(LinkTarget::Internal(link), cx);
                                } else if let Some(path) = path.clone() {
                                    this.open_file(path, None, cx);
                                }
                            })
                        })
                        // * A right-click on a Backlinks or Outgoing row opens
                        //   the row menu the other panes have had: open the
                        //   note here, in a new tab, or to the right, and copy
                        //   its path. An unresolved row targets the note its
                        //   link would create, the same place its click goes.
                        .on_mouse_down(gpui::MouseButton::Right, {
                            let path = path.clone();
                            let link = link.clone();
                            cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                                let target = SidePaneTarget::Link {
                                    path: path.clone().unwrap_or_default(),
                                    link: if path.is_none() { link.clone() } else { None },
                                };
                                this.open_side_menu(event.position, target, cx);
                                cx.stop_propagation();
                            })
                        })
                        .child(gutter)
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .text_size(px(FONT_UI_SMALL))
                                        .line_height(px(17.))
                                        .text_color(if flat {
                                            theme.text_muted
                                        } else {
                                            theme.text
                                        })
                                        .when(flat, |d| {
                                            d.group_hover("pane-group", |s| {
                                                s.text_color(theme.text)
                                            })
                                        })
                                        .whitespace_nowrap()
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .child(SharedString::from(group.label.clone())),
                                )
                                .children(subtext.map(|sub| {
                                    div()
                                        .text_size(px(11.))
                                        .line_height(px(14.))
                                        .text_color(theme.text_faint)
                                        .whitespace_nowrap()
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .child(sub)
                                })),
                        )
                        .children(flair.map(|count| {
                            div()
                                .flex_shrink_0()
                                .pt(px(2.))
                                .text_size(px(FONT_UI_SMALLER))
                                .text_color(theme.text_faint)
                                .child(count)
                        })),
                );
                if flat || collapsed {
                    continue;
                }
                let mut card = Self::match_card(theme);
                for m in &group.matches {
                    card =
                        card.child(self.render_match(which, next(), &current, group, m, theme, cx));
                }
                body = body.child(card);
            }
            list = list.child(body);
        }
        list.into_any_element()
    }

    /// Obsidian's `.search-result-file-matches`: the note's background,
    /// a 4px radius, 4px above and 8px below, no border.
    pub(crate) fn match_card(theme: &Theme) -> gpui::Div {
        div()
            .w_full()
            .flex_shrink_0()
            .mt(px(4.))
            .mb(px(8.))
            .flex()
            .flex_col()
            .rounded(px(4.))
            .bg(theme.bg)
    }

    /// One match in a file's card: its context with the matched text
    /// highlighted; for an unlinked mention, the control that turns the
    /// text into a link: a Link button at the row's right that shows under
    /// the pointer (Backlinks), or a chip under the text naming the note
    /// the mention is of (Outgoing links).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render_match(
        &self,
        which: RightTab,
        ix: usize,
        current: &str,
        group: &Group,
        m: &PaneMatch,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let path = group.path.clone();
        let offset = m.offset;
        // * The match's own \"Show more context\": its line with the lines
        //   either side of it, kept in the pane state keyed by the file and
        //   the match's byte offset. Collapsed it is the single line the
        //   sections were built with.
        let expanded = path
            .as_ref()
            .and_then(|p| offset.map(|o| (p.clone(), o)))
            .and_then(|key| self.backlink_state.expanded.get(&key));
        let show = expanded
            .map(|m| SharedString::from(m.text.clone()))
            .unwrap_or_else(|| SharedString::from(m.text.clone()));
        let ranges = expanded
            .map(|m| m.ranges.as_slice())
            .unwrap_or(m.ranges.as_slice());
        let highlights: Vec<(Range<usize>, HighlightStyle)> = ranges
            .iter()
            .filter(|r| r.end <= show.len())
            .map(|r| {
                (
                    r.clone(),
                    HighlightStyle {
                        background_color: Some(theme.highlight_bg),
                        color: Some(theme.text),
                        ..HighlightStyle::default()
                    },
                )
            })
            .collect();
        let text = StyledText::new(show).with_highlights(highlights);
        // * Which note the mention is in and which it names depend on the
        //   pane: the Backlinks pane lists other notes mentioning this one,
        //   the Outgoing pane this note mentioning others.
        let outgoing = which == RightTab::Outgoing;
        let link = m.mention.clone().and_then(|mention| {
            let (source, target) = if outgoing {
                (current.to_string(), m.target.clone()?)
            } else {
                (path.clone()?, current.to_string())
            };
            Some((source, target, mention))
        });
        let chip = link
            .clone()
            .filter(|_| outgoing)
            .map(|(source, target, mention)| {
                let name: SharedString = display_name(target.rsplit('/').next().unwrap_or(&target))
                    .to_string()
                    .into();
                div().mt(px(6.)).flex().flex_row().child(
                    div()
                        .id(("pane-chip", ix))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(4.))
                        .px(px(6.))
                        .py(px(4.))
                        .rounded(px(4.))
                        .bg(theme.border)
                        .cursor_pointer()
                        .tooltip(tooltip("Link this file"))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.link_mention(
                                source.clone(),
                                target.clone(),
                                mention.clone(),
                                window,
                                cx,
                            );
                            cx.stop_propagation();
                        }))
                        .child(icon("link", 14., theme.text_faint))
                        .child(
                            div()
                                .text_size(px(FONT_UI_SMALLER))
                                .text_color(theme.text_muted)
                                .child(name),
                        ),
                )
            });
        let button = link.filter(|_| !outgoing).map(|(source, target, mention)| {
            div()
                .id(("pane-link", ix))
                .absolute()
                .top(px(2.))
                .right(px(24.))
                .px(px(8.))
                .py(px(4.))
                .rounded(px(5.))
                .bg(theme.border)
                .text_size(px(FONT_UI_SMALLER))
                .text_color(theme.text)
                .cursor_pointer()
                .invisible()
                .group_hover("pane-match", |s| s.visible())
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.link_mention(source.clone(), target.clone(), mention.clone(), window, cx);
                    cx.stop_propagation();
                }))
                .child("Link")
        });
        // * The match's own "Show more context" chevron: expands the match
        //   to its line with the lines either side of it, clicked again to
        //   collapse. Hidden when the match has no range to grow from.
        let chevron = (path.is_some() && offset.is_some() && !m.ranges.is_empty()).then(|| {
            let path = path.as_ref().unwrap().clone();
            let offset = offset.unwrap();
            let on = expanded.is_some();
            div()
                .id(("pane-match-more", ix))
                .absolute()
                .top(px(8.))
                .right(px(6.))
                .px(px(2.))
                .py(px(1.))
                .rounded(px(4.))
                .opacity(if on { 1. } else { 0.6 })
                .hover(|s| s.opacity(1.))
                .cursor_pointer()
                .tooltip(tooltip("Show more context"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    let path = path.clone();
                    this.toggle_backlink_match(path, offset, cx);
                    cx.stop_propagation();
                }))
                .child(icon(
                    if on { "chevron-down" } else { "chevron-right" },
                    10.,
                    theme.text_muted,
                ))
        });
        let mut hover = theme.accent;
        hover.a = 0.33;
        let path_for_right_click = path.clone();
        let entity = cx.entity();
        div()
            .id(("pane-match", ix))
            .group("pane-match")
            .relative()
            .w_full()
            .flex_shrink_0()
            .pl(px(12.))
            .pr(px(20.))
            .py(px(8.))
            .flex()
            .flex_col()
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(path) = path.clone() {
                    this.open_file(path, None, cx);
                    if let Some(offset) = offset {
                        this.reveal_offset(offset, cx);
                    }
                }
            }))
            .on_mouse_down(gpui::MouseButton::Right, move |_, _, cx| {
                if let Some(path) = path_for_right_click.clone() {
                    entity.update(cx, |ws, cx| {
                        ws.right_dock_context_menu = Some(path);
                        cx.notify();
                    });
                }
            })
            .child(
                div()
                    .text_size(px(FONT_UI_SMALLER))
                    .text_color(theme.text_muted)
                    .line_height(relative(1.3))
                    .child(text),
            )
            .children(chevron)
            .children(chip)
            .children(button)
    }
}
