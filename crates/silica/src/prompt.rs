//! A small modal for file operations: a title, an optional text field, an
//! optional list of suggestions the field filters (the folder picker of
//! "Move file to..."), an error line, and one or two buttons. Enter submits,
//! Escape cancels; Up and Down walk the suggestions.

use crate::text_input::{InputEvent, TextInput};
use crate::theme::Theme;
use gpui::{
    actions, div, prelude::*, px, uniform_list, App, Context, Entity, EventEmitter, FocusHandle,
    Focusable, FontWeight, HighlightStyle, KeyBinding, ScrollStrategy, SharedString, StyledText,
    Subscription, UniformListScrollHandle, Window,
};
use silica_search::fuzzy_match;
use serde_json::{json, Value};
use std::ops::Range;

actions!(prompt, [Confirm, Dismiss]);

pub fn bind_keys(cx: &mut App) {
    let ctx = Some("Prompt");
    cx.bind_keys([
        KeyBinding::new("enter", Confirm, ctx),
        KeyBinding::new("escape", Dismiss, ctx),
    ]);
}

pub enum PromptEvent {
    /// Enter, or the primary button: the field's text, or the chosen
    /// suggestion when there is a list.
    Submit(String),
    /// The secondary button.
    Secondary,
    Cancel,
}

/// What the prompt asks for.
#[derive(Debug, Clone, Default)]
pub struct PromptSpec {
    pub title: SharedString,
    /// Shown under the title.
    pub detail: Option<SharedString>,
    /// `Some(initial text)` shows the text field.
    pub input: Option<String>,
    pub placeholder: SharedString,
    /// Candidates the field filters; Enter submits the selected one.
    pub suggestions: Vec<String>,
    pub confirm: SharedString,
    pub secondary: Option<SharedString>,
}

pub struct Prompt {
    focus_handle: FocusHandle,
    input: Entity<TextInput>,
    spec: PromptSpec,
    error: Option<SharedString>,
    filtered: Vec<(usize, Vec<Range<usize>>)>,
    selected: usize,
    scroll: UniformListScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<PromptEvent> for Prompt {}

impl Focusable for Prompt {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        if self.spec.input.is_some() {
            self.input.focus_handle(cx)
        } else {
            self.focus_handle.clone()
        }
    }
}

impl Prompt {
    pub fn new(cx: &mut Context<Self>) -> Prompt {
        let input = cx.new(|cx| TextInput::new("", cx));
        let subscriptions =
            vec![
                cx.subscribe(&input, |this, _, event: &InputEvent, cx| match event {
                    InputEvent::Changed => this.refresh(cx),
                    InputEvent::Submit => this.submit(cx),
                    InputEvent::Cancel => cx.emit(PromptEvent::Cancel),
                    InputEvent::Up => this.select(this.selected.saturating_sub(1), cx),
                    InputEvent::Down => this.select(this.selected + 1, cx),
                }),
            ];
        Prompt {
            focus_handle: cx.focus_handle(),
            input,
            spec: PromptSpec::default(),
            error: None,
            filtered: Vec::new(),
            selected: 0,
            scroll: UniformListScrollHandle::new(),
            _subscriptions: subscriptions,
        }
    }

    pub fn open(&mut self, spec: PromptSpec, window: &mut Window, cx: &mut Context<Self>) {
        self.error = None;
        let initial = spec.input.clone().unwrap_or_default();
        self.spec = spec;
        self.input.update(cx, |i, cx| {
            i.set_placeholder(self.spec.placeholder.clone());
            let prefilled = !initial.is_empty();
            i.set_text(initial, cx);
            // Rename opens on the current name, and Obsidian's inline rename
            // selects the whole basename so the first keystroke replaces it.
            if prefilled {
                i.select_all(cx);
            }
        });
        window.focus(&self.focus_handle(cx), cx);
        self.refresh(cx);
    }

    /// Keeps the prompt open and shows why the operation failed.
    pub fn set_error(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.error = Some(message.into());
        cx.notify();
    }

    /// What the prompt is asking, for the control socket's state dump: the
    /// field's text, the suggestions it filtered down to, and the error line
    /// a failed operation left on screen.
    pub fn describe(&self, rows: usize, cx: &App) -> Value {
        let total = self.filtered.len();
        let shown: Vec<&str> = self
            .filtered
            .iter()
            .take(rows)
            .filter_map(|(ix, _)| self.spec.suggestions.get(*ix).map(String::as_str))
            .collect();
        json!({
            "query": self.input.read(cx).text(),
            "selected": self.selected,
            "rows": shown,
            "rows_total": if total > shown.len() { Some(total) } else { None },
            "error": self.error,
        })
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let query = self.input.read(cx).text().trim().to_string();
        let mut scored: Vec<(f64, usize, Vec<Range<usize>>)> = Vec::new();
        for (ix, candidate) in self.spec.suggestions.iter().enumerate() {
            if query.is_empty() {
                scored.push((-(ix as f64), ix, Vec::new()));
            } else if let Some(m) = fuzzy_match(&query, candidate) {
                scored.push((m.score, ix, m.ranges));
            }
        }
        scored.sort_by(|a, b| {
            b.0.partial_cmp(&a.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.1.cmp(&b.1))
        });
        self.filtered = scored.into_iter().map(|(_, ix, r)| (ix, r)).collect();
        self.selected = 0;
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        if self.filtered.is_empty() {
            return;
        }
        self.selected = ix.min(self.filtered.len() - 1);
        self.scroll
            .scroll_to_item(self.selected, ScrollStrategy::Center);
        cx.notify();
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        let value = match self.filtered.get(self.selected) {
            Some((ix, _)) if !self.spec.suggestions.is_empty() => {
                self.spec.suggestions[*ix].clone()
            }
            _ => self.input.read(cx).text().to_string(),
        };
        cx.emit(PromptEvent::Submit(value));
    }

    fn confirm(&mut self, _: &Confirm, _: &mut Window, cx: &mut Context<Self>) {
        self.submit(cx);
    }

    fn dismiss(&mut self, _: &Dismiss, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(PromptEvent::Cancel);
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
                let (candidate_ix, ranges) = self.filtered.get(ix)?;
                let label = self.spec.suggestions.get(*candidate_ix)?.clone();
                let selected = ix == self.selected;
                let highlights: Vec<_> = ranges
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
                Some(
                    div()
                        .id(("prompt-row", ix))
                        .w_full()
                        .h(px(30.))
                        .px_3()
                        .flex()
                        .items_center()
                        .rounded_sm()
                        .cursor_pointer()
                        .text_size(px(13.))
                        .when(selected, |d| d.bg(theme.bg_active))
                        .hover(|s| s.bg(theme.bg_alt))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.selected = ix;
                            this.submit(cx);
                        }))
                        .child(
                            div()
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .text_ellipsis()
                                .child(
                                    StyledText::new(SharedString::from(label))
                                        .with_highlights(highlights),
                                ),
                        ),
                )
            })
            .collect()
    }
}

impl Render for Prompt {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let count = self.filtered.len();
        let rows = (count.min(10) as f32) * 30.;
        // The id and the click listener belong on the painted button rather
        // than on a bare wrapper around it: gpui hit-tests the element that
        // registered the listener, and the wrapper's box is not the box the
        // user aims at.
        let button = |id: &'static str, label: SharedString, primary: bool, theme: &Theme| {
            div()
                .id(id)
                .px_3()
                .py_1()
                .rounded_md()
                .cursor_pointer()
                .text_size(px(13.))
                .when(primary, |d| d.bg(theme.accent).text_color(gpui::white()))
                .when(!primary, |d| {
                    d.bg(theme.bg_alt)
                        .text_color(theme.text)
                        .hover(|s| s.bg(theme.bg_active))
                })
                .child(label)
        };
        div()
            .id("prompt")
            .key_context("Prompt")
            // gpui focuses a tracked handle on every mouse-down inside the
            // element, so this has to be the same handle `Focusable` hands
            // out: tracking the box's own handle while the field held the
            // keyboard meant a click anywhere in the prompt — on the field
            // itself, on a button — took the keyboard away from the field and
            // left the prompt typeable only after Escape and a fresh F2.
            .track_focus(&self.focus_handle(cx))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::dismiss))
            .w(px(480.))
            .flex()
            .flex_col()
            .gap_2()
            .bg(theme.bg_secondary)
            .border_1()
            .border_color(theme.border)
            .rounded_lg()
            .shadow_lg()
            .p_4()
            .on_mouse_down_out(cx.listener(|_, _, _, cx| cx.emit(PromptEvent::Cancel)))
            .child(
                div()
                    .text_size(px(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.text)
                    .child(self.spec.title.clone()),
            )
            .children(self.spec.detail.clone().map(|d| {
                div()
                    .text_size(px(13.))
                    .text_color(theme.text_muted)
                    .child(d)
            }))
            .when(self.spec.input.is_some(), |d| d.child(self.input.clone()))
            .when(count > 0, |d| {
                d.child(
                    uniform_list("prompt-list", count, cx.processor(Self::render_rows))
                        .track_scroll(&self.scroll)
                        .h(px(rows)),
                )
            })
            .children(self.error.clone().map(|e| {
                div()
                    .text_size(px(12.))
                    .text_color(gpui::hsla(0., 0.7, 0.6, 1.))
                    .child(e)
            }))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_end()
                    .gap_2()
                    .mt_1()
                    .children(self.spec.secondary.clone().map(|label| {
                        button("prompt-secondary", label, false, &theme)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(PromptEvent::Secondary)))
                    }))
                    .child(
                        button("prompt-cancel", "Cancel".into(), false, &theme)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(PromptEvent::Cancel))),
                    )
                    .child(
                        button("prompt-confirm", self.spec.confirm.clone(), true, &theme)
                            .on_click(cx.listener(|this, _, _, cx| this.submit(cx))),
                    ),
            )
    }
}
