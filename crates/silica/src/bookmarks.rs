//! The Bookmarks pane: `.obsidian/bookmarks.json` as a tree in the left
//! dock, the way the Bookmarks core plugin shows it — a header of actions
//! and one row per item, groups collapsible, the open note's row marked.
//! The pane only shows and asks; the workspace owns the file and does the
//! opening, so every click comes out as a [`BookmarkEvent`].

use crate::theme::Theme;
use gpui::{
    anchored, deferred, div, prelude::*, px, Context, EventEmitter, Pixels, Point, SharedString,
    Window,
};
use silica_formats::bookmarks::Bookmark;
use serde_json::{json, Value};
use std::collections::HashSet;

pub enum BookmarkEvent {
    /// A file bookmark: the note, scrolled to its `#Heading` or `#^block`.
    Open {
        path: String,
        subpath: Option<String>,
    },
    /// A search bookmark: the Search pane with this query.
    Search(String),
    Graph,
    /// A folder bookmark: shown in the Files pane.
    Folder(String),
    Url(String),
    /// "Bookmark the active tab...".
    AddCurrent,
    /// "New group".
    NewGroup,
    /// Remove the item at this index path (indexes into groups).
    Remove(Vec<usize>),
}

/// One visible row: the item, where it sits, and how deep.
struct Row {
    at: Vec<usize>,
    depth: usize,
    item: Bookmark,
    collapsed: bool,
}

struct Menu {
    position: Point<Pixels>,
    at: Vec<usize>,
}

pub struct BookmarksPane {
    items: Vec<Bookmark>,
    /// Index paths of the groups folded shut.
    collapsed: HashSet<Vec<usize>>,
    /// The open note, whose row is marked.
    active: Option<String>,
    menu: Option<Menu>,
}

impl EventEmitter<BookmarkEvent> for BookmarksPane {}

impl BookmarksPane {
    pub fn new(_cx: &mut Context<Self>) -> BookmarksPane {
        BookmarksPane {
            items: Vec::new(),
            collapsed: HashSet::new(),
            active: None,
            menu: None,
        }
    }

    pub fn set_items(&mut self, items: Vec<Bookmark>, cx: &mut Context<Self>) {
        self.items = items;
        cx.notify();
    }

    pub fn set_active(&mut self, path: Option<String>, cx: &mut Context<Self>) {
        if self.active != path {
            self.active = path;
            cx.notify();
        }
    }

    /// "Collapse all" folds every group; a second press opens them.
    fn toggle_all(&mut self, cx: &mut Context<Self>) {
        if self.collapsed.is_empty() {
            let mut groups = Vec::new();
            fn walk(items: &[Bookmark], at: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
                for (ix, item) in items.iter().enumerate() {
                    if item.kind() == "group" {
                        at.push(ix);
                        out.push(at.clone());
                        walk(&item.items(), at, out);
                        at.pop();
                    }
                }
            }
            walk(&self.items, &mut Vec::new(), &mut groups);
            self.collapsed = groups.into_iter().collect();
        } else {
            self.collapsed.clear();
        }
        cx.notify();
    }

    fn rows(&self) -> Vec<Row> {
        fn walk(
            items: &[Bookmark],
            at: &mut Vec<usize>,
            depth: usize,
            collapsed: &HashSet<Vec<usize>>,
            out: &mut Vec<Row>,
        ) {
            for (ix, item) in items.iter().enumerate() {
                at.push(ix);
                let folded = collapsed.contains(at);
                let members = (item.kind() == "group").then(|| item.items());
                out.push(Row {
                    at: at.clone(),
                    depth,
                    item: item.clone(),
                    collapsed: folded,
                });
                if let Some(members) = members {
                    if !folded {
                        walk(&members, at, depth + 1, collapsed, out);
                    }
                }
                at.pop();
            }
        }
        let mut out = Vec::new();
        walk(&self.items, &mut Vec::new(), 0, &self.collapsed, &mut out);
        out
    }

    /// The glyph standing in for Obsidian's icon per kind.
    fn glyph(item: &Bookmark) -> &'static str {
        match item.kind() {
            "file" => match item.subpath() {
                Some(s) if s.starts_with("#^") => "¶",
                Some(_) => "#",
                None => "▢",
            },
            "folder" => "▭",
            "search" => "⌕",
            "graph" => "◎",
            "url" => "⊕",
            "group" => "",
            _ => "·",
        }
    }

    fn activate(&mut self, row: &Row, cx: &mut Context<Self>) {
        let item = &row.item;
        match item.kind() {
            "group" => {
                if !self.collapsed.remove(&row.at) {
                    self.collapsed.insert(row.at.clone());
                }
                cx.notify();
            }
            "file" => {
                if let Some(path) = item.path() {
                    cx.emit(BookmarkEvent::Open {
                        path: path.to_string(),
                        subpath: item.subpath().map(str::to_string),
                    });
                }
            }
            "search" => cx.emit(BookmarkEvent::Search(
                item.query().unwrap_or("").to_string(),
            )),
            "graph" => cx.emit(BookmarkEvent::Graph),
            "folder" => cx.emit(BookmarkEvent::Folder(item.path().unwrap_or("").to_string())),
            "url" => {
                if let Some(url) = item.url() {
                    cx.emit(BookmarkEvent::Url(url.to_string()));
                }
            }
            _ => {}
        }
    }

    /// What the pane shows, for the control socket's state dump.
    pub fn describe(&self, rows: usize) -> Value {
        let all = self.rows();
        let total = all.len();
        let listed: Vec<Value> = all
            .iter()
            .take(rows)
            .map(|r| {
                json!({
                    "at": r.at,
                    "depth": r.depth,
                    "kind": r.item.kind(),
                    "title": r.item.display_title(),
                    "path": r.item.path(),
                    "subpath": r.item.subpath(),
                    "query": r.item.query(),
                    "collapsed": (r.item.kind() == "group").then_some(r.collapsed),
                    "active": r.item.path().is_some_and(|p| self.active.as_deref() == Some(p))
                        && r.item.kind() == "file",
                })
            })
            .collect();
        json!({
            "count": self.items.len(),
            "rows": listed,
            "rows_total": (total > rows).then_some(total),
        })
    }

    fn render_menu(&mut self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let menu = self.menu.as_ref()?;
        let theme = cx.global::<Theme>().clone();
        let at = menu.at.clone();
        let list = div()
            .id("bookmarks-menu")
            .flex()
            .flex_col()
            .min_w(px(160.))
            .py_1()
            .bg(theme.bg_secondary)
            .border_1()
            .border_color(theme.border)
            .rounded_md()
            .shadow_lg()
            .text_size(px(13.))
            .text_color(theme.text)
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.menu = None;
                cx.notify();
            }))
            .child(
                div()
                    .id("bookmarks-menu-remove")
                    .px_3()
                    .py_1()
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.bg_alt))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.menu = None;
                        cx.emit(BookmarkEvent::Remove(at.clone()));
                        cx.notify();
                    }))
                    .child("Remove"),
            );
        Some(
            deferred(
                anchored()
                    .position(menu.position)
                    .snap_to_window_with_margin(px(8.))
                    .child(list),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }
}

impl Render for BookmarksPane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let button =
            |id: &'static str, glyph: &'static str, event: fn(&mut Self, &mut Context<Self>)| {
                div()
                    .id(id)
                    .w(px(24.))
                    .h(px(24.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_sm()
                    .cursor_pointer()
                    .text_size(px(14.))
                    .text_color(theme.text_muted)
                    .hover(|s| s.bg(theme.bg_alt).text_color(theme.text))
                    .on_click(cx.listener(move |this, _, _, cx| event(this, cx)))
                    .child(glyph)
            };
        // Obsidian's header, in its order: "Bookmark the active tab...",
        // "New group", "Collapse all". Its fourth button, the search
        // filter, is not built.
        let header = div()
            .flex()
            .flex_row()
            .items_center()
            .gap_1()
            .px_2()
            .py_1()
            .child(button("bookmark-add", "+", |_, cx| {
                cx.emit(BookmarkEvent::AddCurrent)
            }))
            .child(button("bookmark-group", "▣", |_, cx| {
                cx.emit(BookmarkEvent::NewGroup)
            }))
            .child(button("bookmark-collapse", "⇅", |this, cx| {
                this.toggle_all(cx)
            }));
        let rows = self.rows();
        let empty = rows.is_empty();
        let list = div()
            .id("bookmarks-list")
            .flex()
            .flex_col()
            .flex_1()
            .overflow_y_scroll()
            .px_1()
            .when(empty, |d| {
                d.child(
                    div()
                        .p_3()
                        .text_size(px(12.))
                        .text_color(theme.text_muted)
                        .child("No bookmarks yet"),
                )
            })
            .children(rows.into_iter().enumerate().map(|(ix, row)| {
                let active = row.item.kind() == "file"
                    && row.item.subpath().is_none()
                    && row
                        .item
                        .path()
                        .is_some_and(|p| self.active.as_deref() == Some(p));
                let title: SharedString = row.item.display_title().into();
                let is_group = row.item.kind() == "group";
                let marker: &'static str = if is_group {
                    if row.collapsed {
                        "▸"
                    } else {
                        "▾"
                    }
                } else {
                    Self::glyph(&row.item)
                };
                let at = row.at.clone();
                let row = std::rc::Rc::new(row);
                let click_row = row.clone();
                div()
                    .id(("bookmark", ix))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .h(px(24.))
                    .pl(px(8. + 14. * row.depth as f32))
                    .pr_2()
                    .rounded_sm()
                    .cursor_pointer()
                    .text_size(px(13.))
                    .when(active, |d| d.bg(theme.bg_active))
                    .hover(|s| s.bg(theme.bg_alt))
                    .on_click(cx.listener(move |this, _, _, cx| this.activate(&click_row, cx)))
                    .on_mouse_down(
                        gpui::MouseButton::Right,
                        cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                            this.menu = Some(Menu {
                                position: event.position,
                                at: at.clone(),
                            });
                            cx.notify();
                        }),
                    )
                    .child(
                        div()
                            .w(px(14.))
                            .flex_shrink_0()
                            .text_color(theme.text_muted)
                            .text_size(px(11.))
                            .child(marker),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .text_ellipsis()
                            .text_color(theme.text)
                            .child(title),
                    )
            }));
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(header)
            .child(list)
            .children(self.render_menu(cx))
    }
}
