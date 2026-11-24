//! Spellchecking, without the UI.
//!
//! Obsidian's editor is spellchecked by Chromium's Hunspell-based checker;
//! Silica checks with [spellbook](https://crates.io/crates/spellbook), a pure
//! Rust reader of the same Hunspell `.aff`/`.dic` dictionaries. This crate
//! owns:
//!
//! - [`dict`]: the bundled `en_US` (SCOWL's, the source of Chromium's own
//!   `en-US`) and dictionaries read from disk;
//! - [`custom`]: the "Add to dictionary" words, in Chromium's
//!   `Custom Dictionary.txt` format;
//! - [`text`]: which bytes of a note are checked (from the Markdown tree)
//!   and how they split into words;
//! - [`Speller`]: all of it together, and [`Speller::misspellings`], what
//!   the editor squiggles.

pub mod custom;
pub mod dict;
pub mod text;

use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

pub use custom::CustomDictionary;
pub use dict::Dictionary;

/// The most corrections the editor menu offers: Chromium's three.
pub const MAX_SUGGESTIONS: usize = 3;

/// The dictionaries of the chosen languages and the custom words. Cheap to
/// clone: the dictionaries are shared. Remembers each word's verdict, so a
/// note rechecked on every keystroke looks up each distinct word once.
#[derive(Clone)]
pub struct Speller {
    dictionaries: Arc<RwLock<Vec<Arc<Dictionary>>>>,
    custom: Arc<Mutex<CustomDictionary>>,
    /// Where the custom words are kept, when the app knows its
    /// configuration folder.
    custom_path: Arc<Mutex<Option<PathBuf>>>,
    /// The language codes the dictionaries were loaded for, so a settings
    /// write that does not change them loads nothing again.
    configured: Arc<Mutex<Vec<String>>>,
    verdicts: Arc<Mutex<HashMap<String, bool>>>,
}

impl std::fmt::Debug for Speller {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Speller")
            .field("languages", &self.languages())
            .field("custom", &self.custom_len())
            .finish()
    }
}

impl Speller {
    pub fn new(dictionaries: Vec<Arc<Dictionary>>, custom: CustomDictionary) -> Speller {
        Speller {
            dictionaries: Arc::new(RwLock::new(dictionaries)),
            custom: Arc::new(Mutex::new(custom)),
            custom_path: Arc::new(Mutex::new(None)),
            configured: Arc::new(Mutex::new(Vec::new())),
            verdicts: Arc::default(),
        }
    }

    /// The bundled `en_US` dictionary and an empty custom list.
    pub fn bundled() -> Speller {
        Speller::new(vec![Arc::new(Dictionary::bundled())], CustomDictionary::default())
    }

    /// The one speller the app shares: the bundled dictionary is parsed
    /// once for every editor, not once per tab.
    pub fn shared() -> Arc<Speller> {
        static SHARED: OnceLock<Arc<Speller>> = OnceLock::new();
        SHARED.get_or_init(|| Arc::new(Speller::bundled())).clone()
    }

    /// The languages checked, in order.
    pub fn languages(&self) -> Vec<String> {
        self.dictionaries
            .read()
            .map(|d| d.iter().map(|d| d.language.clone()).collect())
            .unwrap_or_default()
    }

    /// Checks the words with the dictionaries of `languages`, each read from
    /// the first folder that has it and the bundled `en-US` when none does.
    /// The languages with no dictionary anywhere are returned, and the
    /// custom words are kept.
    pub fn set_languages(&self, languages: &[String], config_dir: Option<&Path>) -> Vec<String> {
        let (dicts, missing) = load_languages(languages, config_dir);
        if let Ok(mut c) = self.configured.lock() {
            *c = languages.to_vec();
        }
        if let Ok(mut d) = self.dictionaries.write() {
            *d = dicts;
        }
        // * The verdicts belong to the old dictionaries.
        if let Ok(mut v) = self.verdicts.lock() {
            v.clear();
        }
        missing
    }

    /// Where the custom words are kept. Words added before the path is set
    /// stay in memory and are written when the path arrives.
    pub fn set_custom_path(&self, path: PathBuf) {
        if let Ok(mut p) = self.custom_path.lock() {
            *p = Some(path);
        }
    }

    /// The language codes the dictionaries were loaded for.
    pub fn configured_languages(&self) -> Vec<String> {
        self.configured.lock().map(|c| c.clone()).unwrap_or_default()
    }

    /// The custom words, in the order the file writes them.
    pub fn custom_words(&self) -> Vec<String> {
        self.custom
            .lock()
            .map(|c| c.words().map(str::to_string).collect())
            .unwrap_or_default()
    }

    /// How many words the custom dictionary holds.
    pub fn custom_len(&self) -> usize {
        self.custom.lock().map(|c| c.len()).unwrap_or(0)
    }

    /// Loads the custom words from `path` and keeps it for later writes.
    /// The number of words read is returned.
    pub fn load_custom(&self, path: &Path) -> usize {
        if let Ok(mut c) = self.custom.lock() {
            *c = CustomDictionary::load(path);
        }
        if let Ok(mut p) = self.custom_path.lock() {
            *p = Some(path.to_path_buf());
        }
        self.custom_len()
    }

    /// Adds a word to the custom dictionary and writes the file, when one
    /// is known. False when the word was there already or is not valid.
    pub fn add_to_dictionary(&self, word: &str) -> bool {
        let added = self
            .custom
            .lock()
            .map(|mut c| c.add(word))
            .unwrap_or(false);
        if !added {
            return false;
        }
        // * The new word is a word now, whatever the dictionaries say.
        if let Ok(mut v) = self.verdicts.lock() {
            v.insert(word.to_string(), true);
        }
        self.save_custom();
        true
    }

    /// Writes the custom dictionary to its file, when one is known.
    pub fn save_custom(&self) -> bool {
        let path = self.custom_path.lock().ok().and_then(|p| p.clone());
        let Some(path) = path else {
            return false;
        };
        let words = self.custom.lock().map(|c| c.clone()).unwrap_or_default();
        words.save(&path).is_ok()
    }

    /// Whether one word is spelled right: a custom word, a word of any
    /// chosen language, or a joined word whose every part is one. With no
    /// dictionary at all nothing is wrong.
    pub fn check(&self, word: &str) -> bool {
        match self.verdicts.lock() {
            Ok(mut m) => self.check_with(&mut m, word),
            Err(_) => self.check_uncached(word),
        }
    }

    /// [`Speller::check`] with the verdicts already locked, so a whole
    /// note takes the lock once.
    fn check_with(&self, verdicts: &mut HashMap<String, bool>, word: &str) -> bool {
        if self.dictionaries.read().map(|d| d.is_empty()).unwrap_or(true) || !text::is_checked(word)
        {
            return true;
        }
        if let Some(&v) = verdicts.get(word) {
            return v;
        }
        let v = self.check_uncached(word);
        if verdicts.len() > 200_000 {
            verdicts.clear();
        }
        verdicts.insert(word.to_string(), v);
        v
    }

    fn check_one(&self, word: &str) -> bool {
        if !text::is_checked(word) {
            return true;
        }
        if self.custom.lock().map(|c| c.contains(word)).unwrap_or(false) {
            return true;
        }
        self.dictionaries
            .read()
            .map(|d| d.iter().any(|d| d.check(word)))
            .unwrap_or(false)
    }

    fn check_uncached(&self, word: &str) -> bool {
        if self.check_one(word) {
            return true;
        }
        let parts = text::parts(word);
        parts.len() > 1 && parts.iter().all(|(_, p)| self.check_one(p))
    }

    /// Corrections for a misspelled word, at most [`MAX_SUGGESTIONS`], the
    /// chosen languages' interleaved in order, without repeats.
    pub fn suggest(&self, word: &str) -> Vec<String> {
        let Ok(dictionaries) = self.dictionaries.read() else {
            return Vec::new();
        };
        let lists: Vec<Vec<String>> = dictionaries.iter().map(|d| d.suggest(word)).collect();
        let mut out: Vec<String> = Vec::new();
        let longest = lists.iter().map(Vec::len).max().unwrap_or(0);
        for i in 0..longest {
            for l in &lists {
                if let Some(s) = l.get(i) {
                    if !out.contains(s) {
                        out.push(s.clone());
                    }
                }
            }
        }
        out.truncate(MAX_SUGGESTIONS);
        out
    }

    /// The misspelled words of a parsed note, as byte ranges of its source,
    /// in order. `typing` is the caret while the word before it is being
    /// typed: a word that ends there is left alone until a space or a move
    /// ends it, as Chromium leaves it (observed 2026-11-15).
    pub fn misspellings(
        &self,
        doc: &silica_markdown::Document<'_>,
        typing: Option<usize>,
    ) -> Vec<Range<usize>> {
        let src = doc.source;
        let mut out = Vec::new();
        if self.dictionaries.read().map(|d| d.is_empty()).unwrap_or(true) {
            return out;
        }
        let Ok(mut verdicts) = self.verdicts.lock() else {
            return out;
        };
        for range in text::checked_ranges(doc) {
            let base = range.start;
            text::for_each_word(&src[range], |at, word| {
                let start = base + at;
                let end = start + word.len();
                if typing != Some(end) && !self.check_with(&mut verdicts, word) {
                    out.push(start..end);
                }
            });
        }
        out
    }

    /// The misspelled word at byte `offset` of `src`, if any: its range in
    /// `ranges` (as [`Speller::misspellings`] returned them).
    pub fn word_at(ranges: &[Range<usize>], offset: usize) -> Option<Range<usize>> {
        let i = ranges.partition_point(|r| r.end < offset);
        ranges
            .get(i)
            .filter(|r| r.start <= offset && offset <= r.end)
            .cloned()
    }
}

/// Loads the dictionaries for `languages` (Obsidian's codes): each from the
/// first folder that has it, the bundled `en-US` when none does. A language
/// with no dictionary anywhere is skipped and named in the second list.
pub fn load_languages(
    languages: &[String],
    config_dir: Option<&Path>,
) -> (Vec<Arc<Dictionary>>, Vec<String>) {
    let found = discover::find(&discover::search_dirs(config_dir));
    let mut dicts: Vec<Arc<Dictionary>> = Vec::new();
    let mut missing = Vec::new();
    for lang in languages {
        let name = lang.replace('-', "_");
        let pair = found
            .iter()
            .find(|(code, _)| code.eq_ignore_ascii_case(&name) || code.eq_ignore_ascii_case(lang));
        match pair {
            Some((_, (aff, dic))) => match Dictionary::load(lang, aff, dic) {
                Ok(d) => dicts.push(Arc::new(d)),
                Err(_) => missing.push(lang.clone()),
            },
            None => missing.push(lang.clone()),
        }
    }
    if dicts.is_empty() {
        dicts.push(Arc::new(Dictionary::bundled()));
    }
    (dicts, missing)
}

/// Where the dictionaries on disk are looked for.
pub mod discover {
    use std::path::{Path, PathBuf};

    /// The folders a dictionary is looked for in, in order: the desktop's
    /// Hunspell folders and the app's own configuration folder.
    pub fn search_dirs(config_dir: Option<&Path>) -> Vec<PathBuf> {
        let mut dirs = Vec::new();
        if let Some(home) = std::env::var_os("HOME") {
            let home = PathBuf::from(home);
            dirs.push(home.join(".local/share/hunspell"));
            dirs.push(home.join(".hunspell"));
            dirs.push(PathBuf::from("/usr/share/hunspell"));
            dirs.push(PathBuf::from("/usr/share/myspell"));
        }
        if let Some(dir) = config_dir {
            dirs.push(dir.join("dictionaries"));
        }
        dirs
    }

    /// Every `.aff`/`.dic` pair in `dirs`, as `(language code, (aff, dic))`.
    pub fn find(dirs: &[PathBuf]) -> Vec<(String, (PathBuf, PathBuf))> {
        let mut out = Vec::new();
        for dir in dirs {
            let Ok(entries) = std::fs::read_dir(dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("aff") {
                    continue;
                }
                let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                    continue;
                };
                let dic = path.with_extension("dic");
                if dic.is_file() {
                    out.push((stem.to_string(), (path, dic)));
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_dictionary_knows_real_words() {
        let s = Speller::bundled();
        assert!(s.check("world"));
        assert!(s.check("Obsidian"));
        assert!(s.check("don't"));
        assert!(!s.check("wrold"));
        assert!(!s.check("teh"));
    }

    #[test]
    fn corrections_are_offered() {
        let s = Speller::bundled();
        let out = s.suggest("wrold");
        assert!(out.contains(&"world".to_string()), "{out:?}");
        assert!(out.len() <= MAX_SUGGESTIONS);
    }

    #[test]
    fn a_custom_word_is_accepted_and_kept() {
        let s = Speller::bundled();
        assert!(!s.check("qzqxqzxvv"));
        assert!(s.add_to_dictionary("qzqxqzxvv"));
        assert!(s.check("qzqxqzxvv"));
        assert_eq!(s.custom_words(), vec!["qzqxqzxvv".to_string()]);
    }

    #[test]
    fn a_joined_word_is_accepted_when_every_part_is() {
        let s = Speller::bundled();
        assert!(s.check("don't"));
        assert!(s.check("e.g"));
    }

    #[test]
    fn the_misspelled_words_of_a_note_are_found() {
        let s = Speller::bundled();
        let src = "A wrold of tehh words.\n";
        let doc = silica_markdown::parse(src);
        let ranges = s.misspellings(&doc, None);
        let words: Vec<&str> = ranges.iter().map(|r| &src[r.clone()]).collect();
        assert_eq!(words, vec!["wrold", "tehh"]);
    }

    #[test]
    fn the_word_being_typed_is_not_flagged() {
        let s = Speller::bundled();
        let src = "A wrold";
        let doc = silica_markdown::parse(src);
        let ranges = s.misspellings(&doc, Some(src.len()));
        assert!(ranges.is_empty(), "{ranges:?}");
    }

    #[test]
    fn code_urls_and_tags_are_not_checked() {
        let s = Speller::bundled();
        let src = "A `wrold` and https://example.com/wrold and #wrold and [[wrold]].\n";
        let doc = silica_markdown::parse(src);
        let ranges = s.misspellings(&doc, None);
        // * Only the inside of the wikilink is checked, the way Obsidian's
        //   editor checks it.
        let words: Vec<&str> = ranges.iter().map(|r| &src[r.clone()]).collect();
        assert_eq!(words, vec!["wrold"]);
    }

    #[test]
    fn the_word_at_an_offset_is_found() {
        let ranges = vec![3..8, 12..17];
        assert_eq!(Speller::word_at(&ranges, 4), Some(3..8));
        assert_eq!(Speller::word_at(&ranges, 13), Some(12..17));
        assert_eq!(Speller::word_at(&ranges, 9), None);
    }
}
