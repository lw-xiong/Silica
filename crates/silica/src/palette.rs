//! The command palette (`command-palette:open`, Ctrl+P): fuzzy search over
//! the names of the commands the workspace can run, with their hotkeys.
//! While the query is empty, pinned commands (`command-palette.json`) come
//! first, then the commands used this session, then the rest in Obsidian's
//! registration order.

use crate::text_input::{InputEvent, TextInput};
use crate::theme::Theme;
use gpui::{
    div, prelude::*, px, uniform_list, App, Context, Entity, EventEmitter, FocusHandle, Focusable,
    FontWeight, HighlightStyle, ScrollStrategy, SharedString, StyledText, Subscription,
    UniformListScrollHandle, Window,
};
use silica_commands::{hotkey_label, Command};
use silica_search::fuzzy_match;
use serde_json::{json, Value};
use std::ops::Range;

const MAX_RESULTS: usize = 60;
const ROW_HEIGHT: f32 = 32.;

/// One command the palette can offer.
#[derive(Debug, Clone)]
pub struct PaletteCommand {
    pub id: String,
    pub name: String,
    pub hotkeys: Vec<String>,
    pub pinned: bool,
}

impl PaletteCommand {
    pub fn from_command(command: &Command, pinned: bool) -> PaletteCommand {
        PaletteCommand {
            id: command.id.clone(),
            name: command.name.clone(),
            hotkeys: command.hotkeys.iter().map(hotkey_label).collect(),
            pinned,
        }
    }
}

#[derive(Debug, Clone)]
struct Item {
    command: PaletteCommand,
    ranges: Vec<Range<usize>>,
}

pub enum PaletteEvent {
    Run(String),
    Close,
}

pub struct CommandPalette {
    input: Entity<TextInput>,
    commands: Vec<PaletteCommand>,
    /// Command ids run this session, most recent first.
    recent: Vec<String>,
    items: Vec<Item>,
    selected: usize,
    scroll: UniformListScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<PaletteEvent> for CommandPalette {}

impl Focusable for CommandPalette {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl CommandPalette {
    pub fn new(cx: &mut Context<Self>) -> CommandPalette {
        let input = cx.new(|cx| TextInput::new("Select a command...", cx));
        let subscriptions =
            vec![
                cx.subscribe(&input, |this, _, event: &InputEvent, cx| match event {
                    InputEvent::Changed => this.refresh(cx),
                    InputEvent::Submit => this.run_selected(cx),
                    InputEvent::Cancel => cx.emit(PaletteEvent::Close),
                    InputEvent::Up => this.select(this.selected.saturating_sub(1), cx),
                    InputEvent::Down => this.select(this.selected + 1, cx),
                }),
            ];
        CommandPalette {
            input,
            commands: Vec::new(),
            recent: Vec::new(),
            items: Vec::new(),
            selected: 0,
            scroll: UniformListScrollHandle::new(),
            _subscriptions: subscriptions,
        }
    }

    /// Shows the palette over `commands` with `recent` ids (most recent
    /// first) and focuses the input.
    pub fn open(
        &mut self,
        commands: Vec<PaletteCommand>,
        recent: Vec<String>,
        query: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.commands = commands;
        self.recent = recent;
        self.input.update(cx, |i, cx| i.set_text(query, cx));
        window.focus(&self.input.focus_handle(cx), cx);
        self.refresh(cx);
    }

    /// What the palette is showing, for the control socket's state dump. The
    /// rows are command names, the same text the query matches; `error` is
    /// always null, and is here only so every modal reports the same keys.
    pub fn describe(&self, rows: usize, cx: &App) -> Value {
        let total = self.items.len();
        let shown: Vec<&str> = self
            .items
            .iter()
            .take(rows)
            .map(|item| item.command.name.as_str())
            .collect();
        json!({
            "query": self.input.read(cx).text(),
            "selected": self.selected,
            "rows": shown,
            "rows_total": if total > shown.len() { Some(total) } else { None },
            "error": Value::Null,
        })
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let query = self.input.read(cx).text().trim().to_string();
        let mut items: Vec<Item> = Vec::new();
        if query.is_empty() {
            let plain = |c: &PaletteCommand| Item {
                command: c.clone(),
                ranges: Vec::new(),
            };
            let mut seen: Vec<&str> = Vec::new();
            for c in self.commands.iter().filter(|c| c.pinned) {
                seen.push(&c.id);
                items.push(plain(c));
            }
            for id in &self.recent {
                if seen.contains(&id.as_str()) {
                    continue;
                }
                if let Some(c) = self.commands.iter().find(|c| &c.id == id) {
                    seen.push(&c.id);
                    items.push(plain(c));
                }
            }
            for c in &self.commands {
                if !seen.contains(&c.id.as_str()) {
                    items.push(plain(c));
                }
            }
        } else {
            let mut scored: Vec<(f64, Item)> = self
                .commands
                .iter()
                .filter_map(|c| {
                    let m = fuzzy_match(&query, &c.name)?;
                    Some((
                        m.score,
                        Item {
                            command: c.clone(),
                            ranges: m.ranges,
                        },
                    ))
                })
                .collect();
            scored.sort_by(|a, b| {
                b.0.partial_cmp(&a.0)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.1.command.name.len().cmp(&b.1.command.name.len()))
            });
            items = scored.into_iter().map(|(_, i)| i).collect();
        }
        items.truncate(MAX_RESULTS);
        self.items = items;
        self.selected = 0;
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        if self.items.is_empty() {
            return;
        }
        self.selected = ix.min(self.items.len() - 1);
        self.scroll
            .scroll_to_item(self.selected, ScrollStrategy::Center);
        cx.notify();
    }

    fn run_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(item) = self.items.get(self.selected) {
            cx.emit(PaletteEvent::Run(item.command.id.clone()));
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
                let item = self.items.get(ix)?;
                let selected = ix == self.selected;
                let c = &item.command;
                let highlights: Vec<_> = item
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
                let name =
                    StyledText::new(SharedString::from(c.name.clone())).with_highlights(highlights);
                let hotkeys = c.hotkeys.clone();
                Some(
                    div()
                        .id(("palette-row", ix))
                        .w_full()
                        .h(px(ROW_HEIGHT))
                        .px_3()
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap_3()
                        .rounded_sm()
                        .cursor_pointer()
                        .when(selected, |d| d.bg(theme.bg_active))
                        .hover(|s| s.bg(theme.bg_alt))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.selected = ix;
                            this.run_selected(cx);
                        }))
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap_2()
                                .min_w_0()
                                .when(c.pinned, |d| {
                                    d.child(
                                        div()
                                            .flex_shrink_0()
                                            .text_size(px(10.))
                                            .text_color(theme.text_muted)
                                            .child("PINNED"),
                                    )
                                })
                                .child(
                                    div()
                                        .text_size(px(14.))
                                        .text_color(theme.text)
                                        .whitespace_nowrap()
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .child(name),
                                ),
                        )
                        .child(
                            div().flex().flex_row().flex_shrink_0().gap_1().children(
                                hotkeys.into_iter().map(|label| hotkey_chip(label, &theme)),
                            ),
                        ),
                )
            })
            .collect()
    }
}

/// A key combination drawn the way the palette and the Hotkeys tab show it.
pub fn hotkey_chip(label: String, theme: &Theme) -> gpui::Div {
    div()
        .px(px(6.))
        .py(px(1.))
        .rounded_sm()
        .bg(theme.bg_alt)
        .border_1()
        .border_color(theme.border)
        .text_size(px(11.))
        .text_color(theme.text_muted)
        .whitespace_nowrap()
        .child(SharedString::from(label))
}

impl Render for CommandPalette {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let count = self.items.len();
        let rows = (count.min(14) as f32) * ROW_HEIGHT;
        let instruction = |keys: &'static str, text: &'static str, theme: &Theme| {
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_1()
                .child(
                    div()
                        .text_color(theme.text)
                        .font_weight(FontWeight::BOLD)
                        .child(keys),
                )
                .child(div().text_color(theme.text_muted).child(text))
        };
        div()
            .id("command-palette")
            .w(px(620.))
            .flex()
            .flex_col()
            .bg(theme.bg_secondary)
            .border_1()
            .border_color(theme.border)
            .rounded_lg()
            .shadow_lg()
            .p_2()
            .on_mouse_down_out(cx.listener(|_, _, _, cx| cx.emit(PaletteEvent::Close)))
            .child(self.input.clone())
            .when(count > 0, |d| {
                d.child(
                    uniform_list("palette-list", count, cx.processor(Self::render_rows))
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
                        .child("No commands found."),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap_4()
                    .px_3()
                    .pt_2()
                    .pb_1()
                    .mt_1()
                    .border_t_1()
                    .border_color(theme.border)
                    .text_size(px(11.))
                    .child(instruction("↑↓", "to navigate", &theme))
                    .child(instruction("↵", "to use", &theme))
                    .child(instruction("esc", "to dismiss", &theme)),
            )
    }
}
