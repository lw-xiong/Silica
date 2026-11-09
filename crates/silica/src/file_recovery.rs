//! File recovery: list every snapshot Silica has taken of every note, and
//! restore one. Reads the `.obsidian/.silica-versions` store
//! ([`crate::history::History`]) and writes restored bytes back through the
//! vault's own write path ([`silica_vault::Vault::write_bytes`]), never
//! directly.

use std::path::Path;

use silica_vault::Vault;

/// One snapshot of one note, newest first within the note.
#[derive(Debug, Clone)]
pub struct NoteSnapshot {
    /// Vault-relative path of the note.
    pub path: String,
    /// The note's snapshots, newest first.
    pub versions: Vec<crate::history::Version>,
}

/// Collects every markdown note that has at least one snapshot, scanning
/// `files` (vault-relative paths). Notes without history are skipped.
pub fn collect(
    vault_root: &Path,
    files: impl Iterator<Item = impl AsRef<str>>,
) -> Vec<NoteSnapshot> {
    let history = crate::history::History::new(vault_root, ".obsidian");
    let mut out = Vec::new();
    for file in files {
        let path = file.as_ref();
        let versions = history.list(path);
        if !versions.is_empty() {
            out.push(NoteSnapshot {
                path: path.to_string(),
                versions,
            });
        }
    }
    out
}

/// Restores the snapshot taken at `at` of `path`: reads its bytes from the
/// store and writes them back through the vault's own write path. Returns
/// the restored path.
pub fn restore(vault: &Vault, vault_root: &Path, path: &str, at: u64) -> Result<String, String> {
    let history = crate::history::History::new(vault_root, ".obsidian");
    let bytes = history
        .read(path, at)
        .ok_or_else(|| format!("snapshot of {path} at {at} is gone"))?;
    vault
        .write_bytes(path, bytes.as_bytes())
        .map_err(|err| format!("could not restore {path}: {err}"))?;
    Ok(path.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A vault with one note and one recorded snapshot of it.
    fn vault_with_snapshot(root: &Path) -> Vault {
        std::fs::create_dir_all(root.join(".obsidian")).unwrap();
        std::fs::write(root.join("A.md"), "# first\n").unwrap();
        let history = crate::history::History::new(root, ".obsidian");
        history.record("A.md", "# first\n");
        // * The stamp is nanoseconds and two saves can land on the same one,
        //   so the second snapshot waits a beat to be distinguishable.
        std::thread::sleep(std::time::Duration::from_millis(2));
        std::fs::write(root.join("A.md"), "# second\n").unwrap();
        history.record("A.md", "# second\n");
        Vault::open(root).unwrap()
    }

    /// Every note with a snapshot is listed, newest snapshot first, and a
    /// note with no history is not.
    #[test]
    fn collect_lists_only_notes_with_history_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("B.md"), "# never saved\n").unwrap();
        let _vault = vault_with_snapshot(root);

        let entries = collect(root, ["A.md", "B.md"].into_iter());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "A.md");
        assert_eq!(entries[0].versions.len(), 2);
        assert!(
            entries[0].versions[0].at > entries[0].versions[1].at,
            "newest snapshot first"
        );
    }

    /// The restore writes the snapshot's bytes back through the vault's own
    /// write path, so the note on disk is the old content again.
    #[test]
    fn restore_writes_the_snapshotted_bytes_back() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let vault = vault_with_snapshot(root);

        let entries = collect(root, ["A.md"].into_iter());
        let newest = entries[0].versions[0].at;
        let restored = restore(&vault, root, "A.md", newest).unwrap();
        assert_eq!(restored, "A.md");
        assert_eq!(
            std::fs::read_to_string(root.join("A.md")).unwrap(),
            "# second\n"
        );
    }

    /// An older snapshot is restorable too, and it overwrites what the note
    /// holds now.
    #[test]
    fn restore_of_an_older_snapshot_overwrites_the_current_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let vault = vault_with_snapshot(root);

        let entries = collect(root, ["A.md"].into_iter());
        let oldest = entries[0].versions.last().unwrap().at;
        restore(&vault, root, "A.md", oldest).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("A.md")).unwrap(),
            "# first\n"
        );
    }

    /// A snapshot that is gone is reported, not silently written as empty.
    #[test]
    fn restore_of_a_missing_snapshot_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let vault = vault_with_snapshot(root);

        let err = restore(&vault, root, "A.md", 1).unwrap_err();
        assert!(err.contains("A.md"), "{err}");
        let err = restore(&vault, root, "Nope.md", 1).unwrap_err();
        assert!(err.contains("Nope.md"), "{err}");
    }
}
