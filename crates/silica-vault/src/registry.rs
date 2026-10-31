//! The vaults this machine has opened: what the vault switcher lists.
//!
//! Obsidian keeps the list outside every vault, in `obsidian.json` under
//! the app's configuration folder, as `{"vaults": {<id>: {"path", "ts",
//! "open"}}}` — a sixteen-hex-digit id per vault, the folder's absolute
//! path, the millisecond time it was last opened, and `open: true` on the
//! vaults a window is showing. Silica writes the same document as
//! `silica.json` under its own folder (`~/.config/silica` on Linux,
//! `%APPDATA%\silica` on Windows, `~/Library/Application Support/silica`
//! on macOS, or `$SILICA_CONFIG_DIR`), never inside a vault, and keeps
//! every key it does not know.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

/// The file's name under the configuration folder.
pub const FILE_NAME: &str = "silica.json";

/// One remembered vault.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultEntry {
    /// The vault folder, absolute.
    pub path: PathBuf,
    /// When it was last opened, in milliseconds since the epoch.
    pub ts: i64,
    /// Whether a window is showing it now.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub open: bool,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl VaultEntry {
    /// The vault's name: its folder's, as the switcher shows it.
    pub fn name(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.display().to_string())
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct Document {
    #[serde(default)]
    vaults: IndexMap<String, VaultEntry>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

/// The list, read from and written back to one file.
#[derive(Debug, Clone)]
pub struct VaultRegistry {
    path: PathBuf,
    doc: Document,
}

impl VaultRegistry {
    /// Where the file lives on this machine, or `None` when the desktop
    /// gives no configuration folder to put it in.
    pub fn default_path() -> Option<PathBuf> {
        Some(config_dir()?.join(FILE_NAME))
    }

    /// Reads the file at `path`; a missing file is an empty list, and a
    /// file that is not the document is an error rather than something to
    /// overwrite.
    pub fn load(path: impl Into<PathBuf>) -> std::io::Result<VaultRegistry> {
        let path = path.into();
        let doc = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("{}: {e}", path.display()),
                )
            })?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Document::default(),
            Err(e) => return Err(e),
        };
        Ok(VaultRegistry { path, doc })
    }

    /// The machine's own list, from [`Self::default_path`].
    pub fn open() -> std::io::Result<Option<VaultRegistry>> {
        Self::default_path().map(Self::load).transpose()
    }

    /// Where this list is written.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The vaults in the document's own order, which is the order the
    /// switcher lists them in: a vault keeps the place it was first opened
    /// in and a new one goes last, whatever `ts` says. Obsidian's switcher
    /// does the same — the menu of a registry whose entries were out of
    /// recency order listed them in key order, with the open vault fourth
    /// of six rather than first (probe of 2026-10-29).
    pub fn vaults(&self) -> Vec<(&str, &VaultEntry)> {
        self.doc
            .vaults
            .iter()
            .map(|(id, entry)| (id.as_str(), entry))
            .collect()
    }

    /// The entry for `root`, if the folder is on the list.
    pub fn find(&self, root: &Path) -> Option<(&str, &VaultEntry)> {
        self.doc
            .vaults
            .iter()
            .find(|(_, e)| same_folder(&e.path, root))
            .map(|(id, e)| (id.as_str(), e))
    }

    /// Notes that `root` is the vault this window shows now: its entry
    /// (made if it is new) gets the time and `open`, and every other entry
    /// loses `open`, since Silica shows one vault per process. Returns the
    /// vault's id.
    pub fn record_open(&mut self, root: &Path, now_ms: i64) -> String {
        let root = absolute(root);
        for entry in self.doc.vaults.values_mut() {
            entry.open = false;
        }
        let id = match self.find(&root) {
            Some((id, _)) => id.to_string(),
            None => {
                let id = fresh_id(&root, now_ms, |id| !self.doc.vaults.contains_key(id));
                self.doc.vaults.insert(
                    id.clone(),
                    VaultEntry {
                        path: root.clone(),
                        ts: now_ms,
                        open: false,
                        extra: Map::new(),
                    },
                );
                id
            }
        };
        let entry = self.doc.vaults.get_mut(&id).expect("just inserted");
        entry.path = root;
        entry.ts = now_ms;
        entry.open = true;
        id
    }

    /// Drops the entry with `id`, which does not touch the folder.
    pub fn forget(&mut self, id: &str) -> bool {
        self.doc.vaults.shift_remove(id).is_some()
    }

    /// Writes the document back, making the folder it lives in.
    pub fn save(&self) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string(&self.doc)?;
        std::fs::write(&self.path, text)
    }
}

/// The folder `silica.json` lives under: `$SILICA_CONFIG_DIR` when set,
/// otherwise the desktop's per-user configuration folder. Everything
/// Silica keeps outside a vault goes under it.
pub fn config_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("SILICA_CONFIG_DIR") {
        return Some(PathBuf::from(dir));
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join("AppData/Roaming"))
            })
            .map(|p| p.join("silica"))
    } else if cfg!(target_os = "macos") {
        home.map(|h| h.join("Library/Application Support/silica"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| home.map(|h| h.join(".config")))
            .map(|p| p.join("silica"))
    }
}

/// `root` as an absolute path, so the same folder opened from two working
/// directories is one entry. A relative path that cannot be resolved
/// stays as given.
fn absolute(root: &Path) -> PathBuf {
    if root.is_absolute() {
        root.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(root))
            .unwrap_or_else(|_| root.to_path_buf())
    }
}

/// Whether two entries name one folder: by path, case-insensitively on
/// Windows where the file system is.
fn same_folder(a: &Path, b: &Path) -> bool {
    if cfg!(target_os = "windows") {
        a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
    } else {
        a == b
    }
}

/// A sixteen-hex-digit id like Obsidian's, from the path and the time so
/// two machines opening the same folder do not collide on the id. `free`
/// says whether an id is unused; the time is stepped until one is.
fn fresh_id(root: &Path, now_ms: i64, free: impl Fn(&str) -> bool) -> String {
    let mut salt = now_ms;
    loop {
        let mut h = DefaultHasher::new();
        root.hash(&mut h);
        salt.hash(&mut h);
        let id = format!("{:016x}", h.finish());
        if free(&id) {
            return id;
        }
        salt += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_is_an_empty_list_and_saving_makes_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config/silica/silica.json");
        let mut reg = VaultRegistry::load(&path).unwrap();
        assert!(reg.vaults().is_empty());
        let id = reg.record_open(&dir.path().join("Notes"), 1000);
        assert_eq!(id.len(), 16);
        reg.save().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"open\":true"), "{text}");
        assert!(text.contains("\"ts\":1000"), "{text}");
    }

    #[test]
    fn opening_again_keeps_the_id_and_the_place_and_clears_the_others() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("silica.json");
        let mut reg = VaultRegistry::load(&path).unwrap();
        let a = reg.record_open(&dir.path().join("A"), 1);
        let b = reg.record_open(&dir.path().join("B"), 2);
        assert_ne!(a, b);
        let again = reg.record_open(&dir.path().join("A"), 3);
        assert_eq!(again, a);
        // Reopening A does not move it: the list keeps the document's order,
        // the order the vaults were first opened in.
        let listed = reg.vaults();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].0, a);
        assert!(listed[0].1.open);
        assert_eq!(listed[1].0, b);
        assert!(!listed[1].1.open);
        assert!(reg.forget(&b));
        assert_eq!(reg.vaults().len(), 1);
    }

    /// The fixture's vault folders are absolute on the platform the test
    /// runs on: a rootless path is relative on Windows and would be
    /// resolved against the current drive.
    const V: &str = if cfg!(windows) { "C:/v" } else { "/v" };

    #[test]
    fn obsidians_document_round_trips_with_its_unknown_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("silica.json");
        std::fs::write(
            &path,
            format!(
                r#"{{"vaults":{{"d9bdd8d517a88dd3":{{"path":"{V}/one","ts":1792091578741,"open":true,"color":"red"}},"0260ef8a33e2e677":{{"path":"{V}/two","ts":1792369318734}}}},"updateChannel":"insider"}}"#
            ),
        )
        .unwrap();
        let mut reg = VaultRegistry::load(&path).unwrap();
        let (one, two) = (format!("{V}/one"), format!("{V}/two"));
        // Key order, not recency: `two` is the newer `ts` and still second.
        assert_eq!(reg.vaults()[0].1.path, Path::new(&one));
        assert_eq!(reg.vaults()[1].1.path, Path::new(&two));
        assert_eq!(reg.find(Path::new(&one)).unwrap().0, "d9bdd8d517a88dd3");
        reg.record_open(Path::new(&two), 1792400000000);
        reg.save().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains(r#""color":"red""#), "{text}");
        assert!(text.contains(r#""updateChannel":"insider""#), "{text}");
        assert!(
            text.contains(&format!(
                r#""0260ef8a33e2e677":{{"path":"{two}","ts":1792400000000,"open":true}}"#
            )),
            "{text}"
        );
        assert!(
            !text.contains(&format!(r#""{one}","ts":1792091578741,"open""#)),
            "one lost open: {text}"
        );
    }

    #[test]
    fn a_new_vault_goes_last_and_an_old_one_stays_where_it_is() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("silica.json");
        let mut reg = VaultRegistry::load(&path).unwrap();
        let a = reg.record_open(&dir.path().join("A"), 10);
        let b = reg.record_open(&dir.path().join("B"), 20);
        reg.record_open(&dir.path().join("A"), 30);
        let c = reg.record_open(&dir.path().join("C"), 40);
        let ids: Vec<&str> = reg.vaults().into_iter().map(|(id, _)| id).collect();
        assert_eq!(ids, vec![a.as_str(), b.as_str(), c.as_str()]);
    }

    #[test]
    fn a_file_that_is_not_the_document_is_an_error_not_a_blank_slate() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("silica.json");
        std::fs::write(&path, "not json").unwrap();
        assert!(VaultRegistry::load(&path).is_err());
    }
}
