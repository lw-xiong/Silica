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

/// The key a vault's entry carries when its config folder is not
/// `.obsidian`: Obsidian's "Override config folder", kept outside the vault
/// because the folder it names is the one that would hold it.
const CONFIG_FOLDER_KEY: &str = "configFolder";

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
    ///
    /// The bytes land in a unique companion in the same folder and are renamed
    /// onto the registry, so a crash mid-write leaves the old document in
    /// place instead of a truncated JSON file that nothing can parse. Renaming
    /// within one folder is atomic on every platform Silica ships.
    pub fn save(&self) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string(&self.doc)?;
        let temp = self.path.with_file_name(format!(
            ".{}.{}.silica-tmp",
            self.path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "registry".to_string()),
            std::process::id()
        ));
        {
            use std::io::Write as _;
            let mut file = std::fs::File::create(&temp)?;
            file.write_all(text.as_bytes())?;
            // * Flushed to the device before the rename, so the name never
            // * points at bytes still only in the page cache.
            file.sync_all()?;
        }
        match std::fs::rename(&temp, &self.path) {
            Ok(()) => {
                // * The directory entry is what a crash could lose.
                if let Some(dir) = self.path.parent() {
                    if let Ok(handle) = std::fs::File::open(dir) {
                        let _ = handle.sync_all();
                    }
                }
                // * A process that died between its create and its rename left
                // * its companion behind, and nothing else ever removes it: the
                // * configuration folder would collect a hidden file per
                // * crashed run. The rename just succeeded, so this is the one
                // * moment it is safe to sweep the others — best effort, since
                // * failing to collect junk is not a reason to fail a save.
                self.sweep_dead_companions();
                Ok(())
            }
            Err(error) => {
                let _ = std::fs::remove_file(&temp);
                Err(error)
            }
        }
    }

    /// Removes the companions of a save that never finished, in the folder
    /// this registry lives in. Only files this registry itself would have
    /// written are touched: any other file in the user's configuration folder
    /// is left exactly as it is.
    fn sweep_dead_companions(&self) {
        let Some(dir) = self.path.parent() else {
            return;
        };
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let Some(stem) = self
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
        else {
            return;
        };
        let prefix = format!(".{stem}.");
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(&prefix) && name.ends_with(".silica-tmp") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }

    /// A document-level string setting, kept beside `vaults` rather than in
    /// a vault: Obsidian keeps "Window frame style" outside `.obsidian`
    /// (the probe of 2026-10-20 wrote nothing there), and this file is the
    /// per-user place Silica already has.
    pub fn setting(&self, key: &str) -> Option<&str> {
        self.doc.extra.get(key).and_then(|v| v.as_str())
    }

    /// Sets one, removing it when `value` is `None`.
    pub fn set_setting(&mut self, key: &str, value: Option<&str>) {
        match value {
            Some(v) => {
                self.doc
                    .extra
                    .insert(key.to_string(), Value::String(v.to_string()));
            }
            None => {
                self.doc.extra.shift_remove(key);
            }
        }
    }

    /// The config folder this vault was opened with, when it is not the
    /// default `.obsidian`: Obsidian's "Override config folder". Kept on the
    /// vault's own entry so two vaults can differ.
    pub fn config_folder(&self, root: &Path) -> Option<String> {
        self.find(root).and_then(|(_, e)| {
            e.extra
                .get(CONFIG_FOLDER_KEY)
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
    }

    /// Sets or clears it.
    pub fn set_config_folder(&mut self, root: &Path, folder: Option<&str>) {
        let root = absolute(root);
        let Some((id, _)) = self.find(&root).map(|(id, _)| (id.to_string(), ())) else {
            return;
        };
        if let Some(entry) = self.doc.vaults.get_mut(&id) {
            match folder {
                Some(f) => {
                    entry
                        .extra
                        .insert(CONFIG_FOLDER_KEY.to_string(), Value::String(f.to_string()));
                }
                None => {
                    entry.extra.shift_remove(CONFIG_FOLDER_KEY);
                }
            }
        }
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

    /// The write is a companion plus a rename, so a save leaves exactly one
    /// file behind — the registry itself. A scratch file left sitting in the
    /// configuration folder would be junk the user cannot see and no code
    /// removes. And a scratch file a *crashed* process left behind must not
    /// break the next load: the list is read from the registry, and the
    /// stray file is simply not it.
    #[test]
    fn a_save_leaves_no_scratch_and_a_crashed_one_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config/silica/silica.json");

        // * What a process killed between the create and the rename leaves.
        let stray = dir.path().join("config/silica/.silica.json.999.silica-tmp");
        std::fs::create_dir_all(stray.parent().unwrap()).unwrap();
        std::fs::write(&stray, "{\"vaults\":{\"deadbee").unwrap();

        let mut reg = VaultRegistry::load(&path).unwrap();
        let id = reg.record_open(&dir.path().join("Notes"), 1000);
        reg.save().unwrap();

        let entries: Vec<String> = std::fs::read_dir(dir.path().join("config/silica"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            entries
                .iter()
                .filter(|n| n.as_str() == "silica.json")
                .count(),
            1,
            "the registry file is there: {entries:?}"
        );
        assert!(
            entries.iter().all(|n| !n.ends_with(".999.silica-tmp")),
            "a save cleaned up after the crash: {entries:?}"
        );

        // * A save also leaves nothing of its own behind: one registry file.
        let owned: Vec<&String> = entries
            .iter()
            .filter(|n| n.ends_with(".silica-tmp"))
            .collect();
        assert!(
            owned.is_empty(),
            "a save left its scratch behind: {owned:?} — all entries: {entries:?}"
        );

        // * A file that is not this registry's companion is never touched:
        // * the sweep must not eat the user's configuration folder.
        let keep = dir.path().join("config/silica/keep-me.txt");
        std::fs::write(&keep, "mine\n").unwrap();
        let mut reg = VaultRegistry::load(&path).unwrap();
        reg.record_open(&dir.path().join("Other"), 2000);
        reg.save().unwrap();
        assert!(keep.is_file(), "the sweep removed a file it does not own");
        assert_eq!(std::fs::read_to_string(&keep).unwrap(), "mine\n");

        // * The stray never became part of the list, and the load still works:
        // * two vaults (Notes, then Other), the original keeping its place.
        let reloaded = VaultRegistry::load(&path).unwrap();
        assert_eq!(reloaded.vaults().len(), 2);
        assert_eq!(reloaded.vaults()[0].0, id.as_str());
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
        // * Reopening A does not move it: the list keeps the document's order,
        // * the order the vaults were first opened in.
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
        // * Key order, not recency: `two` is the newer `ts` and still second.
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
