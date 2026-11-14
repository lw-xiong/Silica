//! The Flashcards and AI chat panes: two of Silica's own right-dock tabs.
//!
//! All the logic lives in the `silica-flashcards` and `silica-ai` crates;
//! this module is gpui state and rendering only. Cards are parsed from the
//! vault's texts as `#flashcard` blocks, graded with the crate's SM-2 and
//! persisted to `<vault>/.obsidian/silica-flashcards.json`. Chat turns go to
//! the `SILICA_AI_*`-configured model server on the background executor, with
//! the question's `[[links]]` and the open note as context.

use super::*;
use gpui::ClipboardItem;

use std::path::PathBuf;

use silica_ai as ai;
use silica_flashcards as fc;

/// How many notes may back one chat turn as context.
const AI_CONTEXT_NOTES: usize = 8;
/// Character cap on the context block sent with a chat turn.
const AI_CONTEXT_CHARS: usize = 8_000;

#[derive(Default)]
pub(crate) struct FlashcardState {
    /// Every card parsed from the vault, refreshed when the pane opens.
    pub(crate) cards: Vec<fc::Card>,
    /// Ids of the cards due now, in due order.
    pub(crate) queue: Vec<String>,
    /// The card on screen.
    pub(crate) current: Option<String>,
    pub(crate) show_answer: bool,
    /// Where the review store lives, once the vault is known.
    pub(crate) store_path: Option<PathBuf>,
    pub(crate) store: fc::Store,
    pub(crate) notice: Option<String>,
}

/// One chat turn: a question or an answer, rendered with `ai::parse_inline`.
#[derive(Clone)]
pub(crate) struct AiTurn {
    pub(crate) user: bool,
    pub(crate) text: String,
}

#[derive(Default)]
pub(crate) struct AiState {
    pub(crate) turns: Vec<AiTurn>,
    pub(crate) sending: bool,
    pub(crate) notice: Option<String>,
}

impl Workspace {
    /// `<vault>/.silica-flashcards.json`: the review store lives at the
    /// vault root under the crate's own `STORE_NAME`.
    fn flashcard_store_path(&self) -> Option<PathBuf> {
        let state = self.state.as_deref()?;
        Some(state.vault().root().join(fc::STORE_NAME))
    }

    /// How many cards are due today, from the pane's last load; zero
    /// until the pane has been opened once. Drives the status bar.
    pub(crate) fn due_count(&self) -> usize {
        if self.flashcard_state.cards.is_empty() {
            return 0;
        }
        let today = chrono::Local::now().date_naive();
        fc::due_cards(
            &self.flashcard_state.cards,
            &self.flashcard_state.store,
            today,
        )
        .len()
    }

    /// Parses every card in the vault and rebuilds the due queue. Called when
    /// the pane opens, on the refresh button, and after each grade.
    pub(crate) fn reload_flashcards(&mut self) {
        let Some(state) = self.state.as_deref() else {
            return;
        };
        let texts = state.texts();
        let mut cards = Vec::new();
        for path in texts.keys() {
            cards.extend(fc::parse_cards(path, texts.get(path).map_or("", |t| t.as_str())));
        }
        cards.sort_by(|a, b| a.source.cmp(&b.source).then(a.id.cmp(&b.id)));
        let store_path = self.flashcard_store_path();
        let store = match &store_path {
            Some(path) => fc::load(path).unwrap_or_default(),
            None => fc::Store::default(),
        };
        let today = chrono::Local::now().date_naive();
        let queue: Vec<String> = fc::due_cards(&cards, &store, today)
            .into_iter()
            .map(|c| c.id.clone())
            .collect();
        self.flashcard_state.cards = cards;
        self.flashcard_state.store_path = store_path;
        self.flashcard_state.store = store;
        self.flashcard_state.queue = queue;
        self.flashcard_state.current = None;
        self.flashcard_state.show_answer = false;
    }

    fn current_flashcard(&self) -> Option<&fc::Card> {
        let id = self.flashcard_state.current.as_ref()?;
        self.flashcard_state.cards.iter().find(|c| &c.id == id)
    }

    /// The next due card moves to the screen.
    fn next_flashcard(&mut self) {
        let next = self.flashcard_state.queue.first().cloned();
        if let Some(id) = next {
            self.flashcard_state.queue.remove(0);
            self.flashcard_state.current = Some(id);
            self.flashcard_state.show_answer = false;
        } else {
            self.flashcard_state.current = None;
        }
    }

    pub(crate) fn reveal_flashcard_answer(&mut self, cx: &mut Context<Self>) {
        if self.flashcard_state.current.is_some() {
            self.flashcard_state.show_answer = true;
            cx.notify();
        }
    }

    /// Grades the card on screen and persists the store: 0 again, 1 hard,
    /// 2 good, 3 easy — the buttons and the `workspace::FlashcardGrade`
    /// action both land here.
    /// Keyboard path to the pane's grade buttons: opens the flashcards
    /// tab (loading cards if needed) and grades the card on screen.
    pub(crate) fn grade_flashcard_key(&mut self, grade: u8, cx: &mut Context<Self>) {
        self.select_right_tab(RightTab::Flashcards, cx);
        if self.flashcard_state.cards.is_empty() {
            self.reload_flashcards();
        }
        self.grade_flashcard(grade, cx);
    }

    pub(crate) fn grade_flashcard(&mut self, grade: u8, cx: &mut Context<Self>) {
        let Some(card) = self.current_flashcard().cloned() else {
            return;
        };
        let grade = match grade {
            0 => fc::Grade::Again,
            1 => fc::Grade::Hard,
            3 => fc::Grade::Easy,
            _ => fc::Grade::Good,
        };
        fc::review(
            &mut self.flashcard_state.store,
            &card.id,
            grade,
            chrono::Local::now().date_naive(),
        );
        if let Some(path) = self.flashcard_state.store_path.clone() {
            if let Err(err) = fc::save(&path, &self.flashcard_state.store) {
                self.flashcard_state.notice = Some(format!("Could not save the review store: {err}"));
            } else {
                self.flashcard_state.notice = None;
            }
        }
        self.next_flashcard();
        cx.notify();
    }

    pub(crate) fn on_flashcard_grade(
        &mut self,
        action: &FlashcardGrade,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.right_tab == RightTab::Flashcards {
            self.grade_flashcard(action.grade, cx);
        }
    }

    /// Sends the input's text as a chat turn, with the question's `[[links]]`
    /// and the open note as context. The request runs on the background
    /// executor so the window never blocks on the model.
    /// `silica:ai-summarize-note`: asks the configured AI backend for a
    /// short bullet summary of the current note and inserts it at the
    /// cursor. The buffer wins over disk when the note is dirty.
    pub(crate) fn summarize_current_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(current) = self.current.clone() else {
            self.report("No note is open", cx);
            return;
        };
        let text = {
            let editor = self.editor.read(cx);
            if editor.name == current && editor.is_dirty() {
                Some(editor.file_contents())
            } else {
                self.state.as_deref().and_then(|state| {
                    state
                        .texts()
                        .get(&current)
                        .map(|t| t.to_string())
                        .or_else(|| std::fs::read_to_string(state.vault().absolute(&current)).ok())
                })
            }
        };
        let Some(text) = text else {
            self.report(format!("{} could not be read", basename(&current)), cx);
            return;
        };
        let config = match ai::Config::from_env() {
            Ok(c) => c,
            Err(err) => {
                self.report(format!("AI: {err}"), cx);
                return;
            }
        };
        let model = config.model.clone();
        let truncated: String = text.chars().take(8000).collect();
        let request = ai::ChatRequest {
            model,
            messages: vec![
                ai::ChatMessage::system(
                    "Summarize the note in 3-5 short bullet points. Markdown bullets only, no preamble.",
                ),
                ai::ChatMessage::user(truncated),
            ],
            temperature: Some(0.2),
            max_tokens: None,
        };
        self.ai_state.sending = true;
        cx.notify();
        let editor = self.editor.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { ai::chat(&config, &request) })
                .await;
            this.update_in(cx, |this, window, cx| {
                this.ai_state.sending = false;
                match result {
                    Ok(reply) => {
                        let block = format!("## Summary\n\n{}\n\n", reply.text.trim());
                        editor.update(cx, |e, cx| e.insert_at_cursors(&block, window, cx));
                        this.report("Summary inserted", cx);
                    }
                    Err(err) => this.report(format!("AI: {err}"), cx),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn ai_send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ai_state.sending {
            return;
        }
        let question = self.ai_input.read(cx).text().trim().to_string();
        if question.is_empty() {
            return;
        }
        self.ai_input
            .update(cx, |input, cx| input.set_text("", cx));
        // * A non-empty editor selection rides along as quoted context,
        //   so "what does this mean?" works without copy-pasting.
        let selection = {
            let editor = self.editor.read(cx);
            let sel = editor.selection();
            if sel.is_empty() {
                None
            } else {
                Some(editor.text()[sel].trim().to_string())
            }
        };
        let question = match selection {
            Some(sel) if !sel.is_empty() => format!("{question}\n\nSelected text:\n{sel}"),
            _ => question,
        };
        self.ai_state.turns.push(AiTurn {
            user: true,
            text: question.clone(),
        });
        self.ai_state.sending = true;
        self.ai_state.notice = None;
        cx.notify();

        let notes = self.ai_context(&question);
        let config = ai::Config::from_env();
        let model = config
            .as_ref()
            .map(|c| c.model.clone())
            .unwrap_or_else(|_| ai::DEFAULT_MODEL.to_string());
        let mut messages = ai::context_messages(&notes, AI_CONTEXT_CHARS);
        messages.push(ai::ChatMessage::user(question));
        let request = ai::ChatRequest {
            model,
            messages,
            temperature: Some(0.2),
            max_tokens: None,
        };
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    match &config {
                        Ok(config) => ai::chat(config, &request),
                        Err(err) => Err(err.clone()),
                    }
                })
                .await;
            this.update_in(cx, |this, _window, cx| {
                this.ai_state.sending = false;
                match result {
                    Ok(reply) => this.ai_state.turns.push(AiTurn {
                        user: false,
                        text: reply.text,
                    }),
                    Err(err) => this.ai_state.notice = Some(format!("AI: {err}")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The notes that back a question: the open note plus every note a
    /// `[[link]]` in the question resolves to, capped at `AI_CONTEXT_NOTES`.
    fn ai_context(&self, question: &str) -> Vec<(String, String)> {
        let Some(state) = self.state.as_deref() else {
            return Vec::new();
        };
        let texts = state.texts();
        let cache = state.cache();
        let mut paths: Vec<String> = Vec::new();
        if let Some(current) = self.current.clone() {
            if texts.contains_key(&current) {
                paths.push(current);
            }
        }
        for link in question
            .split("[[")
            .skip(1)
            .filter_map(|rest| rest.split("]]").next())
        {
            let link = link.split('|').next().unwrap_or("").trim();
            if link.is_empty() {
                continue;
            }
            let source = paths.first().cloned().unwrap_or_default();
            if let Some(target) = cache.resolve(link, &source) {
                let target = target.to_string();
                if !paths.contains(&target) && texts.contains_key(&target) {
                    paths.push(target);
                }
            }
            if paths.len() >= AI_CONTEXT_NOTES {
                break;
            }
        }
        paths.truncate(AI_CONTEXT_NOTES);
        paths
            .into_iter()
            .filter_map(|path| texts.get(&path).map(|text| (path, text.as_str().to_string())))
            .collect()
    }

    pub(crate) fn render_flashcards_pane(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        if self.flashcard_state.store_path.is_none() {
            self.reload_flashcards();
        }
        if self.flashcard_state.current.is_none() && !self.flashcard_state.queue.is_empty() {
            self.next_flashcard();
        }
        let due = self.flashcard_state.queue.len();
        let total = self.flashcard_state.cards.len();
        let header = Self::pane_header().child(
            Self::nav_button(
                "flashcards-refresh",
                "rotate-ccw",
                "Re-scan the vault for cards",
                false,
                theme,
            )
            .on_click(cx.listener(|this, _, _, cx| {
                this.reload_flashcards();
                this.next_flashcard();
                cx.notify();
            })),
        );
        let body: gpui::AnyElement = match self.current_flashcard().cloned() {
            None => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(12.))
                .text_color(theme.text_muted)
                .child(if total == 0 {
                    "No #flashcard blocks in the vault".to_string()
                } else {
                    format!("All {total} cards reviewed — {due} still due")
                })
                .into_any_element(),
            Some(card) => {
                let show_answer = self.flashcard_state.show_answer;
                let front = card.front();
                let back = card.back();
                let mut pane = div()
                    .id("flashcards")
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .p(px(12.))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(theme.text_muted)
                            .child({
                                let today = chrono::Local::now().date_naive();
                                let learned = self
                                    .flashcard_state
                                    .cards
                                    .iter()
                                    .filter(|c| {
                                        self.flashcard_state
                                            .store
                                            .cards
                                            .get(&c.id)
                                            .and_then(|sc| sc.due.as_deref())
                                            .and_then(|d| {
                                                chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()
                                            })
                                            .is_some_and(|due| due > today)
                                    })
                                    .count();
                                format!(
                                    "{due} due · {learned} learned · {total} in the vault · {card_source}",
                                    card_source = card.source
                                )
                            }),
                    )
                    .child(
                        div().id("flashcard-front").text_size(px(14.)).child(front),
                    );
                if show_answer && !back.is_empty() {
                    pane = pane.child(
                        div()
                            .id("flashcard-back")
                            .text_size(px(13.))
                            .text_color(theme.text_muted)
                            .border_t_1()
                            .border_color(theme.border)
                            .pt(px(8.))
                            .child(back),
                    );
                }
                let mut buttons = div().flex().flex_row().gap(px(6.)).pt(px(4.));
                if !show_answer {
                    buttons = buttons.child(
                        Self::nav_button(
                            "flashcard-show",
                            "book-open",
                            "Show the answer",
                            false,
                            theme,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.reveal_flashcard_answer(cx))),
                    );
                } else {
                    for (grade, id, label) in [
                        (0u8, "flashcard-grade-0", "Again"),
                        (1, "flashcard-grade-1", "Hard"),
                        (2, "flashcard-grade-2", "Good"),
                        (3, "flashcard-grade-3", "Easy"),
                    ] {
                        buttons = buttons.child(
                            Self::nav_button(id, "check", label, false, theme).on_click(
                                cx.listener(move |this, _, _, cx| this.grade_flashcard(grade, cx)),
                            ),
                        );
                    }
                }
                pane.child(buttons).into_any_element()
            }
        };
        let mut pane = div()
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .child(header)
            .child(body);
        if let Some(notice) = self.flashcard_state.notice.clone() {
            pane = pane.child(
                div()
                    .text_size(px(11.))
                    .text_color(theme.text_muted)
                    .child(notice),
            );
        }
        pane.into_any_element()
    }

    pub(crate) fn render_ai_pane(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let turns = self.ai_state.turns.clone();
        let sending = self.ai_state.sending;
        let notice = self.ai_state.notice.clone();
        let mut list = div()
            .id("ai-chat")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(8.))
            .p(px(12.));
        if turns.is_empty() && notice.is_none() {
            list = list.child(
                div()
                    .text_size(px(12.))
                    .text_color(theme.text_muted)
                    .child("Ask about your notes — a [[link]] in the question pulls that note in as context"),
            );
        }
        for (ix, turn) in turns.iter().enumerate() {
            let text = turn.text.clone();
            let mut bubble = div()
                .max_w_full()
                .rounded_md()
                .p(px(8.))
                .text_size(px(13.))
                .child(render_segments(&turn.text, theme));
            if !turn.user {
                // * Assistant bubbles get a small copy affordance.
                bubble = bubble.child(
                    div()
                        .id(("ai-copy", ix))
                        .text_size(px(10.))
                        .text_color(theme.text_muted)
                        .cursor_pointer()
                        .child("copy")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                            this.ai_state.notice = Some("Copied to clipboard".into());
                            cx.notify();
                        })),
                );
            }
            list = list.child(if turn.user {
                bubble.self_end().bg(theme.bg_active)
            } else {
                bubble.self_start().bg(theme.bg_alt)
            });
        }
        if sending {
            list = list.child(
                div()
                    .text_size(px(12.))
                    .text_color(theme.text_muted)
                    .child("…"),
            );
        }
        if let Some(notice) = notice {
            list = list.child(
                div()
                    .text_size(px(11.))
                    .text_color(theme.text_muted)
                    .child(notice),
            );
        }
        let input = self.ai_input.clone();
        div()
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .child(Self::pane_header())
            .child(list)
            .child(
                div()
                    .p(px(8.))
                    .border_t_1()
                    .border_color(theme.border)
                    .child(input),
            )
            .into_any_element()
    }
}

/// Renders one reply as lines of inline segments: `[[links]]`, `` `code` ``,
/// `**bold**`, `*italic*`, and code fences as their own blocks.
fn render_segments(text: &str, theme: &Theme) -> gpui::AnyElement {
    fn inline_part(segment: ai::Segment, theme: &Theme) -> Option<gpui::AnyElement> {
        Some(match segment {
            ai::Segment::Text(text) if text.is_empty() => return None,
            ai::Segment::Text(text) => text.into_any_element(),
            ai::Segment::Link(target) => div()
                .text_color(theme.link)
                .child(format!("[[{target}]]"))
                .into_any_element(),
            ai::Segment::Code(code) => div()
                .font_family(theme.mono_font.clone())
                .text_size(px(12.))
                .bg(theme.code_bg)
                .rounded(px(3.))
                .px(px(4.))
                .child(code)
                .into_any_element(),
            ai::Segment::Bold(text) => div()
                .font_weight(gpui::FontWeight::BOLD)
                .child(text)
                .into_any_element(),
            ai::Segment::Italic(text) => div().italic().child(text).into_any_element(),
            ai::Segment::CodeBlock(_lang, code) => div()
                .w_full()
                .font_family(theme.mono_font.clone())
                .text_size(px(12.))
                .bg(theme.code_bg)
                .rounded_md()
                .p(px(6.))
                .child(code)
                .into_any_element(),
        })
    }
    fn flush(parts: &mut Vec<gpui::AnyElement>, children: &mut Vec<gpui::AnyElement>) {
        if parts.is_empty() {
            return;
        }
        let mut row = div().flex().flex_row().flex_wrap().gap(px(4.));
        for part in std::mem::take(parts) {
            row = row.child(part);
        }
        children.push(row.into_any_element());
    }
    let mut children: Vec<gpui::AnyElement> = Vec::new();
    let mut parts: Vec<gpui::AnyElement> = Vec::new();
    for segment in ai::parse_inline(text) {
        match segment {
            ai::Segment::Text(text) => {
                for (ix, line) in text.split('\n').enumerate() {
                    if ix > 0 {
                        flush(&mut parts, &mut children);
                    }
                    if !line.is_empty() {
                        parts.push(line.to_string().into_any_element());
                    }
                }
            }
            ai::Segment::CodeBlock(..) => {
                flush(&mut parts, &mut children);
                children.push(inline_part(segment, theme).expect("code block renders"));
            }
            other => {
                if let Some(part) = inline_part(other, theme) {
                    parts.push(part);
                }
            }
        }
    }
    flush(&mut parts, &mut children);
    let mut col = div().flex().flex_col().gap(px(4.)).w_full();
    for child in children {
        col = col.child(child);
    }
    col.into_any_element()
}
