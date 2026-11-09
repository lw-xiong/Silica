//! Persisted fold state for the reading view: one entry per note, kept
//! outside the vault.
//!
//! Every entry is keyed by the note's vault-relative path and holds the
//! indexes of its folded headings in the reading view's heading order.
//! The record lives in `<Silica config dir>/reading-folds.json` — the same
//! per-user application folder that holds `silica.json` — and never inside
//! the user's vault folder. A store with nowhere to live keeps folds for
//! the session only.
//!
//! Heading-list indexes are used instead of fold keys: a fold is restored
//! by position when the note is reopened, and a heading that moved or went
//! away simply fails to resolve, which leaves the note unfolded — the same
//! outcome as a note never folded before.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::PathBuf;

/// The folded headings of every note the reading view has shown, keyed by
/// vault-relative path. Each `usize` is the index of a folded heading in
/// the view's `headings()` order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadingFoldStore {
    notes: HashMap<String, HashSet<usize>>,
    path: Option<PathBuf>,
}

impl ReadingFoldStore {
    /// Opens the store from `<config dir>/reading-folds.json`. A missing
    /// file or a missing configuration folder yields an empty store that
    /// still records changes — they are silently dropped when there is
    /// nowhere to write.
    pub fn open() -> ReadingFoldStore {
        let path = silica_vault::registry::config_dir().map(|dir| dir.join("reading-folds.json"));
        let notes = path
            .as_ref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        ReadingFoldStore { notes, path }
    }

    /// The saved folded-heading indexes for `path`, if any were saved.
    pub fn get(&self, path: &str) -> Option<&HashSet<usize>> {
        self.notes.get(path)
    }

    /// Records `indexes` for `path` and writes the store. An empty set
    /// removes the note's entry. Returns whether the store changed.
    pub fn put(&mut self, path: &str, indexes: HashSet<usize>) -> bool {
        let changed = if indexes.is_empty() {
            self.notes.remove(path).is_some()
        } else {
            self.notes.get(path) != Some(&indexes) && {
                self.notes.insert(path.to_string(), indexes);
                true
            }
        };
        if changed {
            self.write();
        }
        changed
    }

    fn write(&self) {
        let Some(path) = &self.path else {
            return;
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string_pretty(&self.notes) {
            let _ = std::fs::write(path, text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fold_set_round_trips_outside_the_vault() {
        let dir = tempfile::tempdir().expect("a temp dir");
        std::env::set_var("SILICA_CONFIG_DIR", dir.path());
        let mut store = ReadingFoldStore::open();
        store.put("Sub Folder/Fold Probe.md", HashSet::from([0, 3]));
        store.put("Other.md", HashSet::new()); // empty set: no entry kept

        let again = ReadingFoldStore::open();
        assert_eq!(
            again.get("Sub Folder/Fold Probe.md"),
            Some(&HashSet::from([0, 3]))
        );
        assert!(again.get("Other.md").is_none());
        assert!(!dir.path().join("vault").exists());

        // * The file is under the configuration folder, not the vault.
        assert!(dir.path().join("reading-folds.json").exists());
        std::env::remove_var("SILICA_CONFIG_DIR");
    }
}
