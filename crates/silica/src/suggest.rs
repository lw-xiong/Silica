//! The autocompletion popup: the rows the editor is offering, and where
//! they are drawn.
//!
//! What triggers it, what it offers and what accepting a row writes all
//! live in `silica-suggest`, free of gpui and tested against Obsidian's own
//! rows. This module is the window onto that: a list anchored under the
//! cursor, the selected row, and the mouse.

use crate::editor::Editor;
use crate::theme::Theme;
use gpui::{
    div, prelude::*, px, Context, FontWeight, HighlightStyle, Pixels, SharedString, StyledText,
};
use silica_suggest::{Accept, Item, Suggestion};
use std::ops::Range;

/// Height of one row.
const ROW: f32 = 26.0;
/// Rows drawn before the list starts scrolling with the selection; also
/// what the page keys move the selection by.
pub(crate) const VISIBLE: usize = 12;
const WIDTH: f32 = 380.0;

/// The popup as the editor holds it.
pub struct SuggestState {
    /// The line the query is on, and the byte columns accepting it replaces.
    pub line: usize,
    pub range: Range<usize>,
    pub items: Vec<Suggestion>,
    pub selected: usize,
}

impl SuggestState {
    /// Moves the selection by `delta`, clamped rather than wrapped, the way
    /// Obsidian's list moves.
    pub fn step(&mut self, delta: isize) {
        if self.items.is_empty() {
            return;
        }
        let last = self.items.len() - 1;
        let next = self.selected as isize + delta;
        self.selected = next.clamp(0, last as isize) as usize;
    }

    pub fn select_first(&mut self) {
        self.selected = 0;
    }

    pub fn select_last(&mut self) {
        self.selected = self.items.len().saturating_sub(1);
    }

    /// The window of rows drawn, which follows the selection.
    fn window(&self) -> Range<usize> {
        if self.items.len() <= VISIBLE {
            return 0..self.items.len();
        }
        let half = VISIBLE / 2;
        let start = self
            .selected
            .saturating_sub(half)
            .min(self.items.len() - VISIBLE);
        start..start + VISIBLE
    }

    fn height(&self) -> Pixels {
        px(self.window().len() as f32 * ROW + 8.)
    }
}

/// Where the popup sits: under the cursor, or above it when there is no
/// room below.
pub struct Anchor {
    /// The cursor's left edge, relative to the editor's own bounds.
    pub x: Pixels,
    /// The top of the cursor's line.
    pub top: Pixels,
    /// The bottom of the cursor's line.
    pub bottom: Pixels,
    /// The editor's own size, for keeping the popup inside it.
    pub width: Pixels,
    pub height: Pixels,
}

/// Draws the popup for `state` at `anchor`.
pub fn render(
    state: &SuggestState,
    anchor: &Anchor,
    theme: &Theme,
    cx: &mut Context<Editor>,
) -> impl IntoElement {
    let height = state.height();
    // Below the line unless it would fall out of the view.
    let below = anchor.bottom + height <= anchor.height;
    let top = if below {
        anchor.bottom
    } else {
        (anchor.top - height).max(px(0.))
    };
    let left = anchor.x.min((anchor.width - px(WIDTH)).max(px(0.)));
    let window = state.window();
    let rows = window.map(|ix| {
        let item = &state.items[ix];
        let selected = ix == state.selected;
        let highlights: Vec<(Range<usize>, HighlightStyle)> = item
            .ranges
            .iter()
            .map(|r| {
                (
                    r.clone(),
                    HighlightStyle {
                        color: Some(theme.accent),
                        font_weight: Some(FontWeight::BOLD),
                        ..HighlightStyle::default()
                    },
                )
            })
            .collect();
        let label =
            StyledText::new(SharedString::from(item.label.clone())).with_highlights(highlights);
        let muted = matches!(item.item, Item::Literal);
        div()
            .id(("suggest-row", ix))
            .w_full()
            .h(px(ROW))
            .px_2()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap_2()
            .rounded_sm()
            .cursor_pointer()
            .when(selected, |d| d.bg(theme.bg_active))
            .hover(|s| s.bg(theme.bg_alt))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    this.accept_suggestion_at(ix, Accept::Whole, window, cx);
                }),
            )
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(if muted { theme.text_muted } else { theme.text })
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(label),
            )
            .children(item.detail.clone().map(|d| {
                div()
                    .flex_shrink_0()
                    .text_size(px(11.))
                    .text_color(theme.text_muted)
                    .child(SharedString::from(d))
            }))
    });
    div()
        .absolute()
        .left(left)
        .top(top)
        .w(px(WIDTH))
        .flex()
        .flex_col()
        .p_1()
        .bg(theme.bg_secondary)
        .border_1()
        .border_color(theme.border)
        .rounded_md()
        .shadow_lg()
        .children(rows)
}
