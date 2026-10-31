//! The quick switcher (Ctrl+O): fuzzy search over file names, aliases and
//! paths, recent files first while the query is empty, unresolved link
//! targets shown as "Not created yet"; Enter on one of those, or on a query
//! nothing matches, creates the note.

use crate::text_input::{InputEvent, TextInput};
use crate::theme::Theme;
use crate::workspace::VaultState;
use gpui::{
    div, prelude::*, px, uniform_list, App, Context, Entity, EventEmitter, FocusHandle, Focusable,
    FontWeight, HighlightStyle, ScrollStrategy, SharedString, StyledText, Subscription,
    UniformListScrollHandle, Window,
};
use silica_cache::frontmatter_aliases;
use silica_formats::config::SwitcherConfig;
use silica_formats::FileKind;
use silica_search::fuzzy_match;
use serde_json::{json, Value};
use std::ops::Range;
use std::sync::Arc;

const MAX_RESULTS: usize = 50;

/// Whether `switcher.json` lets a file of this kind be offered.
///
/// "Show attachments" off hides images, audio, video and PDFs and leaves
/// notes, canvases and bases. "Show all file types" is the switcher's own
/// gate on the kinds Obsidian cannot open, and it sits behind the
/// vault-wide "Show all file types" in Files and links: a file the vault
/// does not list at all cannot be offered here either.
fn offers(kind: FileKind, show_unsupported: bool, switcher: &SwitcherConfig) -> bool {
    match kind {
        FileKind::Other => show_unsupported && switcher.show_all_file_types.unwrap_or(false),
        FileKind::Image | FileKind::Audio | FileKind::Video | FileKind::Pdf => {
            switcher.show_attachments.unwrap_or(true)
        }
        FileKind::Markdown | FileKind::Canvas | FileKind::Base => true,
    }
}

/// Whether the switcher offers link targets no file resolves to, the rows
/// labelled "Not created yet". "Show existing only" hides them.
fn offers_unresolved(switcher: &SwitcherConfig) -> bool {
    !switcher.show_existing_only.unwrap_or(false)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ItemKind {
    File,
    Alias,
    /// A link target no file resolves to; opening it would create the note.
    Unresolved,
}

#[derive(Debug, Clone)]
struct Candidate {
    /// Vault path, or the link text for an unresolved target.
    path: String,
    /// What is matched and shown: the file name without `.md`, or the alias.
    label: String,
    kind: ItemKind,
}

#[derive(Debug, Clone)]
struct Item {
    candidate: Candidate,
    ranges: Vec<Range<usize>>,
}

pub enum SwitcherEvent {
    Open(String),
    /// Enter on a target no file resolves to, or on a query nothing
    /// matches: create the note with that name.
    Create(String),
    Close,
}

pub struct QuickSwitcher {
    input: Entity<TextInput>,
    state: Option<Arc<VaultState>>,
    candidates: Vec<Candidate>,
    recent: Vec<String>,
    items: Vec<Item>,
    selected: usize,
    scroll: UniformListScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<SwitcherEvent> for QuickSwitcher {}

impl Focusable for QuickSwitcher {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl QuickSwitcher {
    pub fn new(cx: &mut Context<Self>) -> QuickSwitcher {
        let input = cx.new(|cx| TextInput::new("Find or create a note…", cx));
        let subscriptions =
            vec![
                cx.subscribe(&input, |this, _, event: &InputEvent, cx| match event {
                    InputEvent::Changed => this.refresh(cx),
                    InputEvent::Submit => this.open_selected(cx),
                    InputEvent::Cancel => cx.emit(SwitcherEvent::Close),
                    InputEvent::Up => this.select(this.selected.saturating_sub(1), cx),
                    InputEvent::Down => this.select(this.selected + 1, cx),
                }),
            ];
        QuickSwitcher {
            input,
            state: None,
            candidates: Vec::new(),
            recent: Vec::new(),
            items: Vec::new(),
            selected: 0,
            scroll: UniformListScrollHandle::new(),
            _subscriptions: subscriptions,
        }
    }

    pub fn set_state(&mut self, state: Arc<VaultState>, cx: &mut Context<Self>) {
        let mut candidates = Vec::new();
        let show_unsupported = state.app().show_unsupported_files.unwrap_or(false);
        let switcher = state.switcher().clone();
        let vault = state.vault();
        let cache = state.cache();
        for file in vault.files() {
            if !offers(file.kind, show_unsupported, &switcher) {
                continue;
            }
            let name = file.path.rsplit('/').next().unwrap_or(&file.path);
            let label = if file.kind == FileKind::Markdown {
                name.trim_end_matches(".md").to_string()
            } else {
                name.to_string()
            };
            candidates.push(Candidate {
                path: file.path.clone(),
                label,
                kind: ItemKind::File,
            });
            if let Some(meta) = cache.file(&file.path) {
                for alias in frontmatter_aliases(meta) {
                    candidates.push(Candidate {
                        path: file.path.clone(),
                        label: alias,
                        kind: ItemKind::Alias,
                    });
                }
            }
        }
        let mut unresolved: Vec<String> = if offers_unresolved(&switcher) {
            cache
                .files()
                .flat_map(|(p, _)| {
                    cache
                        .unresolved_links(p)
                        .map(|m| m.keys().cloned().collect::<Vec<_>>())
                        .unwrap_or_default()
                })
                .filter(|t| !t.is_empty())
                .collect()
        } else {
            Vec::new()
        };
        unresolved.sort();
        unresolved.dedup();
        for target in unresolved {
            candidates.push(Candidate {
                path: target.clone(),
                label: target,
                kind: ItemKind::Unresolved,
            });
        }
        drop(vault);
        drop(cache);
        self.candidates = candidates;
        self.state = Some(state);
        self.refresh(cx);
    }

    /// Shows the switcher with `recent` (most recent first) and focuses the input.
    pub fn open(
        &mut self,
        recent: Vec<String>,
        query: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.recent = recent;
        self.input.update(cx, |i, cx| i.set_text(query, cx));
        window.focus(&self.input.focus_handle(cx), cx);
        self.refresh(cx);
    }

    /// What the switcher is showing, for the control socket's state dump. The
    /// switcher has no error line, so `error` is always null; it is in the
    /// dump so that every modal reports the same keys.
    pub fn describe(&self, rows: usize, cx: &App) -> Value {
        let total = self.items.len();
        let shown: Vec<&str> = self
            .items
            .iter()
            .take(rows)
            .map(|item| item.candidate.label.as_str())
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
        let mut items: Vec<(f64, Item)> = Vec::new();
        if query.is_empty() {
            for (ix, path) in self.recent.iter().enumerate() {
                if let Some(c) = self
                    .candidates
                    .iter()
                    .find(|c| c.kind == ItemKind::File && &c.path == path)
                {
                    items.push((
                        -(ix as f64),
                        Item {
                            candidate: c.clone(),
                            ranges: Vec::new(),
                        },
                    ));
                }
            }
        } else {
            for c in &self.candidates {
                let by_label = fuzzy_match(&query, &c.label);
                let by_path = if c.kind == ItemKind::File {
                    fuzzy_match(&query, &c.path).map(|m| (m.score - 2.0, m))
                } else {
                    None
                };
                let best = match (by_label, by_path) {
                    (Some(l), Some((ps, _))) if l.score >= ps => Some((l.score, l.ranges)),
                    // A path match highlights nothing in the label.
                    (_, Some((ps, _))) => Some((ps, Vec::new())),
                    (Some(l), None) => Some((l.score, l.ranges)),
                    (None, None) => None,
                };
                if let Some((score, ranges)) = best {
                    let penalty = match c.kind {
                        ItemKind::File => 0.0,
                        ItemKind::Alias => 0.5,
                        ItemKind::Unresolved => 3.0,
                    };
                    items.push((
                        score - penalty,
                        Item {
                            candidate: c.clone(),
                            ranges,
                        },
                    ));
                }
            }
            items.sort_by(|a, b| {
                b.0.partial_cmp(&a.0)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.1.candidate.label.len().cmp(&b.1.candidate.label.len()))
                    .then_with(|| a.1.candidate.path.cmp(&b.1.candidate.path))
            });
        }
        self.items = items
            .into_iter()
            .map(|(_, i)| i)
            .take(MAX_RESULTS)
            .collect();
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

    fn open_selected(&mut self, cx: &mut Context<Self>) {
        match self.items.get(self.selected) {
            Some(item) if item.candidate.kind == ItemKind::Unresolved => {
                cx.emit(SwitcherEvent::Create(item.candidate.path.clone()));
            }
            Some(item) => cx.emit(SwitcherEvent::Open(item.candidate.path.clone())),
            None => {
                let query = self.input.read(cx).text().trim().to_string();
                if !query.is_empty() {
                    cx.emit(SwitcherEvent::Create(query));
                }
            }
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
                let c = &item.candidate;
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
                let label = StyledText::new(SharedString::from(c.label.clone()))
                    .with_highlights(highlights);
                let detail: Option<SharedString> = match c.kind {
                    ItemKind::File => c
                        .path
                        .rsplit_once('/')
                        .map(|(dir, _)| dir.to_string().into()),
                    ItemKind::Alias => Some(
                        format!(
                            "alias of {}",
                            c.path
                                .rsplit('/')
                                .next()
                                .unwrap_or(&c.path)
                                .trim_end_matches(".md")
                        )
                        .into(),
                    ),
                    ItemKind::Unresolved => Some("Not created yet".into()),
                };
                Some(
                    div()
                        .id(("switcher-row", ix))
                        .w_full()
                        .h(px(34.))
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
                            this.open_selected(cx);
                        }))
                        .child(
                            div()
                                .text_size(px(14.))
                                .text_color(if c.kind == ItemKind::Unresolved {
                                    theme.text_muted
                                } else {
                                    theme.text
                                })
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .text_ellipsis()
                                .child(label),
                        )
                        .children(detail.map(|d| {
                            div()
                                .flex_shrink_0()
                                .text_size(px(11.))
                                .text_color(theme.text_muted)
                                .child(d)
                        })),
                )
            })
            .collect()
    }
}

impl Render for QuickSwitcher {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let count = self.items.len();
        let rows = (count.min(12) as f32) * 34.;
        div()
            .id("quick-switcher")
            .w(px(620.))
            .flex()
            .flex_col()
            .bg(theme.bg_secondary)
            .border_1()
            .border_color(theme.border)
            .rounded_lg()
            .shadow_lg()
            .p_2()
            .on_mouse_down_out(cx.listener(|_, _, _, cx| cx.emit(SwitcherEvent::Close)))
            .child(self.input.clone())
            .when(count > 0, |d| {
                d.child(
                    uniform_list("switcher-list", count, cx.processor(Self::render_rows))
                        .track_scroll(&self.scroll)
                        .h(px(rows))
                        .mt_2(),
                )
            })
            .when(count == 0 && !self.input.read(cx).text().is_empty(), |d| {
                d.child(
                    div()
                        .p_3()
                        .text_size(px(12.))
                        .text_color(theme.text_muted)
                        .child("No matching files. Enter to create."),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(existing: bool, attachments: bool, all_types: bool) -> SwitcherConfig {
        SwitcherConfig {
            show_existing_only: Some(existing),
            show_attachments: Some(attachments),
            show_all_file_types: Some(all_types),
            ..SwitcherConfig::default()
        }
    }

    /// A vault with no `switcher.json`: notes, canvases, bases and
    /// attachments are offered, unknown extensions are not, and the
    /// not-created-yet targets are.
    #[test]
    fn the_defaults_match_a_vault_with_no_switcher_json() {
        let d = SwitcherConfig::default();
        assert!(offers(FileKind::Markdown, false, &d));
        assert!(offers(FileKind::Canvas, false, &d));
        assert!(offers(FileKind::Base, false, &d));
        assert!(offers(FileKind::Image, false, &d));
        assert!(offers(FileKind::Pdf, false, &d));
        assert!(!offers(FileKind::Other, true, &d));
        assert!(offers_unresolved(&d));
    }

    #[test]
    fn show_attachments_off_hides_the_attachment_kinds_only() {
        let c = config(false, false, false);
        for kind in [
            FileKind::Image,
            FileKind::Audio,
            FileKind::Video,
            FileKind::Pdf,
        ] {
            assert!(!offers(kind, true, &c), "{kind:?} was still offered");
        }
        assert!(offers(FileKind::Markdown, true, &c));
        assert!(offers(FileKind::Canvas, true, &c));
    }

    /// The switcher's own "Show all file types" has to be on *and* the
    /// vault has to be listing unsupported files at all.
    #[test]
    fn an_unknown_extension_needs_both_gates() {
        assert!(!offers(FileKind::Other, false, &config(false, true, false)));
        assert!(!offers(FileKind::Other, true, &config(false, true, false)));
        assert!(!offers(FileKind::Other, false, &config(false, true, true)));
        assert!(offers(FileKind::Other, true, &config(false, true, true)));
    }

    #[test]
    fn show_existing_only_hides_the_unresolved_targets() {
        assert!(!offers_unresolved(&config(true, true, true)));
        assert!(offers_unresolved(&config(false, true, true)));
    }
}
