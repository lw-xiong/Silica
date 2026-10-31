//! A vault is a plain folder. This crate opens one, enumerates its files, and
//! reads `.obsidian` configuration without ever owning data outside the folder.

pub mod ops;
pub mod registry;

pub use ops::{OpError, TrashOption};
pub use registry::{VaultEntry, VaultRegistry};
use serde::de::DeserializeOwned;
use silica_cache::LinkResolver;
use silica_formats::bookmarks::Bookmarks;
use silica_formats::config::{
    AppConfig, AppearanceConfig, CommandPaletteConfig, CommunityPlugins, CorePlugins,
    DailyNotesConfig, GraphConfig, Hotkeys, PluginManifest, PropertyTypes, SwitcherConfig,
    TemplatesConfig, DEFAULT_CONFIG_DIR,
};
use silica_formats::FileKind;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    #[error("vault path is not a directory: {0}")]
    NotADirectory(PathBuf),
    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid JSON in {path}: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("{0} is not a path inside the vault")]
    OutsideVault(String),
    #[error("{0} is a symbolic link; the vault folder is the only place Silica writes")]
    Symlink(PathBuf),
    /// A config file on disk that nothing can parse. Writing it would
    /// replace every key it holds with the handful Silica models, so the
    /// write is refused and the bytes are left alone.
    #[error("{0} is not valid JSON; Silica will not overwrite it")]
    Unparseable(PathBuf),
}

/// One file in the vault. `path` is vault-relative with `/` separators.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultFile {
    pub path: String,
    pub kind: FileKind,
    pub size: u64,
    /// Milliseconds since the Unix epoch, like Obsidian's `TFile.stat.mtime`:
    /// the link suggester orders an empty query by it, newest first. Zero
    /// when the filesystem does not say.
    pub mtime: u64,
    /// Creation time in the same units, like `TFile.stat.ctime`: the Files
    /// view's two "Created time" orders sort by it. The modification time
    /// stands in on a filesystem that keeps no birth time.
    pub ctime: u64,
}

/// A file's modification time in milliseconds since the Unix epoch.
fn mtime_ms(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// A file's creation time in milliseconds since the Unix epoch, or its
/// modification time where the filesystem records no birth time.
fn ctime_ms(meta: &std::fs::Metadata) -> u64 {
    meta.created()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or_else(|| mtime_ms(meta))
}

/// What [`Vault::refresh_path`] found; paths are vault-relative.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileChange {
    Added(String),
    Modified(String),
    Removed(String),
}

impl FileChange {
    pub fn path(&self) -> &str {
        match self {
            FileChange::Added(p) | FileChange::Modified(p) | FileChange::Removed(p) => p,
        }
    }
}

/// Writes a file whole without following a symbolic link in its place: on
/// Unix the file is opened with `O_NOFOLLOW`, so a link swapped in between
/// the path check and the write still cannot redirect it. Elsewhere a link
/// needs privileges to make and the path check alone stands.
pub fn write_no_follow(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)
}

#[derive(Debug)]
pub struct Vault {
    root: PathBuf,
    config_dir: String,
    files: Vec<VaultFile>,
    /// Every folder, vault-relative and sorted, empty ones included (the
    /// Files view shows them; the file list alone would not).
    folders: Vec<String>,
}

impl Vault {
    /// Opens a vault with the default `.obsidian` config directory.
    pub fn open(root: impl AsRef<Path>) -> Result<Vault, VaultError> {
        Vault::open_with_config_dir(root, DEFAULT_CONFIG_DIR)
    }

    pub fn open_with_config_dir(
        root: impl AsRef<Path>,
        config_dir: &str,
    ) -> Result<Vault, VaultError> {
        let root = root.as_ref().to_path_buf();
        if !root.is_dir() {
            return Err(VaultError::NotADirectory(root));
        }
        let mut vault = Vault {
            root,
            config_dir: config_dir.to_string(),
            files: Vec::new(),
            folders: Vec::new(),
        };
        vault.rescan()?;
        Ok(vault)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn name(&self) -> String {
        self.root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    pub fn config_dir(&self) -> &str {
        &self.config_dir
    }

    pub fn files(&self) -> &[VaultFile] {
        &self.files
    }

    /// Every folder in the vault (dot-folders excluded), sorted by path.
    pub fn folders(&self) -> &[String] {
        &self.folders
    }

    fn add_folder(&mut self, rel: &str) {
        if let Err(ix) = self.folders.binary_search_by(|f| f.as_str().cmp(rel)) {
            self.folders.insert(ix, rel.to_string());
        }
    }

    /// Registers every folder above `rel` (a file or folder path).
    fn add_ancestors(&mut self, rel: &str) {
        let mut end = 0;
        while let Some(i) = rel[end..].find('/') {
            end += i;
            self.add_folder(&rel[..end]);
            end += 1;
        }
    }

    pub fn markdown_files(&self) -> impl Iterator<Item = &VaultFile> {
        self.files.iter().filter(|f| f.kind == FileKind::Markdown)
    }

    /// Absolute path for a vault-relative path.
    pub fn absolute(&self, rel: &str) -> PathBuf {
        self.root
            .join(rel.replace('/', std::path::MAIN_SEPARATOR_STR))
    }

    /// The absolute path a write to `rel` may go to. The vault folder is the
    /// only place Silica writes, so this refuses what would reach outside
    /// it: a `..` or an absolute path, and a symbolic link anywhere along
    /// the way — a link inside the vault pointing at `~/.ssh` must not turn
    /// a note save into a write there. Every write in the workspace goes
    /// through here (and [`write_no_follow`]), reads do not.
    pub fn write_path(&self, rel: &str) -> Result<PathBuf, VaultError> {
        self.checked_path(rel, false)
    }

    /// Like [`Vault::write_path`] for an entry that is moved or removed as a
    /// whole: the entry itself may be a link, since renaming or deleting a
    /// link touches the link and never what it points at; its ancestors
    /// may not.
    pub fn entry_path(&self, rel: &str) -> Result<PathBuf, VaultError> {
        self.checked_path(rel, true)
    }

    fn checked_path(&self, rel: &str, leaf_may_link: bool) -> Result<PathBuf, VaultError> {
        let mut absolute = self.root.clone();
        let mut parts = Vec::new();
        for component in Path::new(rel).components() {
            match component {
                std::path::Component::Normal(part) => parts.push(part.to_os_string()),
                std::path::Component::CurDir => {}
                _ => return Err(VaultError::OutsideVault(rel.to_string())),
            }
        }
        if parts.is_empty() {
            return Err(VaultError::OutsideVault(rel.to_string()));
        }
        let last = parts.len() - 1;
        for (ix, part) in parts.into_iter().enumerate() {
            absolute.push(part);
            if ix == last && leaf_may_link {
                break;
            }
            match std::fs::symlink_metadata(&absolute) {
                Ok(meta) if meta.file_type().is_symlink() => {
                    return Err(VaultError::Symlink(absolute));
                }
                // What does not exist yet is created below the vault.
                _ => {}
            }
        }
        Ok(absolute)
    }

    /// Writes `bytes` to a vault-relative path, through [`Vault::write_path`]
    /// and without following a link at the file itself.
    pub fn write_bytes(&self, rel: &str, bytes: &[u8]) -> Result<(), VaultError> {
        let path = self.write_path(rel)?;
        write_no_follow(&path, bytes).map_err(|source| VaultError::Io { path, source })
    }

    /// Re-enumerates the folder. Skips the config directory, `.trash`, and any
    /// dot-prefixed directory; keeps dot-files out of the file list as Obsidian does.
    pub fn rescan(&mut self) -> Result<(), VaultError> {
        let mut files = Vec::new();
        let mut folders = Vec::new();
        let walker = walkdir::WalkDir::new(&self.root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                let name = e.file_name().to_string_lossy();
                !(e.depth() > 0 && name.starts_with('.'))
            });
        for entry in walker {
            let entry = entry.map_err(|e| VaultError::Io {
                path: e.path().map(Path::to_path_buf).unwrap_or_default(),
                source: e
                    .into_io_error()
                    .unwrap_or_else(|| std::io::Error::other("walk")),
            })?;
            let rel = entry
                .path()
                .strip_prefix(&self.root)
                .expect("entry under root")
                .to_string_lossy()
                .replace('\\', "/");
            if entry.file_type().is_dir() {
                if entry.depth() > 0 {
                    folders.push(rel);
                }
                continue;
            }
            if !entry.file_type().is_file() {
                continue;
            }
            let (size, mtime, ctime) = entry
                .metadata()
                .map(|m| (m.len(), mtime_ms(&m), ctime_ms(&m)))
                .unwrap_or((0, 0, 0));
            files.push(VaultFile {
                kind: FileKind::from_path(&rel),
                path: rel,
                size,
                mtime,
                ctime,
            });
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        folders.sort();
        self.files = files;
        self.folders = folders;
        Ok(())
    }

    /// Re-checks one path after a filesystem event and updates the file
    /// list: a file that exists is added or refreshed, a folder that exists
    /// is walked, and anything missing is removed together with everything
    /// under it. Paths inside dot-folders (the config directory, `.trash`)
    /// are ignored.
    pub fn refresh_path(&mut self, path: &Path) -> Vec<FileChange> {
        let Ok(rel_path) = path.strip_prefix(&self.root) else {
            return Vec::new();
        };
        let rel = rel_path.to_string_lossy().replace('\\', "/");
        if rel.is_empty() {
            return Vec::new();
        }
        if rel.split('/').any(|part| part.starts_with('.')) {
            return Vec::new();
        }
        let mut changes = Vec::new();
        match std::fs::metadata(path) {
            Ok(meta) if meta.is_file() => {
                self.add_ancestors(&rel);
                let file = VaultFile {
                    kind: FileKind::from_path(&rel),
                    path: rel.clone(),
                    size: meta.len(),
                    mtime: mtime_ms(&meta),
                    ctime: ctime_ms(&meta),
                };
                match self.files.binary_search_by(|f| f.path.cmp(&rel)) {
                    Ok(ix) => {
                        self.files[ix] = file;
                        changes.push(FileChange::Modified(rel));
                    }
                    Err(ix) => {
                        self.files.insert(ix, file);
                        changes.push(FileChange::Added(rel));
                    }
                }
            }
            Ok(meta) if meta.is_dir() => {
                self.add_ancestors(&rel);
                self.add_folder(&rel);
                let prefix = format!("{rel}/");
                let before: Vec<String> = self
                    .files
                    .iter()
                    .filter(|f| f.path.starts_with(&prefix))
                    .map(|f| f.path.clone())
                    .collect();
                let mut seen = Vec::new();
                for entry in walkdir::WalkDir::new(path)
                    .follow_links(false)
                    .into_iter()
                    .filter_entry(|e| {
                        !(e.depth() > 0 && e.file_name().to_string_lossy().starts_with('.'))
                    })
                    .flatten()
                {
                    if entry.file_type().is_file() {
                        seen.push(entry.path().to_path_buf());
                    } else if entry.file_type().is_dir() && entry.depth() > 0 {
                        let sub = entry
                            .path()
                            .strip_prefix(&self.root)
                            .map(|p| p.to_string_lossy().replace('\\', "/"))
                            .unwrap_or_default();
                        self.add_folder(&sub);
                    }
                }
                for file in seen {
                    changes.extend(self.refresh_path(&file));
                }
                for old in before {
                    if !self.root.join(&old).is_file() {
                        changes.extend(self.refresh_path(&self.root.join(&old)));
                    }
                }
            }
            _ => {
                let prefix = format!("{rel}/");
                let removed: Vec<String> = self
                    .files
                    .iter()
                    .filter(|f| f.path == rel || f.path.starts_with(&prefix))
                    .map(|f| f.path.clone())
                    .collect();
                self.files
                    .retain(|f| !(f.path == rel || f.path.starts_with(&prefix)));
                self.folders
                    .retain(|f| !(*f == rel || f.starts_with(&prefix)));
                changes.extend(removed.into_iter().map(FileChange::Removed));
            }
        }
        changes
    }

    /// Builds a link resolver over every file currently known.
    pub fn link_resolver(&self) -> LinkResolver {
        LinkResolver::new(self.files.iter().map(|f| f.path.as_str()))
    }

    /// Reads `<config_dir>/<name>` as JSON. A missing file is `Ok(None)`;
    /// Obsidian treats every config file as optional.
    pub fn read_config<T: DeserializeOwned>(&self, name: &str) -> Result<Option<T>, VaultError> {
        let path = self.root.join(&self.config_dir).join(name);
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(VaultError::Io { path, source }),
        };
        // A byte order mark is not JSON. Windows editors leave one, and a
        // file carrying one is otherwise perfectly readable.
        serde_json::from_str(text.strip_prefix('\u{feff}').unwrap_or(&text))
            .map(Some)
            .map_err(|source| VaultError::Json { path, source })
    }

    /// Writes one `.obsidian` config file, changing only what `value`
    /// changed.
    ///
    /// Obsidian writes these with two-space indentation and no trailing
    /// newline (observed in `types.json`), and every config type here keeps
    /// the keys it does not model in `extra`, so a file Obsidian wrote comes
    /// back with its unknown keys intact. That is not enough on its own for
    /// the Writer gate: a struct serializes its fields in declaration
    /// order, floats as `1.0` where Obsidian wrote `1`, and an `f32` field
    /// cannot reproduce the digits of `0.518713248970312`. So the new value
    /// is merged into the file's own JSON ([`merge_config`]) and every key
    /// it did not change keeps the bytes it had.
    pub fn write_config<T: serde::Serialize>(
        &self,
        name: &str,
        value: &T,
    ) -> Result<(), VaultError> {
        self.write_config_with(name, value, true)
    }

    /// Writes one `.obsidian` config file whole, so that a key `value` no
    /// longer has leaves the file too. For a file that *is* a map of
    /// entries — `hotkeys.json`, where dropping a command's entry is what
    /// restores its defaults — the merge in [`Vault::write_config`] would
    /// keep every entry ever written. The value keeps the file's own entry
    /// order the way the type reads it, and the byte order mark stays.
    pub fn replace_config<T: serde::Serialize>(
        &self,
        name: &str,
        value: &T,
    ) -> Result<(), VaultError> {
        self.write_config_with(name, value, false)
    }

    fn write_config_with<T: serde::Serialize>(
        &self,
        name: &str,
        value: &T,
        merge: bool,
    ) -> Result<(), VaultError> {
        let path = self.write_path(&format!("{}/{name}", self.config_dir))?;
        let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        std::fs::create_dir_all(&dir).map_err(|source| VaultError::Io {
            path: dir.clone(),
            source,
        })?;
        let fresh = serde_json::to_value(value).map_err(|source| VaultError::Json {
            path: path.clone(),
            source,
        })?;
        // A byte order mark is not JSON, so the file has to be parsed
        // without it — and put back with it, since it is the user's byte
        // and not ours to drop.
        let (bom, merged) = match std::fs::read_to_string(&path) {
            Ok(text) => {
                let bom = text.starts_with('\u{feff}');
                let body = text.strip_prefix('\u{feff}').unwrap_or(&text);
                match serde_json::from_str::<serde_json::Value>(body) {
                    Ok(old) if merge => (bom, merge_config(old, fresh)),
                    Ok(_) => (bom, fresh),
                    // A file nobody can parse is a file nobody may
                    // overwrite. Reading it gave defaults, so writing it
                    // would put those defaults on disk in place of
                    // everything the user had -- the one thing the vault
                    // being the source of truth forbids. The bytes stay as
                    // they are and the caller is told which file it was.
                    Err(_) => return Err(VaultError::Unparseable(path)),
                }
            }
            Err(_) => (false, fresh),
        };
        let text = serde_json::to_string_pretty(&merged).map_err(|source| VaultError::Json {
            path: path.clone(),
            source,
        })?;
        let text = if bom { format!("\u{feff}{text}") } else { text };
        write_no_follow(&path, text.as_bytes()).map_err(|source| VaultError::Io { path, source })
    }

    pub fn app_config(&self) -> Result<AppConfig, VaultError> {
        Ok(self.read_config("app.json")?.unwrap_or_default())
    }

    /// The Bookmarks plugin's `bookmarks.json`; empty when there is none.
    pub fn bookmarks(&self) -> Result<Bookmarks, VaultError> {
        Ok(self.read_config("bookmarks.json")?.unwrap_or_default())
    }

    /// Writes `bookmarks.json` whole, as Obsidian does: items are added,
    /// removed and reordered, so a merge would keep what was taken out.
    pub fn write_bookmarks(&self, bookmarks: &Bookmarks) -> Result<(), VaultError> {
        self.replace_config("bookmarks.json", bookmarks)
    }

    pub fn appearance_config(&self) -> Result<AppearanceConfig, VaultError> {
        Ok(self.read_config("appearance.json")?.unwrap_or_default())
    }

    pub fn hotkeys(&self) -> Result<Hotkeys, VaultError> {
        Ok(self.read_config("hotkeys.json")?.unwrap_or_default())
    }

    pub fn core_plugins(&self) -> Result<Option<CorePlugins>, VaultError> {
        self.read_config("core-plugins.json")
    }

    pub fn community_plugins(&self) -> Result<CommunityPlugins, VaultError> {
        Ok(self
            .read_config("community-plugins.json")?
            .unwrap_or_default())
    }

    pub fn property_types(&self) -> Result<PropertyTypes, VaultError> {
        Ok(self.read_config("types.json")?.unwrap_or_default())
    }

    pub fn graph_config(&self) -> Result<GraphConfig, VaultError> {
        Ok(self.read_config("graph.json")?.unwrap_or_default())
    }

    pub fn command_palette_config(&self) -> Result<CommandPaletteConfig, VaultError> {
        Ok(self
            .read_config("command-palette.json")?
            .unwrap_or_default())
    }

    pub fn switcher_config(&self) -> Result<SwitcherConfig, VaultError> {
        Ok(self.read_config("switcher.json")?.unwrap_or_default())
    }

    /// The Templates plugin's settings. Neither this file nor
    /// `daily-notes.json` exists until a setting is written, so a missing
    /// one is the default rather than an error.
    pub fn templates_config(&self) -> Result<TemplatesConfig, VaultError> {
        Ok(self.read_config("templates.json")?.unwrap_or_default())
    }

    pub fn daily_notes_config(&self) -> Result<DailyNotesConfig, VaultError> {
        Ok(self.read_config("daily-notes.json")?.unwrap_or_default())
    }

    /// Manifests of the community plugins installed under `plugins/`, sorted
    /// by folder name; folders without a readable manifest are skipped.
    pub fn plugin_manifests(&self) -> Vec<PluginManifest> {
        let dir = self.root.join(&self.config_dir).join("plugins");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return Vec::new();
        };
        let mut found: Vec<(String, PluginManifest)> = entries
            .flatten()
            .filter_map(|e| {
                let folder = e.file_name().to_string_lossy().into_owned();
                let text = std::fs::read_to_string(e.path().join("manifest.json")).ok()?;
                let mut manifest: PluginManifest = serde_json::from_str(&text).ok()?;
                if manifest.id.is_empty() {
                    manifest.id = folder.clone();
                }
                Some((folder, manifest))
            })
            .collect();
        found.sort_by(|a, b| a.0.cmp(&b.0));
        found.into_iter().map(|(_, m)| m).collect()
    }

    /// The raw workspace layout; typed modelling comes with the workspace milestone.
    pub fn workspace_json(&self) -> Result<Option<serde_json::Value>, VaultError> {
        self.read_config("workspace.json")
    }
}

/// Merges `fresh` (a config value as its type serializes it) into `old` (the
/// file as it is on disk), so that writing a value nothing changed rewrites
/// nothing.
///
/// `serde_json` is built with `preserve_order`, so `old` carries the file's
/// key order and every number exactly as it was written. A key `fresh`
/// agrees with keeps the old node and its position; a key it changed takes
/// the new value in the old key's place; a key only `fresh` has is appended.
/// A key only `old` has stays: either the type does not model it, or it
/// skips serializing it when empty, and neither is a reason to drop what
/// Obsidian wrote.
fn merge_config(old: serde_json::Value, fresh: serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match (old, fresh) {
        (Value::Object(mut merged), Value::Object(fields)) => {
            for (key, value) in fields {
                match merged.get(&key) {
                    Some(prev) if same_config_value(prev, &value) => {}
                    Some(prev) if prev.is_object() && value.is_object() => {
                        // In place: `Map::remove` would move the key to the
                        // end of the file.
                        if let Some(slot) = merged.get_mut(&key) {
                            let prev = std::mem::replace(slot, Value::Null);
                            *slot = merge_config(prev, value);
                        }
                    }
                    _ => {
                        merged.insert(key, value);
                    }
                }
            }
            Value::Object(merged)
        }
        // Not both objects (`community-plugins.json` is an array): there
        // is nothing to merge into, so the typed value is the file.
        (_, fresh) => fresh,
    }
}

/// Whether a config value means what the file already says.
///
/// Numbers compare at `f32`, the narrowest a config field uses: it makes
/// Obsidian's `1` equal to serde's `1.0`, and a `centerStrength` read into an
/// `f32` and written back equal to the `f64` digits the file carries. The
/// comparison only decides whether to keep the file's bytes, so a value that
/// differs beyond `f32` is written out rather than lost.
fn same_config_value(old: &serde_json::Value, new: &serde_json::Value) -> bool {
    use serde_json::Value;
    match (old, new) {
        (Value::Number(a), Value::Number(b)) => match (a.as_f64(), b.as_f64()) {
            (Some(a), Some(b)) => a == b || a as f32 == b as f32,
            _ => a == b,
        },
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same_config_value(a, b))
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(k, v)| b.get(k).is_some_and(|other| same_config_value(v, other)))
        }
        _ => old == new,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn sample_vault() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        fs::create_dir_all(r.join(".obsidian/plugins/x")).unwrap();
        fs::create_dir_all(r.join(".trash")).unwrap();
        fs::create_dir_all(r.join("Projects")).unwrap();
        fs::write(r.join("Note.md"), "# Note\n[[Plan]]").unwrap();
        fs::write(r.join("Projects/Plan.md"), "plan").unwrap();
        fs::write(r.join("Board.canvas"), "{}").unwrap();
        fs::write(r.join("pic.PNG"), "").unwrap();
        fs::write(r.join(".hidden.md"), "").unwrap();
        fs::write(r.join(".trash/old.md"), "").unwrap();
        fs::write(r.join(".obsidian/app.json"), r#"{"vimMode":true}"#).unwrap();
        fs::write(
            r.join(".obsidian/plugins/x/manifest.json"),
            r#"{"id":"x","name":"X","version":"1.2.3","author":"me","description":"d"}"#,
        )
        .unwrap();
        fs::create_dir_all(r.join(".obsidian/plugins/broken")).unwrap();
        fs::write(
            r.join(".obsidian/command-palette.json"),
            r#"{"pinned":["switcher:open"]}"#,
        )
        .unwrap();
        fs::write(
            r.join(".obsidian/core-plugins.json"),
            r#"{"file-explorer":true,"graph":true,"canvas":false}"#,
        )
        .unwrap();
        fs::write(
            r.join(".obsidian/community-plugins.json"),
            r#"["dataview"]"#,
        )
        .unwrap();
        dir
    }

    #[test]
    fn scans_files_and_skips_dot_entries() {
        let dir = sample_vault();
        let vault = Vault::open(dir.path()).unwrap();
        let paths: Vec<&str> = vault.files().iter().map(|f| f.path.as_str()).collect();
        assert_eq!(
            paths,
            vec!["Board.canvas", "Note.md", "Projects/Plan.md", "pic.PNG"]
        );
        assert_eq!(vault.markdown_files().count(), 2);
        assert_eq!(vault.files()[3].kind, FileKind::Image);
        assert_eq!(vault.folders(), ["Projects"], "dot-folders are not listed");
    }

    #[test]
    fn reads_config_and_resolves_links() {
        let dir = sample_vault();
        let vault = Vault::open(dir.path()).unwrap();
        assert_eq!(vault.app_config().unwrap().vim_mode, Some(true));
        assert!(vault.core_plugins().unwrap().unwrap().is_enabled("graph"));
        assert_eq!(vault.community_plugins().unwrap(), vec!["dataview"]);
        assert!(vault.hotkeys().unwrap().is_empty());
        assert!(vault.workspace_json().unwrap().is_none());
        assert_eq!(
            vault.command_palette_config().unwrap().pinned,
            vec!["switcher:open"]
        );
        assert_eq!(vault.switcher_config().unwrap().show_attachments, None);
        let manifests = vault.plugin_manifests();
        assert_eq!(manifests.len(), 1);
        assert_eq!(manifests[0].name, "X");
        assert_eq!(manifests[0].version, "1.2.3");
        let r = vault.link_resolver();
        assert_eq!(r.resolve("plan", "Note.md"), Some("Projects/Plan.md"));
    }

    #[test]
    fn refresh_path_tracks_files_folders_and_removals() {
        let dir = sample_vault();
        let mut vault = Vault::open(dir.path()).unwrap();
        let r = dir.path();
        fs::write(r.join("Projects/New.md"), "x").unwrap();
        assert_eq!(
            vault.refresh_path(&r.join("Projects/New.md")),
            vec![FileChange::Added("Projects/New.md".into())]
        );
        assert_eq!(
            vault.refresh_path(&r.join("Projects/New.md")),
            vec![FileChange::Modified("Projects/New.md".into())]
        );
        fs::create_dir_all(r.join("Moved/in")).unwrap();
        fs::write(r.join("Moved/in/a.md"), "a").unwrap();
        fs::write(r.join("Moved/.hidden.md"), "a").unwrap();
        assert_eq!(
            vault.refresh_path(&r.join("Moved")),
            vec![FileChange::Added("Moved/in/a.md".into())]
        );
        fs::remove_dir_all(r.join("Projects")).unwrap();
        let mut removed = vault.refresh_path(&r.join("Projects"));
        removed.sort_by(|a, b| a.path().cmp(b.path()));
        assert_eq!(
            removed,
            vec![
                FileChange::Removed("Projects/New.md".into()),
                FileChange::Removed("Projects/Plan.md".into())
            ]
        );
        assert!(vault.refresh_path(&r.join(".obsidian/app.json")).is_empty());
        assert!(vault.refresh_path(&r.join(".trash/x.md")).is_empty());
        assert!(vault.refresh_path(Path::new("/elsewhere/x.md")).is_empty());
        let paths: Vec<&str> = vault.files().iter().map(|f| f.path.as_str()).collect();
        assert_eq!(
            paths,
            vec!["Board.canvas", "Moved/in/a.md", "Note.md", "pic.PNG"]
        );
        assert_eq!(vault.folders(), ["Moved", "Moved/in"]);
        fs::create_dir_all(r.join("Empty")).unwrap();
        assert!(vault.refresh_path(&r.join("Empty")).is_empty());
        assert_eq!(vault.folders(), ["Empty", "Moved", "Moved/in"]);
    }

    /// Obsidian's `graph.json`, as it writes it: `collapse-*` keys among the
    /// others, integers without a decimal point, and more digits of
    /// `centerStrength` than an `f32` field can hold.
    const GRAPH_JSON: &str = r#"{
  "collapse-filter": true,
  "search": "",
  "showTags": false,
  "colorGroups": [],
  "showArrow": false,
  "textFadeMultiplier": 0,
  "nodeSizeMultiplier": 1,
  "centerStrength": 0.518713248970312,
  "linkDistance": 250,
  "close": true
}"#;

    /// `templates.json` as Obsidian wrote it in the phase 3 probe: keys in
    /// the order they were first set, two-space indented, no trailing
    /// newline.
    const TEMPLATES_JSON: &str = r#"{
  "folder": "TProbeB",
  "dateFormat": "YYYY-MM-DD",
  "timeFormat": "HH:mm"
}"#;

    /// `daily-notes.json`, the same way.
    const DAILY_NOTES_JSON: &str = r#"{
  "format": "YYYY-MM-DD",
  "folder": "Journal"
}"#;

    #[test]
    fn the_plugin_configs_read_and_write_back_byte_identical() {
        let dir = sample_vault();
        fs::write(dir.path().join(".obsidian/templates.json"), TEMPLATES_JSON).unwrap();
        fs::write(
            dir.path().join(".obsidian/daily-notes.json"),
            DAILY_NOTES_JSON,
        )
        .unwrap();
        let vault = Vault::open(dir.path()).unwrap();

        let templates = vault.templates_config().unwrap();
        assert_eq!(templates.folder.as_deref(), Some("TProbeB"));
        assert_eq!(templates.date_format.as_deref(), Some("YYYY-MM-DD"));
        assert_eq!(templates.time_format.as_deref(), Some("HH:mm"));
        vault.write_config("templates.json", &templates).unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join(".obsidian/templates.json")).unwrap(),
            TEMPLATES_JSON
        );

        let daily = vault.daily_notes_config().unwrap();
        assert_eq!(daily.format.as_deref(), Some("YYYY-MM-DD"));
        assert_eq!(daily.folder.as_deref(), Some("Journal"));
        assert_eq!(daily.template, None);
        vault.write_config("daily-notes.json", &daily).unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join(".obsidian/daily-notes.json")).unwrap(),
            DAILY_NOTES_JSON
        );
    }

    #[test]
    fn a_plugin_config_that_was_never_written_is_the_default() {
        // Neither file exists until a setting is changed, and a vault
        // without one is not an error.
        let dir = sample_vault();
        let vault = Vault::open(dir.path()).unwrap();
        assert_eq!(
            vault.templates_config().unwrap(),
            TemplatesConfig::default()
        );
        assert_eq!(
            vault.daily_notes_config().unwrap(),
            DailyNotesConfig::default()
        );
    }

    #[test]
    fn a_cleared_templates_field_keeps_its_key() {
        // Obsidian writes the empty string rather than dropping the key.
        let dir = sample_vault();
        let cleared = r#"{
  "folder": "TProbeB",
  "dateFormat": "",
  "timeFormat": ""
}"#;
        fs::write(dir.path().join(".obsidian/templates.json"), cleared).unwrap();
        let vault = Vault::open(dir.path()).unwrap();
        let templates = vault.templates_config().unwrap();
        assert_eq!(templates.date_format.as_deref(), Some(""));
        vault.write_config("templates.json", &templates).unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join(".obsidian/templates.json")).unwrap(),
            cleared
        );
    }

    #[test]
    fn writing_a_config_value_nothing_changed_rewrites_nothing() {
        let dir = sample_vault();
        fs::write(dir.path().join(".obsidian/graph.json"), GRAPH_JSON).unwrap();
        let vault = Vault::open(dir.path()).unwrap();
        let graph = vault.graph_config().unwrap();
        vault.write_config("graph.json", &graph).unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join(".obsidian/graph.json")).unwrap(),
            GRAPH_JSON,
            "a value read and written back changed the file"
        );
    }

    #[test]
    fn writing_a_changed_setting_leaves_the_other_keys_alone() {
        let dir = sample_vault();
        fs::write(dir.path().join(".obsidian/graph.json"), GRAPH_JSON).unwrap();
        let vault = Vault::open(dir.path()).unwrap();
        let mut graph = vault.graph_config().unwrap();
        graph.show_tags = Some(true);
        vault.write_config("graph.json", &graph).unwrap();
        let after = fs::read_to_string(dir.path().join(".obsidian/graph.json")).unwrap();
        assert_eq!(
            after,
            GRAPH_JSON.replace("\"showTags\": false", "\"showTags\": true"),
            "changing one setting rewrote more than its line"
        );
    }

    #[test]
    fn a_key_the_model_does_not_write_is_kept() {
        let dir = sample_vault();
        // `colorGroups` is skipped when empty, and `theme` is a key no
        // graph config models at all. Both have to survive a write.
        fs::write(
            dir.path().join(".obsidian/graph.json"),
            "{\n  \"colorGroups\": [],\n  \"theme\": \"moonstone\",\n  \"scale\": 1\n}",
        )
        .unwrap();
        let vault = Vault::open(dir.path()).unwrap();
        let graph = vault.graph_config().unwrap();
        vault.write_config("graph.json", &graph).unwrap();
        let after = fs::read_to_string(dir.path().join(".obsidian/graph.json")).unwrap();
        assert!(after.contains("\"colorGroups\": []"), "{after}");
        assert!(after.contains("\"theme\": \"moonstone\""), "{after}");
    }

    #[test]
    fn a_nested_object_keeps_its_place_and_its_other_keys() {
        let dir = sample_vault();
        let path = dir.path().join(".obsidian/types.json");
        fs::write(
            &path,
            "{\n  \"first\": 1,\n  \"types\": {\n    \"due\": \"date\",\n    \"owner\": \"text\"\n  },\n  \"last\": 2\n}",
        )
        .unwrap();
        let vault = Vault::open(dir.path()).unwrap();
        let mut types = vault.property_types().unwrap();
        types.types.insert("due".into(), "datetime".into());
        vault.write_config("types.json", &types).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "{\n  \"first\": 1,\n  \"types\": {\n    \"due\": \"datetime\",\n    \"owner\": \"text\"\n  },\n  \"last\": 2\n}",
            "merging into a nested object moved keys"
        );
    }

    #[test]
    fn a_file_that_is_not_an_object_is_replaced() {
        let dir = sample_vault();
        let path = dir.path().join(".obsidian/community-plugins.json");
        fs::write(&path, "[\n  \"old\"\n]").unwrap();
        let vault = Vault::open(dir.path()).unwrap();
        let plugins: CommunityPlugins = vec!["new".to_string()];
        vault
            .write_config("community-plugins.json", &plugins)
            .unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "[\n  \"new\"\n]");
    }

    /// A config file with a byte order mark is still JSON once the mark is
    /// off. Parsing it with the mark fails, which used to mean the file was
    /// replaced wholesale and every key Silica does not model was lost.
    #[test]
    fn a_config_file_with_a_byte_order_mark_keeps_its_keys_and_its_mark() {
        let dir = sample_vault();
        let path = dir.path().join(".obsidian/app.json");
        let original =
            "\u{feff}{\n  \"trashOption\": \"local\",\n  \"somethingSilicaDoesNotModel\": 7\n}";
        fs::write(&path, original).unwrap();
        let vault = Vault::open(dir.path()).unwrap();
        let mut app = vault.app_config().unwrap();
        assert_eq!(
            app.trash_option.as_deref(),
            Some("local"),
            "the mark stopped the file being read"
        );
        app.vim_mode = Some(true);
        vault.write_config("app.json", &app).unwrap();

        let written = fs::read_to_string(&path).unwrap();
        assert!(
            written.starts_with('\u{feff}'),
            "the user's byte order mark was dropped: {written:?}"
        );
        assert!(
            written.contains("\"somethingSilicaDoesNotModel\": 7"),
            "a key Silica does not model was lost: {written}"
        );
        assert!(written.contains("\"vimMode\": true"), "{written}");
        assert!(written.contains("\"trashOption\": \"local\""), "{written}");
    }

    /// A config file nothing can parse is a file nothing may overwrite:
    /// reading it gave defaults, so writing it would put those defaults on
    /// disk in place of everything the user had. Both the merging write
    /// and the whole-file one refuse, and the bytes stay exactly as they
    /// were.
    #[test]
    fn unparsable_json_is_refused_rather_than_replaced() {
        let dir = sample_vault();
        let path = dir.path().join(".obsidian/types.json");
        let broken = "{\n  \"types\": { \"due\": \"date\", },\n}";
        fs::write(&path, broken).unwrap();
        let vault = Vault::open(dir.path()).unwrap();
        let mut types = PropertyTypes::default();
        types.types.insert("owner".into(), "text".into());

        let err = vault.write_config("types.json", &types).unwrap_err();
        assert!(matches!(err, VaultError::Unparseable(_)), "{err:?}");
        assert!(err.to_string().contains("types.json"), "{err}");
        assert_eq!(fs::read_to_string(&path).unwrap(), broken, "bytes changed");

        let err = vault.replace_config("types.json", &types).unwrap_err();
        assert!(matches!(err, VaultError::Unparseable(_)), "{err:?}");
        assert_eq!(fs::read_to_string(&path).unwrap(), broken, "bytes changed");
    }

    /// A file that is valid JSON but not the shape the type expects is a
    /// different case: nothing is lost by writing it, because the merge
    /// keeps every key it did not change.
    #[test]
    fn json_of_the_wrong_shape_is_still_merged_into() {
        let dir = sample_vault();
        let path = dir.path().join(".obsidian/types.json");
        fs::write(&path, "{\"types\": 7, \"future\": \"kept\"}").unwrap();
        let vault = Vault::open(dir.path()).unwrap();
        let mut types = PropertyTypes::default();
        types.types.insert("due".into(), "date".into());
        vault.write_config("types.json", &types).unwrap();
        let written = fs::read_to_string(&path).unwrap();
        assert!(written.contains("\"due\": \"date\""), "{written}");
        assert!(written.contains("\"future\": \"kept\""), "{written}");
    }

    #[test]
    fn rejects_non_directories() {
        let dir = sample_vault();
        assert!(matches!(
            Vault::open(dir.path().join("Note.md")),
            Err(VaultError::NotADirectory(_))
        ));
    }

    #[test]
    fn a_write_path_stays_inside_the_vault_and_off_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        fs::create_dir_all(r.join("Sub")).unwrap();
        fs::write(r.join("Sub/a.md"), "a").unwrap();
        let vault = Vault::open(r).unwrap();
        assert_eq!(
            vault.write_path("Sub/a.md").unwrap(),
            r.join("Sub").join("a.md")
        );
        assert_eq!(
            vault.write_path("New/b.md").unwrap(),
            r.join("New").join("b.md")
        );
        assert!(matches!(
            vault.write_path("../x.md"),
            Err(VaultError::OutsideVault(_))
        ));
        assert!(matches!(
            vault.write_path("Sub/../../x.md"),
            Err(VaultError::OutsideVault(_))
        ));
        assert!(matches!(
            vault.write_path("/etc/passwd"),
            Err(VaultError::OutsideVault(_))
        ));
        assert!(matches!(
            vault.write_path(""),
            Err(VaultError::OutsideVault(_))
        ));
        #[cfg(unix)]
        {
            let outside = tempfile::tempdir().unwrap();
            fs::write(outside.path().join("secret"), "s").unwrap();
            std::os::unix::fs::symlink(outside.path(), r.join("Link")).unwrap();
            std::os::unix::fs::symlink(outside.path().join("secret"), r.join("note.md")).unwrap();
            // A linked folder on the way, and a linked file at the end.
            assert!(matches!(
                vault.write_path("Link/x.md"),
                Err(VaultError::Symlink(_))
            ));
            assert!(matches!(
                vault.write_path("note.md"),
                Err(VaultError::Symlink(_))
            ));
            assert!(vault.write_bytes("note.md", b"x").is_err());
            // Nothing outside changed, and the link itself is still a link.
            assert_eq!(
                fs::read_to_string(outside.path().join("secret")).unwrap(),
                "s"
            );
            assert!(fs::symlink_metadata(r.join("note.md"))
                .unwrap()
                .file_type()
                .is_symlink());
            // A link may be moved or removed as an entry, not written through.
            assert!(vault.entry_path("note.md").is_ok());
            assert!(matches!(
                vault.entry_path("Link/x.md"),
                Err(VaultError::Symlink(_))
            ));
            // The open itself refuses a link that appears after the check.
            assert!(write_no_follow(&r.join("note.md"), b"x").is_err());
        }
        vault.write_bytes("Sub/a.md", b"b").unwrap();
        assert_eq!(fs::read_to_string(r.join("Sub/a.md")).unwrap(), "b");
    }
}
