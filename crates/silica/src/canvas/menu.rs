//! The canvas's menus: a row of icon buttons that floats above the
//! selection (and sits at the bottom of the view as the card menu), and the
//! right-click list anchored at the pointer.
//!
//! This is the mechanism only. Which items Obsidian shows in each menu is
//! being probed; the lists `view.rs` builds from these pieces are
//! placeholders until that list arrives. Every button names an item id that
//! [`CanvasView::menu_pick`] dispatches on, so a menu's contents are a
//! `Vec` of items and nothing else.

use crate::assets::icon;
use crate::canvas::view::CanvasView;
use crate::theme::Theme;
use gpui::{
    anchored, deferred, div, prelude::*, px, AnyElement, AnyView, App, Context, Div, Pixels, Point,
    Render, SharedString, Window,
};

/// A button in an icon row: the id the pick reports, its Lucide icon, and
/// the tooltip.
#[derive(Debug, Clone, Copy)]
pub struct IconItem {
    pub id: &'static str,
    pub icon: &'static str,
    pub label: &'static str,
}

/// An entry of a right-click list: the id the pick reports and its text.
#[derive(Debug, Clone, Copy)]
pub struct ListItem {
    pub id: &'static str,
    pub label: &'static str,
}

/// A button of an icon row, screen pixels.
pub const ICON_BUTTON: f32 = 32.0;
/// Gap between buttons and the row's padding.
pub const ICON_GAP: f32 = 0.0;
pub const ICON_PAD: f32 = 0.0;
/// A list entry whose label is this draws a separator line instead.
pub const SEPARATOR: &str = "---";

/// The width an icon row of `n` buttons takes.
pub fn icon_row_width(n: usize) -> f32 {
    n as f32 * ICON_BUTTON + n.saturating_sub(1) as f32 * ICON_GAP + 2.0 * ICON_PAD
}

/// The height of an icon row.
pub fn icon_row_height() -> f32 {
    ICON_BUTTON + 2.0 * ICON_PAD
}

/// The small label a button shows on hover.
pub struct Tooltip {
    text: SharedString,
}

impl Render for Tooltip {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        div()
            .px_2()
            .py_1()
            .rounded_md()
            .bg(theme.bg_secondary)
            .border_1()
            .border_color(theme.border)
            .shadow_md()
            .text_size(px(12.))
            .text_color(theme.text)
            .child(self.text.clone())
    }
}

/// A tooltip builder for `.tooltip(..)`.
pub fn tooltip(text: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView {
    let text: SharedString = text.into();
    move |_, cx| cx.new(|_| Tooltip { text: text.clone() }).into()
}

/// A row of icon buttons; the caller positions it. `prefix` keys the
/// buttons' element ids, so two rows on screen do not share hover state.
pub fn icon_row(
    prefix: &'static str,
    items: &[IconItem],
    theme: &Theme,
    cx: &mut Context<CanvasView>,
) -> Div {
    let mut row = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(ICON_GAP))
        .p(px(ICON_PAD))
        .rounded(px(4.))
        .bg(theme.canvas.background)
        .border_1()
        .border_color(theme.canvas.control_border)
        .shadow_md()
        .occlude();
    for (ix, item) in items.iter().enumerate() {
        let id = item.id;
        row = row.child(
            div()
                .id((prefix, ix))
                .w(px(ICON_BUTTON))
                .h(px(ICON_BUTTON))
                .flex()
                .items_center()
                .justify_center()
                .rounded_sm()
                .cursor_pointer()
                .hover(|s| s.bg(theme.bg_alt))
                .tooltip(tooltip(item.label))
                .on_click(cx.listener(move |this, _, window, cx| this.menu_pick(id, window, cx)))
                .child(icon(
                    item.icon,
                    crate::theme::ICON_SIZE,
                    theme.canvas.control_fg,
                )),
        );
    }
    row
}

/// The right-click list at `position`, drawn over everything else in the
/// style of the file explorer's menu. A press outside it closes it.
pub fn context_menu(
    items: &[ListItem],
    position: Point<Pixels>,
    theme: &Theme,
    cx: &mut Context<CanvasView>,
) -> AnyElement {
    let mut list = div()
        .id("canvas-context-menu")
        .flex()
        .flex_col()
        .min_w(px(200.))
        .py_1()
        .bg(theme.bg_secondary)
        .border_1()
        .border_color(theme.border)
        .rounded_md()
        .shadow_lg()
        .text_size(px(13.))
        .text_color(theme.text)
        .occlude()
        .on_mouse_down_out(cx.listener(|this, _, _, cx| this.close_context_menu(cx)));
    for (ix, item) in items.iter().enumerate() {
        let id = item.id;
        if item.label == SEPARATOR {
            list = list.child(div().my_1().h(px(1.)).bg(theme.border));
            continue;
        }
        list = list.child(
            div()
                .id(("canvas-context-item", ix))
                .px_3()
                .py_1()
                .cursor_pointer()
                .hover(|s| s.bg(theme.bg_alt))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.close_context_menu(cx);
                    this.menu_pick(id, window, cx);
                }))
                .child(item.label),
        );
    }
    deferred(
        anchored()
            .position(position)
            .snap_to_window_with_margin(px(8.))
            .child(list),
    )
    .with_priority(1)
    .into_any_element()
}
