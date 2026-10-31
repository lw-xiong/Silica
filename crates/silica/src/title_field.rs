//! The text model behind the view header's inline rename field.
//!
//! Obsidian's `.view-header-title` is a `contentEditable` in its resting
//! state: a single click focuses it and puts the caret at the clicked
//! offset, typing edits the title in place while the file keeps its old
//! name, Enter commits the rename and Escape abandons it (see
//! [[Specification#The view-header title is an inline rename field]]).
//!
//! The field is painted as the header's own plain text, caret and all,
//! which is why it is not a [`crate::text_input::TextInput`]: that widget
//! draws a 30 px bordered box of its own and keeps its caret private, so a
//! click could not land the caret where the pointer was. Only the text and
//! the caret live here, with no GPUI in sight, so the editing rules can be
//! tested on their own -- the same split [`crate::text_input`] makes.

use std::ops::Range;

/// What a paste leaves of the clipboard, and what an IME commit of a
/// newline leaves: a title is one line, so the control characters go.
fn single_line(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}

/// The byte offset a UTF-16 offset names, which is how the platform's
/// input handler counts.
pub fn utf16_to_byte(text: &str, utf16: usize) -> usize {
    let mut units = 0;
    for (i, c) in text.char_indices() {
        if units >= utf16 {
            return i;
        }
        units += c.len_utf16();
    }
    text.len()
}

/// The UTF-16 offset a byte offset names, the other way.
pub fn byte_to_utf16(text: &str, byte: usize) -> usize {
    text[..byte.min(text.len())]
        .chars()
        .map(char::len_utf16)
        .sum()
}

/// The title being typed into: the text on screen and the selection in it.
/// The caret is the selection's head, so a field with nothing selected has
/// `cursor == anchor`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TitleField {
    text: String,
    cursor: usize,
    anchor: usize,
}

impl TitleField {
    /// A field over `text` with the caret at its end, which is where a
    /// field opened by anything but a click starts.
    pub fn new(text: impl Into<String>) -> TitleField {
        let text = text.into();
        let at = text.len();
        TitleField {
            text,
            cursor: at,
            anchor: at,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn selection(&self) -> Range<usize> {
        self.cursor.min(self.anchor)..self.cursor.max(self.anchor)
    }

    pub fn has_selection(&self) -> bool {
        self.cursor != self.anchor
    }

    /// Whether the caret is before its anchor, which is what the platform
    /// asks when it reads the selection back.
    pub fn reversed(&self) -> bool {
        self.cursor < self.anchor
    }

    /// What a click leaves: one click puts the caret where the pointer was,
    /// a double click takes the word around it, a third takes the lot. The
    /// offsets come from the shaped line, so they are already on character
    /// boundaries.
    pub fn click(&mut self, at: usize, clicks: usize, extend: bool) {
        match clicks {
            2 => {
                let word = self.word_at(at);
                self.anchor = word.start;
                self.cursor = word.end;
            }
            n if n >= 3 => self.select_all(),
            _ => self.move_to(at, extend),
        }
    }

    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.cursor = self.text.len();
    }

    pub fn move_to(&mut self, to: usize, extend: bool) {
        self.cursor = to.min(self.text.len());
        while !self.text.is_char_boundary(self.cursor) {
            self.cursor -= 1;
        }
        if !extend {
            self.anchor = self.cursor;
        }
    }

    /// Typed, composed or pasted text, which replaces the selection.
    pub fn insert(&mut self, text: &str) {
        let range = self.selection();
        if text.is_empty() && range.is_empty() {
            return;
        }
        self.replace(range, text);
    }

    /// Puts `insert` in the place of `range` and leaves the caret behind
    /// it. The text a file name cannot hold never gets in: this is the
    /// field's one mutation, so filtering here covers typing, composing
    /// and pasting alike. Says how many bytes actually landed, which is
    /// what an input method is told its composition covers.
    pub fn replace(&mut self, range: Range<usize>, insert: &str) -> usize {
        let insert = single_line(insert);
        self.text.replace_range(range.clone(), &insert);
        self.cursor = range.start + insert.len();
        self.anchor = self.cursor;
        insert.len()
    }

    /// Backspace: the selection, or the character before the caret.
    pub fn backspace(&mut self) {
        let range = if self.has_selection() {
            self.selection()
        } else {
            self.prev_boundary(self.cursor)..self.cursor
        };
        self.replace(range, "");
    }

    pub fn delete(&mut self) {
        let range = if self.has_selection() {
            self.selection()
        } else {
            self.cursor..self.next_boundary(self.cursor)
        };
        self.replace(range, "");
    }

    pub fn delete_word_back(&mut self) {
        let range = if self.has_selection() {
            self.selection()
        } else {
            self.word_left(self.cursor)..self.cursor
        };
        self.replace(range, "");
    }

    /// A bare Left or Right collapses a selection onto the edge it points
    /// at rather than stepping from the head, as a native input does.
    pub fn step(&mut self, forward: bool, extend: bool) {
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

    pub fn step_word(&mut self, forward: bool, extend: bool) {
        let to = if forward {
            self.word_right(self.cursor)
        } else {
            self.word_left(self.cursor)
        };
        self.move_to(to, extend);
    }

    pub fn move_to_edge(&mut self, end: bool, extend: bool) {
        let to = if end { self.text.len() } else { 0 };
        self.move_to(to, extend);
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

    /// The start of the word before `at`, words being whitespace-delimited
    /// here as they are in the crate's other field.
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

    /// The word a double click takes: the run of non-whitespace around
    /// `at`, or the run of whitespace when the pointer landed between two
    /// words.
    pub fn word_at(&self, at: usize) -> Range<usize> {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The gesture the probe recorded: `Props` clicked at offset 3 and
    /// typed `X` reads `ProXps`, and nothing but the field has changed --
    /// the file is still `Props.md` until Enter.
    #[test]
    fn a_click_puts_the_caret_at_the_offset_and_typing_edits_in_place() {
        let mut field = TitleField::new("Props");
        field.click(3, 1, false);
        assert_eq!(field.cursor(), 3);
        assert!(!field.has_selection());
        field.insert("X");
        assert_eq!(field.text(), "ProXps");
        assert_eq!(field.cursor(), 4);
    }

    /// A double click stays in the word it landed in: no modal, no menu,
    /// just the word under the pointer.
    #[test]
    fn a_double_click_takes_the_word_it_landed_in() {
        let mut field = TitleField::new("Daily note two");
        field.click(8, 2, false);
        assert_eq!(field.selection(), 6..10, "the word around offset 8");
        assert_eq!(&field.text()[field.selection()], "note");
        field.click(8, 3, false);
        assert_eq!(field.selection(), 0..14, "a third click takes the lot");
    }

    #[test]
    fn typing_over_a_selection_replaces_it() {
        let mut field = TitleField::new("Untitled");
        field.select_all();
        field.insert("Notes");
        assert_eq!(field.text(), "Notes");
        assert_eq!(field.cursor(), 5);
    }

    #[test]
    fn backspace_and_delete_take_a_character_or_the_selection() {
        let mut field = TitleField::new("Alpha");
        field.backspace();
        assert_eq!(field.text(), "Alph");
        field.move_to_edge(false, false);
        field.delete();
        assert_eq!(field.text(), "lph");
        field.click(0, 1, false);
        field.step(true, true);
        assert_eq!(field.selection(), 0..1);
        field.backspace();
        assert_eq!(field.text(), "ph");
        field.delete_word_back();
        assert_eq!(field.text(), "ph", "nothing before the caret to take");
        field.move_to_edge(true, false);
        field.delete_word_back();
        assert_eq!(field.text(), "");
    }

    /// A title is one line: a pasted or composed newline never reaches it,
    /// since a file name cannot hold one.
    #[test]
    fn a_pasted_newline_never_enters_the_field() {
        let mut field = TitleField::new("One");
        field.insert(" two\nthree\r");
        assert_eq!(field.text(), "One twothree");
    }

    /// Multi-byte text moves a character at a time, never into the middle
    /// of one.
    #[test]
    fn the_caret_steps_whole_characters() {
        let mut field = TitleField::new("Ärger✓");
        field.move_to_edge(false, false);
        field.step(true, false);
        assert_eq!(field.cursor(), 2, "Ä is two bytes");
        field.move_to_edge(true, false);
        field.step(false, false);
        assert_eq!(field.cursor(), 6, "✓ is three");
        field.step_word(false, false);
        assert_eq!(field.cursor(), 0);
    }
}
