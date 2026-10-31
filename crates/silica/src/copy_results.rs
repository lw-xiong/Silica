//! The "Copy search results" modal the Search pane's count menu opens: a
//! preview of the text, Show path, Link style and List prefix, and the two
//! buttons. Nothing reaches the clipboard until "Copy results" is pressed,
//! which also leaves the modal open; Escape and "Done" close it.
//!
//! Obsidian's preview is an editable textarea; this one is read-only, and
//! the two native `<select>`s are menus. The text itself comes from
//! [`copy_text`], which the spec's 24 observed combinations pin down.

use crate::search::{copy_text, menu, menu_item, toggle, CopyEntry, LinkStyle, ListPrefix};
use crate::theme::{Theme, FONT_UI_SMALL, FONT_UI_SMALLER};
use gpui::{
    actions, anchored, deferred, div, prelude::*, px, relative, Anchor, App, ClickEvent, Context,
    EventEmitter, FocusHandle, Focusable, FontWeight, KeyBinding, Pixels, Point, SharedString,
    Window,
};
use serde_json::{json, Value};

actions!(copy_results, [Dismiss]);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("escape", Dismiss, Some("CopyResults"))]);
}

pub enum CopyResultsEvent {
    /// "Copy results": the text to put on the clipboard.
    Copy(String),
    /// "Done", Escape or a click outside.
    Close,
}

/// Which select's menu is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Select {
    Link,
    Prefix,
}

pub struct CopyResults {
    focus_handle: FocusHandle,
    entries: Vec<CopyEntry>,
    show_path: bool,
    link: LinkStyle,
    prefix: ListPrefix,
    open: Option<Select>,
    menu_at: Point<Pixels>,
}

impl EventEmitter<CopyResultsEvent> for CopyResults {}

impl Focusable for CopyResults {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl CopyResults {
    pub fn new(cx: &mut Context<Self>) -> CopyResults {
        CopyResults {
            focus_handle: cx.focus_handle(),
            entries: Vec::new(),
            show_path: false,
            link: LinkStyle::default(),
            prefix: ListPrefix::default(),
            open: None,
            menu_at: Point::default(),
        }
    }

    /// The files to list, in the Search pane's order. The three controls
    /// start at their defaults each time, as Obsidian's modal does.
    pub fn open(&mut self, entries: Vec<CopyEntry>, cx: &mut Context<Self>) {
        self.entries = entries;
        self.show_path = false;
        self.link = LinkStyle::default();
        self.prefix = ListPrefix::default();
        self.open = None;
        cx.notify();
    }

    pub fn text(&self) -> String {
        copy_text(&self.entries, self.show_path, self.link, self.prefix)
    }

    pub fn describe(&self) -> Value {
        json!({
            "files": self.entries.len(),
            "show_path": self.show_path,
            "link_style": self.link.label(),
            "list_prefix": self.prefix.label(),
            "text": self.text(),
        })
    }

    fn dismiss(&mut self, _: &Dismiss, _: &mut Window, cx: &mut Context<Self>) {
        if self.open.is_some() {
            self.open = None;
            cx.notify();
        } else {
            cx.emit(CopyResultsEvent::Close);
        }
    }

    /// A `setting-item`: name and description at the left, the control at
    /// the right, a hairline above every row but the first.
    fn row(
        name: &'static str,
        desc: &'static str,
        first: bool,
        control: gpui::AnyElement,
        theme: &Theme,
    ) -> gpui::Div {
        div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap(px(24.))
            .py(px(12.))
            .when(!first, |d| d.border_t_1().border_color(theme.border))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(div().text_size(px(14.)).text_color(theme.text).child(name))
                    .child(
                        div()
                            .text_size(px(FONT_UI_SMALLER))
                            .text_color(theme.text_muted)
                            .child(desc),
                    ),
            )
            .child(div().flex_shrink_0().child(control))
    }

    /// A select in the shape of Obsidian's `dropdown`: the current label
    /// and a chevron pair on a bordered field, its options in a menu.
    fn select(
        &self,
        which: Select,
        label: &'static str,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let open = self.open == Some(which);
        let id = match which {
            Select::Link => "copy-results-link",
            Select::Prefix => "copy-results-prefix",
        };
        let field = div()
            .id(id)
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.))
            .h(px(30.))
            .pl(px(10.))
            .pr(px(8.))
            .rounded(px(5.))
            .bg(theme.bg_alt)
            .border_1()
            .border_color(theme.border)
            .cursor_pointer()
            .text_size(px(FONT_UI_SMALL))
            .text_color(theme.text)
            .when(open, |d| d.border_color(theme.accent))
            .hover(|s| s.bg(theme.bg_active))
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                this.menu_at = event.position();
                this.open = if this.open == Some(which) {
                    None
                } else {
                    Some(which)
                };
                cx.notify();
            }))
            .child(div().whitespace_nowrap().child(label))
            .child(crate::assets::icon(
                "chevrons-up-down",
                12.,
                theme.text_muted,
            ));
        let popup = open.then(|| {
            let mut list = menu(
                match which {
                    Select::Link => "copy-results-link-menu",
                    Select::Prefix => "copy-results-prefix-menu",
                },
                theme,
            )
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.open = None;
                cx.notify();
            }));
            match which {
                Select::Link => {
                    for (ix, style) in LinkStyle::ALL.into_iter().enumerate() {
                        list = list.child(
                            menu_item(
                                "copy-results-link-item",
                                ix,
                                None,
                                style.label(),
                                style == self.link,
                                theme,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.link = style;
                                    this.open = None;
                                    cx.notify();
                                },
                            )),
                        );
                    }
                }
                Select::Prefix => {
                    for (ix, prefix) in ListPrefix::ALL.into_iter().enumerate() {
                        list = list.child(
                            menu_item(
                                "copy-results-prefix-item",
                                ix,
                                None,
                                prefix.label(),
                                prefix == self.prefix,
                                theme,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.prefix = prefix;
                                    this.open = None;
                                    cx.notify();
                                },
                            )),
                        );
                    }
                }
            }
            deferred(
                anchored()
                    .position(self.menu_at)
                    .anchor(Anchor::TopRight)
                    .snap_to_window_with_margin(px(8.))
                    .child(list),
            )
            .with_priority(2)
            .into_any_element()
        });
        div()
            .relative()
            .child(field)
            .children(popup)
            .into_any_element()
    }
}

impl Render for CopyResults {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let text: SharedString = self.text().into();
        let show_path = self.show_path;
        let button = |id: &'static str, label: &'static str, primary: bool, theme: &Theme| {
            div()
                .id(id)
                .h(px(30.))
                .px(px(14.))
                .flex()
                .items_center()
                .rounded(px(5.))
                .cursor_pointer()
                .text_size(px(FONT_UI_SMALL))
                .when(primary, |d| d.bg(theme.accent).text_color(gpui::white()))
                .when(!primary, |d| {
                    d.bg(theme.bg_alt)
                        .text_color(theme.text)
                        .hover(|s| s.bg(theme.bg_active))
                })
                .child(label)
        };
        div()
            .id("copy-results")
            .key_context("CopyResults")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::dismiss))
            .w(px(560.))
            .flex()
            .flex_col()
            .bg(theme.bg_secondary)
            .border_1()
            .border_color(theme.border)
            .rounded_lg()
            .shadow_lg()
            .p_4()
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                if this.open.is_none() {
                    cx.emit(CopyResultsEvent::Close)
                }
            }))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .pb(px(12.))
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.text)
                            .child("Copy search results"),
                    )
                    .child(
                        div()
                            .id("copy-results-close")
                            .size(px(24.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(4.))
                            .cursor_pointer()
                            .hover(|s| s.bg(theme.bg_alt))
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(CopyResultsEvent::Close)))
                            .child(crate::assets::icon("x", 16., theme.text_muted)),
                    ),
            )
            .child(
                // The textarea: two rows tall in Obsidian, scrolling.
                div()
                    .id("copy-results-text")
                    .h(px(56.))
                    .overflow_y_scroll()
                    .px(px(10.))
                    .py(px(6.))
                    .rounded(px(5.))
                    .bg(theme.bg_alt)
                    .border_1()
                    .border_color(theme.border)
                    .text_size(px(FONT_UI_SMALL))
                    .line_height(relative(1.5))
                    .text_color(theme.text)
                    .child(text),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .pt(px(8.))
                    .child(Self::row(
                        "Show path",
                        "Display full path of the file rather than just the file name.",
                        true,
                        toggle(("copy-results-toggle", 0), show_path, &theme)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.show_path = !show_path;
                                cx.notify();
                            }))
                            .into_any_element(),
                        &theme,
                    ))
                    .child(Self::row(
                        "Link style",
                        "Optionally turn each file result in to a link.",
                        false,
                        self.select(Select::Link, self.link.label(), &theme, cx),
                        &theme,
                    ))
                    .child(Self::row(
                        "List prefix",
                        "Optionally add a list item prefix to each file result.",
                        false,
                        self.select(Select::Prefix, self.prefix.label(), &theme, cx),
                        &theme,
                    )),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_end()
                    .gap(px(8.))
                    .pt(px(12.))
                    .child(
                        button("copy-results-copy", "Copy results", true, &theme).on_click(
                            cx.listener(|this, _, _, cx| {
                                cx.emit(CopyResultsEvent::Copy(this.text()))
                            }),
                        ),
                    )
                    .child(
                        button("copy-results-done", "Done", false, &theme)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(CopyResultsEvent::Close))),
                    ),
            )
    }
}
