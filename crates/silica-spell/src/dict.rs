//! Hunspell dictionaries: the bundled `en_US`, and `.aff`/`.dic` pairs read
//! from disk.

use std::fmt;
use std::path::Path;

/// The dictionary Silica ships: SCOWL's `en_US` at size 60, the Hunspell
/// dictionary Chromium's own `en-US` is built from. See
/// `dictionaries/en_US/LICENSE` (SCOWL's permission notice, Ispell's BSD
/// licence and WordNet's).
const BUNDLED_AFF: &str = include_str!("../dictionaries/en_US/en_US.aff");
const BUNDLED_DIC: &str = include_str!("../dictionaries/en_US/en_US.dic");

/// The language code of the bundled dictionary, as Obsidian names it.
pub const BUNDLED_LANGUAGE: &str = "en-US";

/// One loaded Hunspell dictionary.
pub struct Dictionary {
    /// The language code in Obsidian's spelling: `en-US`, `de-DE`, `fr`.
    pub language: String,
    inner: spellbook::Dictionary,
}

impl fmt::Debug for Dictionary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Dictionary")
            .field("language", &self.language)
            .finish_non_exhaustive()
    }
}

/// Why a dictionary did not load.
#[derive(Debug)]
pub enum LoadError {
    Io(std::io::Error),
    /// The `.aff` names an encoding other than UTF-8 or ISO-8859-1.
    Encoding(String),
    Parse(String),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::Io(e) => write!(f, "{e}"),
            LoadError::Encoding(e) => write!(f, "unsupported dictionary encoding {e}"),
            LoadError::Parse(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for LoadError {}

impl Dictionary {
    /// The bundled `en_US`.
    pub fn bundled() -> Dictionary {
        let inner = spellbook::Dictionary::new(BUNDLED_AFF, BUNDLED_DIC)
            .expect("the bundled en_US dictionary parses");
        Dictionary {
            language: BUNDLED_LANGUAGE.to_string(),
            inner,
        }
    }

    /// Parses a dictionary from its two files' text.
    pub fn parse(language: &str, aff: &str, dic: &str) -> Result<Dictionary, LoadError> {
        let inner =
            spellbook::Dictionary::new(aff, dic).map_err(|e| LoadError::Parse(e.to_string()))?;
        Ok(Dictionary {
            language: language.to_string(),
            inner,
        })
    }

    /// Reads a dictionary from its `.aff` and `.dic` files. Hunspell files
    /// name their encoding on a `SET` line; UTF-8 and ISO-8859-1 (the two
    /// the system packages use for nearly every language) are read, others
    /// refused.
    pub fn load(language: &str, aff: &Path, dic: &Path) -> Result<Dictionary, LoadError> {
        let aff_bytes = std::fs::read(aff).map_err(LoadError::Io)?;
        let dic_bytes = std::fs::read(dic).map_err(LoadError::Io)?;
        let encoding = declared_encoding(&aff_bytes);
        let aff_text = decode(&aff_bytes, &encoding)?;
        let dic_text = decode(&dic_bytes, &encoding)?;
        Dictionary::parse(language, &aff_text, &dic_text)
    }

    /// Whether the dictionary knows `word`, Hunspell's rules (affixes,
    /// compounds, case) applied.
    pub fn check(&self, word: &str) -> bool {
        self.inner.check(word)
    }

    /// Corrections for `word`, best first.
    pub fn suggest(&self, word: &str) -> Vec<String> {
        let mut out = Vec::new();
        self.inner.suggest(word, &mut out);
        out
    }
}

/// The encoding an `.aff` file declares on its `SET` line, upper-cased;
/// ISO8859-1 when it has none (Hunspell's default).
fn declared_encoding(aff: &[u8]) -> String {
    for line in aff.split(|&b| b == b'\n') {
        let line = String::from_utf8_lossy(line);
        let line = line.trim_start_matches('\u{feff}').trim();
        if let Some(rest) = line.strip_prefix("SET") {
            if rest.starts_with(char::is_whitespace) {
                return rest.trim().to_ascii_uppercase();
            }
        }
    }
    "ISO8859-1".to_string()
}

fn decode(bytes: &[u8], encoding: &str) -> Result<String, LoadError> {
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    match encoding.replace('_', "-").as_str() {
        "UTF-8" | "UTF8" => Ok(String::from_utf8_lossy(bytes).into_owned()),
        "ISO8859-1" | "ISO-8859-1" | "LATIN1" => Ok(bytes.iter().map(|&b| b as char).collect()),
        other => {
            // A file that says one thing and is plain ASCII reads the same either way.
            if bytes.is_ascii() {
                Ok(String::from_utf8_lossy(bytes).into_owned())
            } else {
                Err(LoadError::Encoding(other.to_string()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_dictionary_checks_and_suggests() {
        let d = Dictionary::bundled();
        assert_eq!(d.language, "en-US");
        for w in [
            "receive", "world", "Obsidian", "don't", "it's", "naive", "42nd", "silica",
        ] {
            assert!(d.check(w), "{w} is a word");
        }
        for w in ["recieve", "wrold", "teh", "MISPELED"] {
            assert!(!d.check(w), "{w} is not a word");
        }
        let s = d.suggest("recieve");
        assert_eq!(s.first().map(String::as_str), Some("receive"), "{s:?}");
    }

    #[test]
    fn a_latin1_dictionary_is_decoded() {
        let dir = tempfile::tempdir().unwrap();
        let aff = dir.path().join("xx_XX.aff");
        let dic = dir.path().join("xx_XX.dic");
        std::fs::write(&aff, b"SET ISO8859-1\n").unwrap();
        std::fs::write(&dic, b"2\ncaf\xe9\nna\xefve\n").unwrap();
        let d = Dictionary::load("xx-XX", &aff, &dic).unwrap();
        assert!(d.check("café"));
        assert!(d.check("naïve"));
        assert!(!d.check("cafe"));
    }

    #[test]
    fn an_unknown_encoding_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let aff = dir.path().join("xx.aff");
        let dic = dir.path().join("xx.dic");
        std::fs::write(&aff, b"SET KOI8-R\n").unwrap();
        std::fs::write(&dic, b"1\n\xc1\xc2\n").unwrap();
        assert!(matches!(
            Dictionary::load("xx", &aff, &dic),
            Err(LoadError::Encoding(_))
        ));
    }
}
