//! The custom dictionary: the words "Add to dictionary" keeps.
//!
//! Obsidian keeps them where Chromium does, in `Custom Dictionary.txt` in
//! its data folder (`~/.config/obsidian` on Linux), outside every vault, so
//! the list is per machine, not per vault. The file is one word per line
//! and a last line `checksum_v1 = <md5>`, the MD5 of everything before it,
//! with no newline after it (observed 2026-11-15: one added word wrote
//! `qzxqzxqzvv\nchecksum_v1 = 0a4d1c7d3f4e9a4f57380a86619fae97`).
//! Silica keeps its own copy of the same file under its configuration
//! folder, in the same bytes, so the file can be copied across either way.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The file's name, Chromium's.
pub const FILE_NAME: &str = "Custom Dictionary.txt";

const CHECKSUM_PREFIX: &str = "checksum_v1 = ";

/// The longest word Chromium accepts into the file, in bytes.
pub const MAX_WORD_BYTES: usize = 99;

/// The custom dictionary's words, sorted the way Chromium writes them
/// (byte order).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CustomDictionary {
    words: BTreeSet<String>,
}

/// Whether `word` may go in the file: not empty, no whitespace, at most
/// [`MAX_WORD_BYTES`].
pub fn is_valid_word(word: &str) -> bool {
    !word.is_empty() && word.len() <= MAX_WORD_BYTES && !word.chars().any(char::is_whitespace)
}

impl CustomDictionary {
    /// The path under a configuration folder.
    pub fn path_in(config_dir: &Path) -> PathBuf {
        config_dir.join(FILE_NAME)
    }

    /// Reads the file's words. A missing file is an empty dictionary; the
    /// checksum line is not a word, and a checksum that does not match is
    /// ignored rather than refused (a person may have edited the file).
    pub fn parse(text: &str) -> CustomDictionary {
        let words = text
            .lines()
            .map(|l| l.trim_end_matches('\r').trim())
            .filter(|l| !l.starts_with(CHECKSUM_PREFIX))
            .filter(|l| is_valid_word(l))
            .map(str::to_string)
            .collect();
        CustomDictionary { words }
    }

    pub fn load(path: &Path) -> CustomDictionary {
        std::fs::read_to_string(path)
            .map(|t| CustomDictionary::parse(&t))
            .unwrap_or_default()
    }

    /// The file's bytes: each word and a newline, then the checksum line.
    pub fn to_text(&self) -> String {
        let mut body = String::new();
        for w in &self.words {
            body.push_str(w);
            body.push('\n');
        }
        let sum = md5::compute(body.as_bytes());
        body.push_str(CHECKSUM_PREFIX);
        body.push_str(&format!("{sum:x}"));
        body
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, self.to_text())
    }

    /// Adds a word; false when it was there already or is not a valid word.
    pub fn add(&mut self, word: &str) -> bool {
        is_valid_word(word) && self.words.insert(word.to_string())
    }

    pub fn remove(&mut self, word: &str) -> bool {
        self.words.remove(word)
    }

    pub fn words(&self) -> impl Iterator<Item = &str> {
        self.words.iter().map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.words.len()
    }

    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    /// Whether the dictionary accepts `word`. As with a word added to a
    /// Hunspell dictionary, a lower-case entry also accepts its Title-case
    /// and ALL-CAPS forms, and a Title-case one its ALL-CAPS form; the
    /// other way round does not hold.
    pub fn contains(&self, word: &str) -> bool {
        if self.words.contains(word) {
            return true;
        }
        let lower = word.to_lowercase();
        if lower != word {
            if word == word.to_uppercase() {
                // ALL-CAPS: any entry that differs only in case below it.
                if self.words.contains(&lower) || self.words.contains(&title_case(&lower)) {
                    return true;
                }
            } else if word == title_case(&lower) && self.words.contains(&lower) {
                return true;
            }
        }
        false
    }
}

fn title_case(lower: &str) -> String {
    let mut c = lower.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_word_writes_the_bytes_the_editor_writes() {
        let mut d = CustomDictionary::default();
        assert!(d.add("qzxqzxqzvv"));
        assert!(!d.add("qzxqzxqzvv"));
        assert_eq!(
            d.to_text(),
            "qzxqzxqzvv\nchecksum_v1 = 0a4d1c7d3f4e9a4f57380a86619fae97"
        );
    }

    #[test]
    fn the_file_round_trips_and_sorts_by_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = CustomDictionary::path_in(dir.path());
        let mut d = CustomDictionary::default();
        for w in ["zeta", "Alpha", "beta", "émile"] {
            d.add(w);
        }
        d.save(&path).unwrap();
        let back = CustomDictionary::load(&path);
        assert_eq!(back, d);
        assert_eq!(
            back.words().collect::<Vec<_>>(),
            ["Alpha", "beta", "zeta", "émile"]
        );
        assert_eq!(CustomDictionary::load(&dir.path().join("none")).len(), 0);
    }

    #[test]
    fn invalid_words_are_refused() {
        let mut d = CustomDictionary::default();
        assert!(!d.add(""));
        assert!(!d.add("two words"));
        assert!(!d.add(&"x".repeat(100)));
        assert!(d.add(&"x".repeat(99)));
    }

    #[test]
    fn case_follows_hunspell() {
        let mut d = CustomDictionary::default();
        d.add("silica");
        d.add("Anya");
        assert!(d.contains("silica"));
        assert!(d.contains("Silica"));
        assert!(d.contains("SILICA"));
        assert!(!d.contains("sIlica"));
        assert!(d.contains("Anya"));
        assert!(d.contains("ANYA"));
        assert!(!d.contains("anya"));
    }
}
