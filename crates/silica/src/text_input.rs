//! A single-line text field for the quick switcher and search.
//!
//! Text comes in through GPUI's input handler so IME composition and dead
//! keys work; navigation and editing keys are actions. The field paints
//! itself with a canvas element: the line is shaped once per frame, scrolled
//! horizontally to keep the cursor visible, and the input handler is
//! registered with the painted bounds.
//!
//! The field carries a selection, the way the HTML inputs Obsidian's modals
//! are made of do: Shift with a motion extends it, Ctrl+A takes all of it,
//! Ctrl+C/X/V go through the system clipboard, and typing replaces it.

use crate::theme::Theme;
use gpui::{
    actions, canvas, div, fill, point, prelude::*, px, size, App, Bounds, ClipboardItem, Context,
    DispatchPhase, ElementInputHandler, EntityInputHandler, EventEmitter, FocusHandle, Focusable,
    KeyBinding, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, ShapedLine,
    SharedString, TextAlign, UTF16Selection, Window,
};
use std::ops::Range;

actions!(
    text_input,
    [
        Left,
        Right,
        Home,
        End,
        WordLeft,
        WordRight,
        SelectLeft,
        SelectRight,
        SelectHome,
        SelectEnd,
        SelectWordLeft,
        SelectWordRight,
        SelectAll,
        Backspace,
        Delete,
        DeleteWordBackward,
        Copy,
        Cut,
        Paste,
        Submit,
        Cancel,
        Up,
        Down,
        Clear,
        /// A chord the field eats so it reaches nothing else. Obsidian's
        /// inputs swallow Ctrl+E rather than letting the workspace toggle
        /// reading view with the keyboard in a field.
        Swallow,
    ]
);

pub fn bind_keys(cx: &mut App) {
    let ctx = Some("TextInput");
    cx.bind_keys([
        KeyBinding::new("left", Left, ctx),
        KeyBinding::new("right", Right, ctx),
        KeyBinding::new("home", Home, ctx),
        KeyBinding::new("end", End, ctx),
        KeyBinding::new("ctrl-left", WordLeft, ctx),
        KeyBinding::new("ctrl-right", WordRight, ctx),
        KeyBinding::new("shift-left", SelectLeft, ctx),
        KeyBinding::new("shift-right", SelectRight, ctx),
        KeyBinding::new("shift-home", SelectHome, ctx),
        KeyBinding::new("shift-end", SelectEnd, ctx),
        KeyBinding::new("ctrl-shift-left", SelectWordLeft, ctx),
        KeyBinding::new("ctrl-shift-right", SelectWordRight, ctx),
        KeyBinding::new("ctrl-a", SelectAll, ctx),
        // Obsidian swallows Ctrl+E inside a text input: it neither moves the
        // caret to the end of the field nor toggles reading view (see
        // [[Specification#Ctrl+E inside a text input does nothing]]). Binding
        // it to End was a divergence, and leaving it unbound is another --
        // the chord would reach the workspace and toggle the mode -- so the
        // field eats it.
        KeyBinding::new("ctrl-e", Swallow, ctx),
        KeyBinding::new("backspace", Backspace, ctx),
        KeyBinding::new("delete", Delete, ctx),
        KeyBinding::new("ctrl-backspace", DeleteWordBackward, ctx),
        KeyBinding::new("ctrl-c", Copy, ctx),
        KeyBinding::new("ctrl-x", Cut, ctx),
        KeyBinding::new("ctrl-v", Paste, ctx),
        KeyBinding::new("ctrl-u", Clear, ctx),
        KeyBinding::new("enter", Submit, ctx),
        KeyBinding::new("escape", Cancel, ctx),
        KeyBinding::new("up", Up, ctx),
        KeyBinding::new("down", Down, ctx),
    ]);
}

pub enum InputEvent {
    Changed,
    Submit,
    Cancel,
    Up,
    Down,
}

#[derive(Default, Clone)]
struct Layout {
    bounds: Bounds<Pixels>,
    line: Option<ShapedLine>,
    scroll_x: Pixels,
    line_height: Pixels,
}

/// The text and the selection in it, with no GPUI in sight so the editing
/// rules can be tested on their own.
#[derive(Default)]
struct Content {
    text: String,
    /// Byte offset of the selection head, where the caret is drawn.
    cursor: usize,
    /// The other end of the selection; equal to `cursor` when nothing is
    /// selected.
    anchor: usize,
}

impl Content {
    fn selection(&self) -> Range<usize> {
        self.cursor.min(self.anchor)..self.cursor.max(self.anchor)
    }

    fn has_selection(&self) -> bool {
        self.cursor != self.anchor
    }

    fn set_text(&mut self, text: String) {
        self.text = text;
        self.cursor = self.text.len();
        self.anchor = self.cursor;
    }

    fn select_all(&mut self) {
        self.anchor = 0;
        self.cursor = self.text.len();
    }

    fn replace(&mut self, range: Range<usize>, insert: &str) {
        self.text.replace_range(range.clone(), insert);
        self.cursor = range.start + insert.len();
        self.anchor = self.cursor;
    }

    fn move_to(&mut self, to: usize, extend: bool) {
        self.cursor = to;
        if !extend {
            self.anchor = to;
        }
    }

    /// A bare Left or Right collapses a selection onto the edge it points at
    /// rather than stepping from the head, which is what a native input does.
    fn step(&mut self, forward: bool, extend: bool) {
        let to = if !extend && self.has_selection() {
            let sel = self.selection();
            if forward {
                sel.end
            } else {
                sel.start
            }
        } else if forward {
            self.next_boundary(self.cursor)
        } else {
            self.prev_boundary(self.cursor)
        };
        self.move_to(to, extend);
    }

    fn step_word(&mut self, forward: bool, extend: bool) {
        let to = if forward {
            self.word_right(self.cursor)
        } else {
            self.word_left(self.cursor)
        };
        self.move_to(to, extend);
    }

    fn move_to_edge(&mut self, end: bool, extend: bool) {
        let to = if end { self.text.len() } else { 0 };
        self.move_to(to, extend);
    }

    /// What Backspace removes: the selection, or the character before the
    /// caret. The same shape serves Delete and Ctrl+Backspace.
    fn backspace_range(&self) -> Range<usize> {
        if self.has_selection() {
            self.selection()
        } else {
            self.prev_boundary(self.cursor)..self.cursor
        }
    }

    fn delete_range(&self) -> Range<usize> {
        if self.has_selection() {
            self.selection()
        } else {
            self.cursor..self.next_boundary(self.cursor)
        }
    }

    fn delete_word_range(&self) -> Range<usize> {
        if self.has_selection() {
            self.selection()
        } else {
            self.word_left(self.cursor)..self.cursor
        }
    }

    fn prev_boundary(&self, at: usize) -> usize {
        let mut i = at.saturating_sub(1);
        while i > 0 && !self.text.is_char_boundary(i) {
            i -= 1;
        }
        i
    }

    fn next_boundary(&self, at: usize) -> usize {
        let mut i = (at + 1).min(self.text.len());
        while i < self.text.len() && !self.text.is_char_boundary(i) {
            i += 1;
        }
        i
    }

    /// The start of the word before `at`: words are whitespace-delimited here,
    /// as they already were for Ctrl+Backspace.
    fn word_left(&self, at: usize) -> usize {
        let before = &self.text[..at];
        let word = &before[..before.trim_end().len()];
        word.char_indices()
            .rev()
            .find(|(_, c)| c.is_whitespace())
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(0)
    }

    fn word_right(&self, at: usize) -> usize {
        let after = &self.text[at..];
        let skipped = after.len() - after.trim_start().len();
        after[skipped..]
            .find(char::is_whitespace)
            .map(|i| at + skipped + i)
            .unwrap_or(self.text.len())
    }

    /// The word a double-click takes: the run of non-whitespace around `at`,
    /// or the run of whitespace when the click landed between two words.
    fn word_at(&self, at: usize) -> Range<usize> {
        let word = |c: &char| !c.is_whitespace();
        let prev = self.text[..at].chars().next_back();
        let next = self.text[at..].chars().next();
        let want = prev.as_ref().is_some_and(word) || next.as_ref().is_some_and(word);
        let start = self.text[..at]
            .char_indices()
            .rev()
            .find(|(_, c)| word(c) != want)
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(0);
        let end = self.text[at..]
            .char_indices()
            .find(|(_, c)| word(c) != want)
            .map(|(i, _)| at + i)
            .unwrap_or(self.text.len());
        start..end
    }
}

/// What a paste leaves of the clipboard: a single-line field drops the line
/// breaks and other control characters, the way a browser `<input>` does.
fn single_line(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}

pub struct TextInput {
    focus_handle: FocusHandle,
    content: Content,
    marked: Option<Range<usize>>,
    placeholder: SharedString,
    pad_left: Pixels,
    pad_right: Pixels,
    layout: Layout,
    dragging: bool,
}

impl EventEmitter<InputEvent> for TextInput {}

impl Focusable for TextInput {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl TextInput {
    pub fn new(placeholder: impl Into<SharedString>, cx: &mut Context<Self>) -> TextInput {
        TextInput {
            focus_handle: cx.focus_handle(),
            content: Content::default(),
            marked: None,
            placeholder: placeholder.into(),
            pad_left: px(8.),
            pad_right: px(8.),
            layout: Layout::default(),
            dragging: false,
        }
    }

    pub fn text(&self) -> &str {
        &self.content.text
    }

    pub fn set_placeholder(&mut self, placeholder: impl Into<SharedString>) {
        self.placeholder = placeholder.into();
    }

    /// Room at the left for a glyph the owner draws over the field, the
    /// 30 px Obsidian's pane filters leave for their magnifier.
    pub fn set_pad_left(&mut self, pad: Pixels) {
        self.pad_left = pad;
    }

    /// Room at the right for controls drawn over the field, like the
    /// Match case and clear buttons of the Search pane. Takes effect at the
    /// next paint; no redraw is requested, since the caller is painting.
    pub fn set_pad_right(&mut self, pad: Pixels) {
        self.pad_right = pad;
    }

    /// Replaces the text and puts the caret at its end with nothing selected;
    /// a caller that wants the old value replaced by the first keystroke —
    /// the rename prompt — follows it with [`TextInput::select_all`].
    pub fn set_text(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.content.set_text(text.into());
        self.marked = None;
        cx.emit(InputEvent::Changed);
        cx.notify();
    }

    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        self.content.select_all();
        cx.notify();
    }

    fn replace(&mut self, range: Range<usize>, insert: &str, cx: &mut Context<Self>) {
        self.content.replace(range, insert);
        self.marked = None;
        cx.emit(InputEvent::Changed);
        cx.notify();
    }

    /// Where typed or composed text lands: the range the platform named, else
    /// the composition, else the selection — so a keystroke with text selected
    /// replaces it.
    fn input_range(&self, range_utf16: Option<Range<usize>>) -> Range<usize> {
        range_utf16
            .map(|r| {
                Self::utf16_to_byte(&self.content.text, r.start)
                    ..Self::utf16_to_byte(&self.content.text, r.end)
            })
            .or_else(|| self.marked.clone())
            .unwrap_or_else(|| self.content.selection())
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        self.content.step(false, false);
        cx.notify();
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        self.content.step(true, false);
        cx.notify();
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.content.step(false, true);
        cx.notify();
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.content.step(true, true);
        cx.notify();
    }

    fn word_left(&mut self, _: &WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.content.step_word(false, false);
        cx.notify();
    }

    fn word_right(&mut self, _: &WordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.content.step_word(true, false);
        cx.notify();
    }

    fn select_word_left(&mut self, _: &SelectWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.content.step_word(false, true);
        cx.notify();
    }

    fn select_word_right(&mut self, _: &SelectWordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.content.step_word(true, true);
        cx.notify();
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.content.move_to_edge(false, false);
        cx.notify();
    }

    /// Eats a chord that must not reach the workspace, changing nothing.
    fn swallow(&mut self, _: &Swallow, _: &mut Window, _: &mut Context<Self>) {}

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.content.move_to_edge(true, false);
        cx.notify();
    }

    fn select_home(&mut self, _: &SelectHome, _: &mut Window, cx: &mut Context<Self>) {
        self.content.move_to_edge(false, true);
        cx.notify();
    }

    fn select_end(&mut self, _: &SelectEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.content.move_to_edge(true, true);
        cx.notify();
    }

    fn select_all_action(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.select_all(cx);
    }

    fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        let range = self.content.backspace_range();
        if !range.is_empty() {
            self.replace(range, "", cx);
        }
    }

    fn delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        let range = self.content.delete_range();
        if !range.is_empty() {
            self.replace(range, "", cx);
        }
    }

    fn delete_word_backward(
        &mut self,
        _: &DeleteWordBackward,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = self.content.delete_word_range();
        if !range.is_empty() {
            self.replace(range, "", cx);
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        let selection = self.content.selection();
        if !selection.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content.text[selection].to_string(),
            ));
        }
    }

    fn cut(&mut self, _: &Cut, _: &mut Window, cx: &mut Context<Self>) {
        let selection = self.content.selection();
        if !selection.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content.text[selection.clone()].to_string(),
            ));
            self.replace(selection, "", cx);
        }
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        let insert = single_line(&text);
        let range = self.content.selection();
        if !insert.is_empty() || !range.is_empty() {
            self.replace(range, &insert, cx);
        }
    }

    fn clear(&mut self, _: &Clear, _: &mut Window, cx: &mut Context<Self>) {
        self.replace(0..self.content.text.len(), "", cx);
    }

    fn submit(&mut self, _: &Submit, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(InputEvent::Submit);
    }

    fn cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(InputEvent::Cancel);
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(InputEvent::Up);
    }

    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(InputEvent::Down);
    }

    /// The byte offset the painted line puts under `x`.
    fn offset_for_x(&self, x: Pixels) -> Option<usize> {
        let line = self.layout.line.as_ref()?;
        let rel = x - self.layout.bounds.origin.x + self.layout.scroll_x;
        let mut ix = line.closest_index_for_x(rel).min(self.content.text.len());
        while !self.content.text.is_char_boundary(ix) {
            ix -= 1;
        }
        Some(ix)
    }

    fn click(
        &mut self,
        x: Pixels,
        clicks: usize,
        extend: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle, cx);
        if let Some(ix) = self.offset_for_x(x) {
            match clicks {
                2 => {
                    let word = self.content.word_at(ix);
                    self.content.anchor = word.start;
                    self.content.cursor = word.end;
                }
                n if n >= 3 => self.content.select_all(),
                _ => {
                    self.content.move_to(ix, extend);
                    self.dragging = true;
                }
            }
        }
        cx.notify();
    }

    fn drag(&mut self, x: Pixels, cx: &mut Context<Self>) {
        if !self.dragging {
            return;
        }
        if let Some(ix) = self.offset_for_x(x) {
            self.content.move_to(ix, true);
            cx.notify();
        }
    }

    fn utf16_to_byte(text: &str, utf16: usize) -> usize {
        let mut units = 0;
        for (i, c) in text.char_indices() {
            if units >= utf16 {
                return i;
            }
            units += c.len_utf16();
        }
        text.len()
    }

    fn byte_to_utf16(text: &str, byte: usize) -> usize {
        text[..byte.min(text.len())]
            .chars()
            .map(char::len_utf16)
            .sum()
    }
}

impl EntityInputHandler for TextInput {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let text = &self.content.text;
        let start = Self::utf16_to_byte(text, range_utf16.start);
        let end = Self::utf16_to_byte(text, range_utf16.end);
        adjusted_range.replace(Self::byte_to_utf16(text, start)..Self::byte_to_utf16(text, end));
        Some(text[start..end].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let selection = self.content.selection();
        let start = Self::byte_to_utf16(&self.content.text, selection.start);
        let end = Self::byte_to_utf16(&self.content.text, selection.end);
        Some(UTF16Selection {
            range: start..end,
            reversed: self.content.cursor < self.content.anchor,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked.as_ref().map(|r| {
            Self::byte_to_utf16(&self.content.text, r.start)
                ..Self::byte_to_utf16(&self.content.text, r.end)
        })
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.marked = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = self.input_range(range_utf16);
        self.replace(range, new_text, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = self.input_range(range_utf16);
        self.replace(range.clone(), new_text, cx);
        let marked = range.start..range.start + new_text.len();
        if let Some(sel) = new_selected_range {
            let at = (marked.start + Self::utf16_to_byte(new_text, sel.end))
                .min(self.content.text.len());
            self.content.move_to(at, false);
        }
        self.marked = Some(marked);
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let line = self.layout.line.as_ref()?;
        let start = Self::utf16_to_byte(&self.content.text, range_utf16.start);
        let end = Self::utf16_to_byte(&self.content.text, range_utf16.end);
        let x0 = element_bounds.origin.x - self.layout.scroll_x;
        Some(Bounds::from_corners(
            point(x0 + line.x_for_index(start), element_bounds.origin.y),
            point(
                x0 + line.x_for_index(end),
                element_bounds.origin.y + self.layout.line_height,
            ),
        ))
    }

    fn character_index_for_point(
        &mut self,
        _point: gpui::Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

struct Prepaint {
    line: ShapedLine,
    cursor_x: Pixels,
    /// The selection's two edges in the shaped line, when there is one.
    selection_x: Option<Range<Pixels>>,
    scroll_x: Pixels,
    line_height: Pixels,
    focused: bool,
}

impl Render for TextInput {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let entity = cx.entity();
        let paint_entity = entity.clone();
        div()
            .key_context("TextInput")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::word_left))
            .on_action(cx.listener(Self::word_right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_home))
            .on_action(cx.listener(Self::select_end))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::select_all_action))
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::delete_word_backward))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::clear))
            .on_action(cx.listener(Self::submit))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::swallow))
            .w_full()
            .h(px(30.))
            .pl(self.pad_left)
            .pr(self.pad_right)
            .flex()
            .items_center()
            .rounded_md()
            .bg(theme.bg_alt)
            .border_1()
            .border_color(if self.focus_handle.is_focused(_window) {
                theme.accent
            } else {
                theme.border
            })
            .text_size(px(13.))
            .text_color(theme.text)
            .child(
                canvas(
                    move |bounds, window, cx| {
                        let style = window.text_style();
                        let font_size = style.font_size.to_pixels(window.rem_size());
                        let line_height = window.line_height();
                        let (text, cursor, selection, placeholder, focused, prev_scroll) = {
                            let input = entity.read(cx);
                            (
                                input.content.text.clone(),
                                input.content.cursor,
                                input.content.selection(),
                                input.placeholder.clone(),
                                input.focus_handle.is_focused(window),
                                input.layout.scroll_x,
                            )
                        };
                        let theme = cx.global::<Theme>();
                        let (shown, color) = if text.is_empty() {
                            (placeholder.to_string(), theme.text_faint)
                        } else {
                            (text.clone(), theme.text)
                        };
                        let mut run = style.to_run(shown.len());
                        run.color = color;
                        let line = window.text_system().shape_line(
                            SharedString::from(shown),
                            font_size,
                            &[run],
                            None,
                        );
                        let cursor_x = if text.is_empty() {
                            px(0.)
                        } else {
                            line.x_for_index(cursor)
                        };
                        let selection_x = (!text.is_empty() && !selection.is_empty()).then(|| {
                            line.x_for_index(selection.start)..line.x_for_index(selection.end)
                        });
                        let width = bounds.size.width;
                        let mut scroll_x = prev_scroll;
                        if cursor_x - scroll_x > width - px(2.) {
                            scroll_x = cursor_x - width + px(2.);
                        }
                        if cursor_x - scroll_x < px(0.) {
                            scroll_x = cursor_x;
                        }
                        if line.width < width {
                            scroll_x = px(0.);
                        }
                        Prepaint {
                            line,
                            cursor_x,
                            selection_x,
                            scroll_x,
                            line_height,
                            focused,
                        }
                    },
                    move |bounds, prepaint: Prepaint, window, cx| {
                        let focus_handle = paint_entity.read(cx).focus_handle.clone();
                        window.handle_input(
                            &focus_handle,
                            ElementInputHandler::new(bounds, paint_entity.clone()),
                            cx,
                        );
                        let y = bounds.origin.y + (bounds.size.height - prepaint.line_height) / 2.;
                        let origin = point(bounds.origin.x - prepaint.scroll_x, y);
                        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
                            if let Some(selection) = &prepaint.selection_x {
                                let theme = cx.global::<Theme>();
                                let mut color = theme.accent;
                                color.a = if theme.dark { 0.35 } else { 0.25 };
                                window.paint_quad(fill(
                                    Bounds::from_corners(
                                        point(origin.x + selection.start, y),
                                        point(origin.x + selection.end, y + prepaint.line_height),
                                    ),
                                    color,
                                ));
                            }
                            let _ = prepaint.line.paint(
                                origin,
                                prepaint.line_height,
                                TextAlign::Left,
                                None,
                                window,
                                cx,
                            );
                            if prepaint.focused {
                                let x = bounds.origin.x + prepaint.cursor_x - prepaint.scroll_x;
                                let color = cx.global::<Theme>().text;
                                window.paint_quad(fill(
                                    Bounds::new(
                                        point(x, y + px(2.)),
                                        size(px(1.5), prepaint.line_height - px(4.)),
                                    ),
                                    color,
                                ));
                            }
                        });
                        let click_entity = paint_entity.clone();
                        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                            if phase == DispatchPhase::Bubble
                                && event.button == MouseButton::Left
                                && bounds.contains(&event.position)
                            {
                                click_entity.update(cx, |input, cx| {
                                    input.click(
                                        event.position.x,
                                        event.click_count,
                                        event.modifiers.shift,
                                        window,
                                        cx,
                                    )
                                });
                            }
                        });
                        let drag_entity = paint_entity.clone();
                        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _window, cx| {
                            if phase == DispatchPhase::Bubble
                                && event.pressed_button == Some(MouseButton::Left)
                            {
                                drag_entity
                                    .update(cx, |input, cx| input.drag(event.position.x, cx));
                            }
                        });
                        let up_entity = paint_entity.clone();
                        window.on_mouse_event(move |event: &MouseUpEvent, phase, _window, cx| {
                            if phase == DispatchPhase::Bubble && event.button == MouseButton::Left {
                                up_entity.update(cx, |input, _| input.dragging = false);
                            }
                        });
                        paint_entity.update(cx, |input, _| {
                            input.layout = Layout {
                                bounds,
                                line: Some(prepaint.line),
                                scroll_x: prepaint.scroll_x,
                                line_height: prepaint.line_height,
                            };
                        });
                    },
                )
                .size_full(),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn content(text: &str) -> Content {
        let mut c = Content::default();
        c.set_text(text.to_string());
        c
    }

    #[test]
    fn select_all_then_typing_replaces_the_whole_field() {
        let mut c = content("Untitled");
        assert_eq!(c.selection(), 8..8);
        c.select_all();
        assert_eq!(c.selection(), 0..8);
        let range = c.selection();
        c.replace(range, "Welcome");
        assert_eq!(c.text, "Welcome");
        assert_eq!(c.cursor, 7);
        assert!(!c.has_selection());
    }

    #[test]
    fn shift_arrows_extend_and_a_bare_arrow_collapses_to_the_edge() {
        let mut c = content("abcd");
        c.step(false, true);
        c.step(false, true);
        assert_eq!(c.selection(), 2..4);
        assert!(c.cursor < c.anchor);
        c.step(false, false);
        assert_eq!((c.cursor, c.anchor), (2, 2));
        c.select_all();
        c.step(true, false);
        assert_eq!((c.cursor, c.anchor), (4, 4));
    }

    #[test]
    fn home_and_end_extend_only_with_shift() {
        let mut c = content("abcd");
        c.move_to_edge(false, true);
        assert_eq!(c.selection(), 0..4);
        c.move_to_edge(true, false);
        assert_eq!(c.selection(), 4..4);
    }

    #[test]
    fn backspace_and_delete_take_the_selection_when_there_is_one() {
        let mut c = content("alpha beta");
        c.select_all();
        assert_eq!(c.backspace_range(), 0..10);
        assert_eq!(c.delete_range(), 0..10);
        assert_eq!(c.delete_word_range(), 0..10);
        c.move_to(5, false);
        assert_eq!(c.backspace_range(), 4..5);
        assert_eq!(c.delete_range(), 5..6);
        assert_eq!(c.delete_word_range(), 0..5);
    }

    #[test]
    fn backspace_at_the_start_of_an_empty_field_removes_nothing() {
        let c = content("");
        assert!(c.backspace_range().is_empty());
        assert!(c.delete_range().is_empty());
        assert!(c.delete_word_range().is_empty());
    }

    #[test]
    fn word_motion_steps_over_whitespace_delimited_words() {
        let mut c = content("alpha beta gamma");
        c.step_word(false, false);
        assert_eq!(c.cursor, 11);
        c.step_word(false, true);
        assert_eq!(c.selection(), 6..11);
        c.move_to(0, false);
        c.step_word(true, false);
        assert_eq!(c.cursor, 5);
        c.step_word(true, false);
        assert_eq!(c.cursor, 10);
    }

    #[test]
    fn double_click_takes_the_word_around_the_offset() {
        let c = content("alpha beta gamma");
        assert_eq!(c.word_at(8), 6..10);
        assert_eq!(c.word_at(6), 6..10);
        assert_eq!(c.word_at(10), 6..10);
        assert_eq!(c.word_at(16), 11..16);
        assert_eq!(content("").word_at(0), 0..0);
    }

    #[test]
    fn multibyte_text_keeps_the_caret_on_char_boundaries() {
        let mut c = content("naïve café");
        c.step(false, false);
        assert_eq!(&c.text[c.cursor..], "é");
        c.move_to_edge(false, false);
        c.step_word(true, false);
        assert_eq!(&c.text[..c.cursor], "naïve");
        c.step(false, false);
        assert_eq!(&c.text[..c.cursor], "naïv");
    }

    #[test]
    fn a_paste_into_a_single_line_field_drops_line_breaks() {
        assert_eq!(single_line("a\nb\r\nc"), "abc");
        assert_eq!(single_line("plain"), "plain");
        assert_eq!(single_line("tab\there"), "tabhere");
    }
}
