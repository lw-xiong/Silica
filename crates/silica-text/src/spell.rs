//! Spell checking: a small embedded dictionary and a checker that tells
//! whether a word is known. The dictionary is the most common English words,
//! enough that ordinary writing rarely flags a real word as misspelled.
//! Unknown words are highlighted with a squiggly underline in the editor.

use std::collections::HashSet;

/// A small embedded dictionary of common English words.
///
/// Sorted so `HashSet` lookups are O(1); the list is small enough that a
/// vault with a hundred open notes still checks every visible word in well
/// under a millisecond. Words are lowercase; the checker lowercases before
/// looking up, so capitalization never causes a false misspelling.
const DICTIONARY: &str = include_str!("dictionary.txt");

/// Checks words against the embedded dictionary.
#[derive(Debug, Clone, Default)]
pub struct SpellChecker {
    words: HashSet<&'static str>,
}

impl SpellChecker {
    /// Builds a checker from the embedded dictionary.
    pub fn new() -> Self {
        let words = DICTIONARY.lines().collect();
        Self { words }
    }

    /// Returns `true` when `word` is in the dictionary. Case-insensitive;
    /// leading and trailing punctuation is stripped before lookup.
    pub fn is_known(&self, word: &str) -> bool {
        let cleaned = word.trim_matches(|c: char| !c.is_alphanumeric());
        if cleaned.is_empty() {
            return true;
        }
        self.words.contains(cleaned.to_lowercase().as_str())
    }

    /// Returns the misspelled words in `text` as `(byte_start, byte_end)`
    /// ranges over `text`'s bytes.
    pub fn misspelled(&self, text: &str) -> Vec<(usize, usize)> {
        let mut misspelled = Vec::new();
        let bytes = text.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            // Skip non-word characters.
            if !bytes[i].is_ascii_alphabetic() && bytes[i] != b'\'' {
                i += 1;
                continue;
            }
            // Find the end of this word run.
            let mut j = i;
            while j < bytes.len()
                && (bytes[j].is_ascii_alphabetic() || bytes[j] == b'\'')
            {
                j += 1;
            }
            let word = &text[i..j];
            if !self.is_known(word) {
                misspelled.push((i, j));
            }
            i = j;
        }
        misspelled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_words_are_not_misspelled() {
        let checker = SpellChecker::new();
        assert!(checker.is_known("hello"));
        assert!(checker.is_known("Hello"));
        assert!(checker.is_known("the"));
        assert!(checker.is_known("silica"));
        // Silica is in the dictionary.
        assert!(checker.misspelled("hello world").is_empty());
    }

    #[test]
    fn unknown_words_are_flagged() {
        let checker = SpellChecker::new();
        let misspelled = checker.misspelled("teh quick broen fox");
        assert!(!misspelled.is_empty());
        // "teh" starts at 0, ends at 3.
        assert_eq!(misspelled[0], (0, 3));
    }

    #[test]
    fn punctuation_is_stripped() {
        let checker = SpellChecker::new();
        // "hello," should be known because the comma is stripped.
        assert!(checker.is_known("hello,"));
        assert!(checker.is_known("(hello)"));
        // "teh," should still be misspelled.
        let misspelled = checker.misspelled("teh, world");
        assert_eq!(misspelled, vec![(0, 3)]);
    }

    #[test]
    fn empty_text_has_no_misspellings() {
        let checker = SpellChecker::new();
        assert!(checker.misspelled("").is_empty());
        assert!(checker.misspelled("   ,..! ").is_empty());
    }
}
