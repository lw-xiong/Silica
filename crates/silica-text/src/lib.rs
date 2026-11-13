//! The editor's text model, free of gpui: one `String` buffer with a line
//! index, the selections over it, the undo history, and the edits the
//! editing commands plan.
//!
//! The source editor is a GPUI view, but the bytes it writes back to the
//! vault are decided here, where they can be tested without a window. That
//! is what the Writer gate needs: the vault round-trips byte-identical, so
//! every path from a keystroke to a file's bytes has to be drivable from a
//! test (`tests/writer_roundtrip.rs`).
//!
//! [`TextBuffer`] holds `\n` line endings; a file's own ending and byte
//! order mark are remembered at load and restored by
//! [`TextBuffer::file_contents`], so a save changes nothing but the edited
//! bytes.

pub mod autopair;
mod buffer;
pub mod find;
pub mod format;
pub mod lists;
mod plan;
mod spell;
pub mod table;
pub mod wrap;

pub use buffer::{
    char_class, next_grapheme_boundary, prev_grapheme_boundary, Cursor, Cursors, EditKind,
    FileText, Sel, TextBuffer,
};
pub use format::BlockInsert;
pub use format::{LinePrefix, LinkSyntax};
pub use plan::{indent_plan, newline_plan, shifted};
pub use spell::SpellChecker;

/// Word/character counts for a note, as the status bar and the word-count
/// command report them. A word is a maximal run of non-whitespace; markdown
/// markup characters count as part of their word, matching how Obsidian's
/// own counter behaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WordStats {
    pub words: usize,
    pub characters: usize,
    pub characters_no_spaces: usize,
}

pub fn word_stats(text: &str) -> WordStats {
    let words = text.split_whitespace().count();
    let characters = text.chars().count();
    let characters_no_spaces = text.chars().filter(|c| !c.is_whitespace()).count();
    WordStats {
        words,
        characters,
        characters_no_spaces,
    }
}

#[cfg(test)]
mod stats_tests {
    use super::word_stats;

    #[test]
    fn counts_words_and_characters() {
        let stats = word_stats("one two  three\n\nfour");
        assert_eq!(stats.words, 4);
        assert_eq!(stats.characters, 20);
        assert_eq!(stats.characters_no_spaces, 15);
    }

    #[test]
    fn empty_note_is_zero() {
        let stats = word_stats("");
        assert_eq!(stats.words, 0);
        assert_eq!(stats.characters, 0);
        assert_eq!(stats.characters_no_spaces, 0);
    }

    #[test]
    fn markup_counts_with_its_word() {
        // "**bold**" is one word, not three.
        assert_eq!(word_stats("**bold** text").words, 2);
    }
}
