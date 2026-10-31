//! A list to pick one thing out of: the template chooser, for now.
//!
//! Obsidian's `insert-template` opens a suggester over the template
//! folder's notes, by basename and sorted, with the placeholder
//! `Type name of a template...` (see
//! [[Specification#Observed templates behavior, Obsidian 1.14.4]]). This is
//! that, kept separate from the quick switcher because it picks from a
//! given list rather than from the vault.

use crate::text_input::{InputEvent, TextInput};
use crate::theme::Theme;
use gpui::{
    div, prelude::*, px, uniform_list, App, Context, Entity, EventEmitter, FocusHandle, Focusable,
    FontWeight, HighlightStyle, ScrollStrategy, SharedString, StyledText, Subscription,
    UniformListScrollHandle, Window,
};
use silica_search::fuzzy_match;
use serde_json::{json, Value};
use std::ops::Range;

/// One thing that can be picked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// What the row reads, and what the query matches.
    pub label: String,
    /// What the pick means; a vault path for the template chooser.
    pub value: String,
}

pub enum ChooserEvent {
    Chosen(String),
    Close,
}

struct Match {
    row: Row,
    ranges: Vec<Range<usize>>,
}

pub struct Chooser {
    input: Entity<TextInput>,
    rows: Vec<Row>,
    matches: Vec<Match>,
    selected: usize,
    /// What the list says when it is empty, which is how a template folder
    /// that is unset or has nothing in it is reported.
    empty: SharedString,
    scroll: UniformListScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ChooserEvent> for Chooser {}

impl Focusable for Chooser {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl Chooser {
    pub fn new(cx: &mut Context<Self>) -> Chooser {
        let input = cx.new(|cx| TextInput::new("", cx));
        let subscriptions =
            vec![
                cx.subscribe(&input, |this, _, event: &InputEvent, cx| match event {
                    InputEvent::Changed => this.refresh(cx),
                    InputEvent::Submit => this.choose(cx),
                    InputEvent::Cancel => cx.emit(ChooserEvent::Close),
                    InputEvent::Up => this.select(this.selected.saturating_sub(1), cx),
                    InputEvent::Down => this.select(this.selected + 1, cx),
                }),
            ];
        Chooser {
            input,
            rows: Vec::new(),
            matches: Vec::new(),
            selected: 0,
            empty: "Nothing to choose".into(),
            scroll: UniformListScrollHandle::new(),
            _subscriptions: subscriptions,
        }
    }

    /// Shows the list and focuses the query.
    pub fn open(
        &mut self,
        rows: Vec<Row>,
        placeholder: &str,
        empty: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.rows = rows;
        self.empty = empty.into();
        self.input.update(cx, |i, cx| {
            i.set_placeholder(placeholder);
            i.set_text("", cx);
        });
        window.focus(&self.input.focus_handle(cx), cx);
        self.refresh(cx);
    }

    /// Types `query` and submits it, for `--template`: the same path a
    /// keystroke takes, so a screenshot run exercises the real matching.
    pub fn pick(&mut self, query: &str, cx: &mut Context<Self>) {
        self.input.update(cx, |i, cx| i.set_text(query, cx));
        self.refresh(cx);
        self.choose(cx);
    }

    /// What the chooser is showing, for the control socket's state dump. The
    /// list has no error line of its own, so `error` carries the message that
    /// stands in for the rows when nothing matches, which is the only thing a
    /// caller could otherwise not see.
    pub fn describe(&self, rows: usize, cx: &App) -> Value {
        let total = self.matches.len();
        let shown: Vec<&str> = self
            .matches
            .iter()
            .take(rows)
            .map(|m| m.row.label.as_str())
            .collect();
        json!({
            "query": self.input.read(cx).text(),
            "selected": self.selected,
            "rows": shown,
            "rows_total": if total > shown.len() { Some(total) } else { None },
            "error": if total == 0 { Some(&self.empty) } else { None },
        })
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let query = self.input.read(cx).text().trim().to_string();
        let mut scored: Vec<(f64, Match)> = Vec::new();
        for row in &self.rows {
            if query.is_empty() {
                scored.push((
                    0.,
                    Match {
                        row: row.clone(),
                        ranges: Vec::new(),
                    },
                ));
                continue;
            }
            if let Some(m) = fuzzy_match(&query, &row.label) {
                scored.push((
                    m.score,
                    Match {
                        row: row.clone(),
                        ranges: m.ranges,
                    },
                ));
            }
        }
        if !query.is_empty() {
            scored.sort_by(|a, b| {
                b.0.partial_cmp(&a.0)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.1.row.label.cmp(&b.1.row.label))
            });
        }
        self.matches = scored.into_iter().map(|(_, m)| m).collect();
        self.selected = 0;
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        if self.matches.is_empty() {
            return;
        }
        self.selected = ix.min(self.matches.len() - 1);
        self.scroll
            .scroll_to_item(self.selected, ScrollStrategy::Center);
        cx.notify();
    }

    fn choose(&mut self, cx: &mut Context<Self>) {
        match self.matches.get(self.selected) {
            Some(m) => cx.emit(ChooserEvent::Chosen(m.row.value.clone())),
            None => cx.emit(ChooserEvent::Close),
        }
    }

    fn render_rows(
        &mut self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<gpui::Stateful<gpui::Div>> {
        let theme = cx.global::<Theme>().clone();
        range
            .filter_map(|ix| {
                let m = self.matches.get(ix)?;
                let selected = ix == self.selected;
                let highlights: Vec<_> = m
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
                let label = StyledText::new(SharedString::from(m.row.label.clone()))
                    .with_highlights(highlights);
                Some(
                    div()
                        .id(("chooser-row", ix))
                        .w_full()
                        .h(px(34.))
                        .px_3()
                        .flex()
                        .flex_row()
                        .items_center()
                        .rounded_sm()
                        .cursor_pointer()
                        .when(selected, |d| d.bg(theme.bg_active))
                        .hover(|s| s.bg(theme.bg_alt))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.selected = ix;
                            this.choose(cx);
                        }))
                        .child(
                            div()
                                .text_size(px(14.))
                                .text_color(theme.text)
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .text_ellipsis()
                                .child(label),
                        ),
                )
            })
            .collect()
    }
}

impl Render for Chooser {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let count = self.matches.len();
        let rows = (count.min(12) as f32) * 34.;
        div()
            .id("chooser")
            .w(px(620.))
            .flex()
            .flex_col()
            .bg(theme.bg_secondary)
            .border_1()
            .border_color(theme.border)
            .rounded_lg()
            .shadow_lg()
            .p_2()
            .on_mouse_down_out(cx.listener(|_, _, _, cx| cx.emit(ChooserEvent::Close)))
            .child(self.input.clone())
            .when(count > 0, |d| {
                d.child(
                    uniform_list("chooser-list", count, cx.processor(Self::render_rows))
                        .track_scroll(&self.scroll)
                        .h(px(rows))
                        .mt_2(),
                )
            })
            .when(count == 0, |d| {
                d.child(
                    div()
                        .p_3()
                        .text_size(px(12.))
                        .text_color(theme.text_muted)
                        .child(self.empty.clone()),
                )
            })
    }
}
