//! File operations on the vault folder: new notes and folders, rename and
//! move, and deletion to the trash. Each operation changes the disk first and
//! then the file list, so the result is what a filesystem event would have
//! produced; link rewriting lives in `silica-cache` and is driven by the app.
//!
//! Behaviors observed in Obsidian 1.14.4: a new note is `Untitled.md`, then
//! `Untitled 1.md`, `Untitled 2.md`; a new folder `Untitled`, `Untitled 1`
//! (files and folders count separately); creating over an existing path
//! fails with "File already exists."; renaming onto an existing path fails
//! with "Destination file already exists!" while a case-only rename of the
//! same file succeeds; a note created from a link goes to `newFileLocation`
//! (`root`, `current`, or the `newFileFolderPath` folder, falling back to
//! the root when that folder does not exist), with subfolders created for a
//! `folder/Name` link; a file trashed locally lands flat in `.trash/`, a
//! folder with its contents, and a name already there gets ` 2`, ` 3`, ...
//! A new attachment goes where `attachmentFolderPath` says: a leading `./`
//! is relative to the note's own folder, anything else is vault-absolute,
//! and `/` and `""` both mean the root ([`attachment_path`], observed on
//! 1.14.5).

use crate::{create_dir_all_at, FileChange, Vault};
use silica_formats::validate_file_name;
use std::path::{Path, PathBuf};

/// `trashOption` in `app.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrashOption {
    /// The operating system's trash (Obsidian's default).
    #[default]
    System,
    /// The vault's own `.trash` folder.
    Local,
    /// Delete permanently.
    None,
}

impl TrashOption {
    pub fn from_config(value: Option<&str>) -> TrashOption {
        match value {
            Some("local") => TrashOption::Local,
            Some("none") => TrashOption::None,
            _ => TrashOption::System,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum OpError {
    #[error("File already exists.")]
    Exists,
    #[error("Destination file already exists!")]
    DestinationExists,
    #[error("{0}")]
    InvalidName(&'static str),
    #[error("{0} does not exist")]
    NotFound(String),
    #[error("{action} {path}: {source}")]
    Io {
        action: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// A path the vault refuses to write: outside the folder, or through a
    /// symbolic link.
    #[error("{0}")]
    Refused(#[from] crate::VaultError),
}

impl OpError {
    /// The sentence a dialog shows. Everything but an I/O failure already
    /// reads as one; an I/O failure's `Display` carries the absolute path
    /// and the errno, which belong in a log and not in a box the user
    /// typed a file name into.
    pub fn user_message(&self) -> String {
        match self {
            OpError::Io {
                action,
                path,
                source,
            } => {
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("that file");
                format!("Could not {action} {name}: {}.", why(source))
            }
            other => other.to_string(),
        }
    }
}

/// Why an I/O call failed, in words, with no errno in them.
fn why(err: &std::io::Error) -> &'static str {
    match err.kind() {
        std::io::ErrorKind::PermissionDenied => "permission was denied",
        std::io::ErrorKind::NotFound => "it is no longer there",
        std::io::ErrorKind::AlreadyExists => "something is already there",
        std::io::ErrorKind::InvalidInput => "the name is not valid",
        _ => "the file system refused",
    }
}

pub fn io_error(action: &'static str, path: PathBuf, source: std::io::Error) -> OpError {
    OpError::Io {
        action,
        path,
        source,
    }
}

pub fn io<'p>(action: &'static str, path: &'p Path) -> impl FnOnce(std::io::Error) -> OpError + 'p {
    move |source| OpError::Io {
        action,
        path: path.to_path_buf(),
        source,
    }
}

/// The folder part of a vault path (`""` at the root).
pub fn folder_of(path: &str) -> &str {
    match path.rfind('/') {
        Some(i) => &path[..i],
        None => "",
    }
}

fn join(folder: &str, name: &str) -> String {
    if folder.is_empty() {
        name.to_string()
    } else {
        format!("{folder}/{name}")
    }
}

/// The folder a new attachment goes into: what `attachmentFolderPath` names
/// for a note at `note`, as a vault-relative path (`""` at the root).
///
/// A leading `./` is relative to the note's own folder, anything else is a
/// vault-absolute path, and `/` and `""` both mean the vault root — observed
/// of `fileManager.getAvailablePathForAttachment` for a root note and one in
/// `Sub/` (Obsidian 1.14.5, the 2026-10-25 `ui-rules` probe; see
/// `Specification#The attachment-location dropdown`). A folder that does not
/// exist is not a fallback: a paste creates the whole chain, which the same
/// probe saw for `att/deep/new`.
///
/// One observed case is deliberately left out, because it needs the file
/// list rather than the two strings: a setting naming an existing **file**
/// resolves to the vault root.
pub fn attachment_folder(setting: Option<&str>, note: &str) -> String {
    let setting = setting.unwrap_or("");
    match setting.strip_prefix("./") {
        Some(under) => {
            let under = under.trim_matches('/');
            let folder = folder_of(note);
            if under.is_empty() {
                folder.to_string()
            } else {
                join(folder, under)
            }
        }
        None => setting.trim_matches('/').to_string(),
    }
}

/// The path the attachment `name` lands at, for a note at `note` under
/// `attachmentFolderPath`.
pub fn attachment_path(setting: Option<&str>, note: &str, name: &str) -> String {
    join(&attachment_folder(setting, note), name)
}

/// What one backup run produced.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BackupReport {
    /// Files copied into the new backup folder.
    pub files: usize,
    /// Bytes those files hold.
    pub bytes: u64,
    /// The backup folder that was written.
    pub dir: std::path::PathBuf,
}

/// Copies the whole vault into a new folder under `backups` and prunes all
/// but the newest `keep`.
///
/// Obsidian never backs a vault up -- that is Sync's job and Sync is paid --
/// so this is Silica's own answer to the most common request the app gets: a
/// local copy that outlives a bad edit. The one rule it keeps is that a
/// backup must never eat the vault it is a copy of: `.git` and `.silica` are
/// skipped (the first is the user's own history, the second is where the
/// backups themselves live), `backups` itself is skipped wherever it sits,
/// and `keep` is at least one so a run cannot delete the only copy that
/// exists.
pub fn backup_vault(root: &Path, backups: &Path, keep: usize) -> Result<BackupReport, OpError> {
    let dest = unique_backup_dir(backups)?;
    std::fs::create_dir_all(&dest).map_err(io("create folder", &dest))?;
    let mut report = BackupReport {
        dir: dest.clone(),
        ..Default::default()
    };
    copy_backup_tree(root, root, &dest, backups, &mut report)?;
    // * Mark the backup complete only after every file copied: a torn run
    //   (crash, power loss, drive unmounted mid-copy) leaves no marker, so
    //   prune can tell a useless half-copy from a good one.
    std::fs::File::create(dest.join(".complete")).map_err(io("create completion marker", &dest))?;
    // fsync the directory so the marker and its entries are durable.
    // * Windows cannot File::open a directory at all (a directory handle
    //   needs FILE_FLAG_BACKUP_SEMANTICS), which is exactly the
    //   PermissionDenied the CI backup tests hit. There the marker file is
    //   what prune and restore read, so syncing it is the guarantee that
    //   matters; NTFS journals the namespace change itself.
    #[cfg(windows)]
    {
        let marker = std::fs::File::open(dest.join(".complete"))
            .map_err(io("open completion marker", &dest))?;
        marker
            .sync_all()
            .map_err(io("sync completion marker", &dest))?;
    }
    #[cfg(not(windows))]
    {
        let dir = std::fs::File::open(&dest).map_err(io("open backup dir", &dest))?;
        dir.sync_all().map_err(io("sync backup dir", &dest))?;
    }
    let keep = keep.max(1);
    prune_backups(backups, keep);
    Ok(report)
}

/// The next `vault-<epoch seconds>` folder inside `backups`, never one that
/// already exists: two runs in the same second get `-2`, `-3`, ... which is
/// what keeps the sort the prune does chronological. Unix time rather than a
/// date because the vault crate has no clock-to-civil conversion of its own,
/// and borrowing the templates crate's for a folder name is not worth the
/// dependency.
fn unique_backup_dir(backups: &Path) -> Result<PathBuf, OpError> {
    std::fs::create_dir_all(backups).map_err(io("create folder", backups))?;
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let base = format!("vault-{secs}");
    let mut n = 1;
    loop {
        let name = if n == 1 {
            base.clone()
        } else {
            format!("{base}-{n}")
        };
        let candidate = backups.join(name);
        if !candidate.exists() {
            return Ok(candidate);
        }
        n += 1;
    }
}

/// Copies everything under `from` into `to`, keeping the folder structure and
/// skipping what a backup must not hold.
fn copy_backup_tree(
    root: &Path,
    from: &Path,
    to: &Path,
    backups: &Path,
    report: &mut BackupReport,
) -> Result<(), OpError> {
    let entries = match std::fs::read_dir(from) {
        Ok(entries) => entries,
        // ! A vault that loses a folder between the walk and the read is not
        //   a failed backup: the rest of it is still worth having.
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(io("read folder", from)(err)),
    };
    let mut entries: Vec<_> = entries.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let rel = match path.strip_prefix(root) {
            Ok(rel) => rel.to_path_buf(),
            Err(_) => continue,
        };
        if path.starts_with(backups) {
            continue;
        }
        // * Silica's own scratch space and the user's own history: both are
        //   already copies of something.
        if matches!(
            rel.components().next().and_then(|c| c.as_os_str().to_str()),
            Some(".git") | Some(".silica")
        ) {
            continue;
        }
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(_) => continue,
        };
        // ! The destination is the mirror of this level, not of the root: the
        //   walk carries the root-relative path, and joining that onto the
        //   level's own folder would nest `Sub/Sub`.
        let local = match path.strip_prefix(from) {
            Ok(local) => local.to_path_buf(),
            Err(_) => continue,
        };
        let dest = to.join(&local);
        if file_type.is_dir() {
            std::fs::create_dir_all(&dest).map_err(io("create folder", &dest))?;
            copy_backup_tree(root, &path, &dest, backups, report)?;
        } else if file_type.is_file() {
            let bytes = std::fs::copy(&path, &dest).map_err(io("copy file", &path))?;
            report.files += 1;
            report.bytes += bytes;
        }
        // ! A symlink is left alone: following one could copy a file the
        //   vault only borrows, and the app's write path refuses them anyway.
    }
    Ok(())
}

/// Deletes every incomplete backup (torn: no completion marker) and then all
/// but the newest `keep` complete backups, by name: the timestamp prefix is
/// fixed width, so the sort is chronological. An incomplete backup is worse
/// than no backup — it promises safety a half-copied vault cannot deliver —
/// so incomplete folders are always removed, never counted toward `keep`.
fn prune_backups(backups: &Path, keep: usize) {
    let Ok(entries) = std::fs::read_dir(backups) else {
        return;
    };
    let mut complete: Vec<PathBuf> = Vec::new();
    let mut incomplete: Vec<PathBuf> = Vec::new();
    for e in entries.filter_map(|e| e.ok()).map(|e| e.path()) {
        if !e.is_dir() {
            continue;
        }
        let Some(name) = e.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.starts_with("vault-") {
            continue;
        }
        if is_complete(&e) {
            complete.push(e);
        } else {
            incomplete.push(e);
        }
    }
    // * A torn backup (crash mid-copy, no completion marker) is deleted first
    //   and never counts toward `keep`.
    for stale in incomplete {
        let _ = std::fs::remove_dir_all(&stale);
    }
    complete.sort();
    if complete.len() <= keep {
        return;
    }
    for stale in &complete[..complete.len() - keep] {
        let _ = std::fs::remove_dir_all(stale);
    }
}

/// True once `backup_vault` has finished copying every file and written its
/// completion marker; a folder without it is a torn run and is not trusted.
fn is_complete(dir: &Path) -> bool {
    dir.join(".complete").is_file()
}

/// Checks every segment of a new path the way Obsidian's rename does.
fn validate_path(path: &str) -> Result<(), OpError> {
    if path.is_empty() {
        return Err(OpError::InvalidName("File name cannot be empty."));
    }
    for segment in path.split('/') {
        validate_file_name(segment).map_err(OpError::InvalidName)?;
    }
    Ok(())
}

impl Vault {
    /// Whether anything exists at this vault path on disk.
    pub fn exists(&self, rel: &str) -> bool {
        !rel.is_empty() && self.absolute(rel).exists()
    }

    /// The first free name among `stem`, `stem 1`, `stem 2`, ... in `folder`,
    /// with `.ext` appended when `ext` is not empty (Obsidian's
    /// `getAvailablePath`).
    pub fn available_path(&self, folder: &str, stem: &str, ext: &str) -> String {
        self.numbered_path(folder, stem, ext, 1)
    }

    fn numbered_path(&self, folder: &str, stem: &str, ext: &str, first: usize) -> String {
        let with_ext = |name: &str| {
            if ext.is_empty() {
                join(folder, name)
            } else {
                join(folder, &format!("{name}.{ext}"))
            }
        };
        let plain = with_ext(stem);
        if !self.exists(&plain) {
            return plain;
        }
        let mut n = first;
        loop {
            let candidate = with_ext(&format!("{stem} {n}"));
            if !self.exists(&candidate) {
                return candidate;
            }
            n += 1;
        }
    }

    /// The folder a new note goes into: `newFileLocation` is `root`,
    /// `current` (the open note's folder) or `folder` (`newFileFolderPath`,
    /// falling back to the root when that folder is missing).
    pub fn new_note_folder(
        &self,
        location: Option<&str>,
        folder_path: Option<&str>,
        current: Option<&str>,
    ) -> String {
        match location {
            Some("current") => current.map(folder_of).unwrap_or("").to_string(),
            Some("folder") => {
                let folder = folder_path
                    .unwrap_or("")
                    .trim_matches('/')
                    .replace('\\', "/");
                if !folder.is_empty() && self.absolute(&folder).is_dir() {
                    folder
                } else {
                    String::new()
                }
            }
            _ => String::new(),
        }
    }

    /// The path a note created for the link text `name` (without `.md`) gets:
    /// a name with a folder in it is taken from the vault root, anything
    /// else goes into the new-note folder.
    pub fn new_note_path(&self, name: &str, new_note_folder: &str) -> String {
        let name = name.trim().trim_matches('/');
        let name = name.strip_suffix(".md").unwrap_or(name);
        if name.contains('/') {
            format!("{name}.md")
        } else {
            join(new_note_folder, &format!("{name}.md"))
        }
    }

    /// Creates a markdown note (and its folders) with `text`. Fails when the
    /// path exists.
    pub fn create_note(&mut self, path: &str, text: &str) -> Result<Vec<FileChange>, OpError> {
        validate_path(path)?;
        if self.exists(path) {
            return Err(OpError::Exists);
        }
        let abs = self.write_path(path)?;
        self.create_parent_dirs_secure(&abs)?;
        crate::write_no_follow(&abs, text.as_bytes()).map_err(io("create", &abs))?;
        Ok(self.refresh_path(&abs))
    }

    /// Saves an attachment file with `bytes`, creating parent directories as needed.
    pub fn save_attachment(
        &mut self,
        path: &str,
        bytes: &[u8],
    ) -> Result<Vec<FileChange>, OpError> {
        validate_path(path)?;
        let abs = self.write_path(path)?;
        self.create_parent_dirs_secure(&abs)?;
        crate::write_no_follow(&abs, bytes).map_err(io("create attachment", &abs))?;
        Ok(self.refresh_path(&abs))
    }

    /// Creates an empty folder (and its parents).
    pub fn create_folder(&mut self, path: &str) -> Result<(), OpError> {
        validate_path(path)?;
        if self.exists(path) {
            return Err(OpError::Exists);
        }
        let abs = self.write_path(path)?;
        self.create_parent_dirs_secure(&abs)?;
        std::fs::create_dir_all(&abs).map_err(io("create folder", &abs))?;
        self.refresh_path(&abs);
        Ok(())
    }

    /// `from` as the index spells it. Windows paths are case-insensitive,
    /// so a caller typing `sub` for the stored `Sub` would plan nothing
    /// and leave every link in the moved folder unrewritten.
    fn stored_from(&self, from: &str) -> Option<String> {
        let wanted: Vec<&str> = from.split('/').filter(|s| !s.is_empty()).collect();
        if wanted.is_empty() {
            return None;
        }
        self.files().iter().find_map(|f| {
            let got: Vec<&str> = f.path.split('/').filter(|s| !s.is_empty()).collect();
            let spelled_same = got.len() >= wanted.len()
                && wanted
                    .iter()
                    .zip(got.iter())
                    .all(|(w, g)| w.to_lowercase() == g.to_lowercase());
            spelled_same.then(|| got[..wanted.len()].join("/"))
        })
    }

    /// Files under `path` when it is a folder, or `path` itself when it is
    /// a file in the list.
    fn files_under(&self, path: &str) -> Vec<String> {
        let prefix = format!("{path}/");
        self.files()
            .iter()
            .filter(|f| f.path == path || f.path.starts_with(&prefix))
            .map(|f| f.path.clone())
            .collect()
    }

    /// The files `rename(from, to)` would move, as `(old path, new path)`,
    /// without touching anything.
    pub fn rename_plan(&self, from: &str, to: &str) -> Vec<(String, String)> {
        let from = from.trim_matches('/');
        let to = to.trim_matches('/');
        let stored = self.stored_from(from);
        let from = stored.as_deref().unwrap_or(from);
        self.files_under(from)
            .into_iter()
            .map(|old| {
                let new = if old == from {
                    to.to_string()
                } else {
                    format!("{to}{}", &old[from.len()..])
                };
                (old, new)
            })
            .collect()
    }

    /// Renames or moves a file or folder. Returns every file that moved as
    /// `(old path, new path)`, so links can be rewritten.
    pub fn rename(&mut self, from: &str, to: &str) -> Result<Vec<(String, String)>, OpError> {
        let from = from.trim_matches('/');
        let to = to.trim_matches('/');
        validate_path(to)?;
        let from_abs = self.entry_path(from)?;
        if !from_abs.exists() {
            return Err(OpError::NotFound(from.to_string()));
        }
        if from == to {
            return Ok(Vec::new());
        }
        let to_abs = self.entry_path(to)?;
        // * A case-only rename of the same entry is allowed; anything else
        // * already at the destination is not.
        let same_entry = from.to_lowercase() == to.to_lowercase()
            && std::fs::canonicalize(&from_abs).ok() == std::fs::canonicalize(&to_abs).ok();
        if to_abs.exists() && !same_entry {
            return Err(OpError::DestinationExists);
        }
        if to.starts_with(&format!("{from}/")) {
            return Err(OpError::InvalidName("Cannot move a folder into itself."));
        }
        if let Some(parent) = Path::new(to).parent() {
            create_dir_all_at(&self.root, parent)
                .map_err(io("create folder", &self.root.join(parent)))?;
        }
        let moved = self.rename_plan(from, to);
        std::fs::rename(&from_abs, &to_abs).map_err(io("rename", &from_abs))?;
        // * The old prefix is gone from disk whatever the filesystem says
        // * about its spelling, so drop it by name instead of refreshing the
        // * old path: on a case-insensitive filesystem that path still
        // * resolves to the renamed entry, and refreshing it would report a
        // * file that is merely spelled differently rather than a removal.
        let old_prefix = format!("{from}/");
        self.files
            .retain(|f| f.path != from && !f.path.starts_with(&old_prefix));
        self.folders
            .retain(|f| *f != from && !f.starts_with(&old_prefix));
        self.refresh_path(&to_abs);
        Ok(moved)
    }

    /// Deletes a file or folder per `option`. Returns the vault paths of the
    /// files that are gone.
    pub fn trash(&mut self, path: &str, option: TrashOption) -> Result<Vec<String>, OpError> {
        let path = path.trim_matches('/');
        let abs = self.entry_path(path)?;
        if !abs.exists() {
            return Err(OpError::NotFound(path.to_string()));
        }
        let removed = self.files_under(path);
        match option {
            TrashOption::None => {
                if abs.is_dir() {
                    std::fs::remove_dir_all(&abs).map_err(io("delete", &abs))?;
                } else {
                    std::fs::remove_file(&abs).map_err(io("delete", &abs))?;
                }
            }
            TrashOption::Local => self.trash_locally(path)?,
            TrashOption::System => {
                if let Err(err) = system_trash(&abs) {
                    // * No system trash here (another platform, or a read-only
                    // * data directory): the vault's own trash is the fallback.
                    if err.kind() == std::io::ErrorKind::Unsupported {
                        self.trash_locally(path)?;
                    } else {
                        return Err(OpError::Io {
                            action: "move to the system trash",
                            path: abs,
                            source: err,
                        });
                    }
                }
            }
        }
        self.refresh_path(&abs);
        Ok(removed)
    }

    fn trash_locally(&self, path: &str) -> Result<(), OpError> {
        let abs = self.entry_path(path)?;
        let trash = self.write_path(".trash")?;
        std::fs::create_dir_all(&trash).map_err(io("create folder", &trash))?;
        let name = path.rsplit('/').next().unwrap_or(path);
        let (stem, ext) = match name.rsplit_once('.') {
            Some((s, e)) if !s.is_empty() && abs.is_file() => (s, e),
            _ => (name, ""),
        };
        let target = self.numbered_path(".trash", stem, ext, 2);
        let target_abs = self.entry_path(&target)?;
        // * Write the origin sidecar before the move: a crash between the
        //   two leaves at most an orphan sidecar (harmless), never a
        //   trashed note with no record of where it came from.
        if let Some((folder, _)) = path.rsplit_once('/') {
            let _ = std::fs::write(trash.join(format!(".{name}.origin")), folder);
        }
        std::fs::rename(&abs, &target_abs).map_err(io("move to .trash", &abs))?;
        Ok(())
    }

    /// Lists the contents of the vault's own `.trash/` folder, returning
    /// vault-relative paths of every trashed entry.
    pub fn list_trash(&self) -> Result<Vec<String>, OpError> {
        let trash = self.write_path(".trash")?;
        if !trash.exists() {
            return Ok(Vec::new());
        }
        let mut out: Vec<String> = std::fs::read_dir(&trash)
            .map_err(io("read trash", &trash))?
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().to_str().map(|s| s.to_string()))
            // * Dot-prefixed entries are the vault's own records, not the
            // * user's trashed notes.
            .filter(|n| !n.starts_with('.'))
            .collect();
        out.sort();
        out.dedup();
        Ok(out)
    }

    /// Restores a trashed entry back to its original vault path.
    /// `name` is the entry's file name as returned by [`list_trash`].
    /// Brings a note back from `.trash` under its own name. The trashed copy
    /// is moved, not read-and-copied, so there is never a moment where two
    /// full copies of the user's data exist; and a name the vault already
    /// holds is refused, so a restore can never trade a newer note for the
    /// older one. The index is refreshed for the restored path — a note that
    /// is on disk but not in the file list is invisible to search, links and
    /// every list in the app, which would look exactly like a lost note.
    /// Where a trashed entry came from, when it did not come from the vault
    /// root. The vault's own `.trash` is flat, so the folder the entry lived
    /// in is the one thing the move itself loses — the freedesktop route
    /// keeps it in its `.trashinfo`, and this is the same job for the
    /// fallback: a sidecar, dot-prefixed so it never looks like a user's
    /// note and is skipped by [`Vault::list_trash`].
    fn origin_of(&self, trash: &Path, name: &str) -> Option<String> {
        let record = trash.join(format!(".{name}.origin"));
        std::fs::read_to_string(&record)
            .ok()
            .filter(|f| !f.is_empty())
    }

    pub fn restore_from_trash(&mut self, name: &str) -> Result<(), OpError> {
        let trash = self.write_path(".trash")?;
        let src = trash.join(name);
        if !src.exists() {
            return Err(OpError::NotFound(name.to_string()));
        }
        // * Back to the folder it was taken from, not to the vault root: the
        // * name in `.trash` alone cannot say where that was.
        let where_to = match self.origin_of(&trash, name) {
            Some(folder) => format!("{folder}/{name}"),
            None => name.to_string(),
        };
        let dst = self.entry_path(&where_to)?;
        if dst.exists() {
            return Err(OpError::DestinationExists);
        }
        self.create_parent_dirs_secure(&dst)?;
        std::fs::rename(&src, &dst).map_err(io("restore from trash", &src))?;
        // * Its record has said everything it has to say; a stale one for a
        // * name that no longer exists is never read, so this is only tidy-up.
        let _ = std::fs::remove_file(trash.join(format!(".{name}.origin")));
        self.refresh_path(&dst);
        Ok(())
    }
}

/// Moves `abs` to the freedesktop.org trash (`$XDG_DATA_HOME/Trash`), writing
/// the `.trashinfo` record the file manager needs to restore it.
#[cfg(all(unix, not(target_os = "macos")))]
fn system_trash(abs: &Path) -> std::io::Result<()> {
    use std::io::{Error, ErrorKind};
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .ok_or_else(|| Error::new(ErrorKind::Unsupported, "no home directory"))?;
    let trash = data_home.join("Trash");
    let files = trash.join("files");
    let info = trash.join("info");
    std::fs::create_dir_all(&files)?;
    std::fs::create_dir_all(&info)?;
    let name = abs
        .file_name()
        .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "no file name"))?
        .to_string_lossy()
        .into_owned();
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() && abs.is_file() => (s.to_string(), format!(".{e}")),
        _ => (name.clone(), String::new()),
    };
    let mut candidate = name.clone();
    let mut n = 2;
    while files.join(&candidate).exists() || info.join(format!("{candidate}.trashinfo")).exists() {
        candidate = format!("{stem} {n}{ext}");
        n += 1;
    }
    let record = format!(
        "[Trash Info]\nPath={}\nDeletionDate={}\n",
        url_encode_path(&abs.to_string_lossy()),
        utc_timestamp()
    );
    std::fs::write(info.join(format!("{candidate}.trashinfo")), record)?;
    let dest = files.join(&candidate);
    match std::fs::rename(abs, &dest) {
        Ok(()) => Ok(()),
        Err(err) if err.raw_os_error() == Some(libc_exdev()) => {
            copy_tree(abs, &dest)?;
            if abs.is_dir() {
                std::fs::remove_dir_all(abs)
            } else {
                std::fs::remove_file(abs)
            }
        }
        Err(err) => {
            let _ = std::fs::remove_file(info.join(format!("{candidate}.trashinfo")));
            Err(err)
        }
    }
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
fn system_trash(_abs: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "system trash is not implemented on this platform",
    ))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn libc_exdev() -> i32 {
    18
}

#[cfg(all(unix, not(target_os = "macos")))]
fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    let meta = match from.symlink_metadata() {
        Ok(meta) => meta,
        // The source vanished while the copy was in flight: nothing to copy.
        Err(_) => return Ok(()),
    };
    // * A symlink is re-created as a symlink, never followed: this keeps the
    // * copy inside the vault and cannot loop, even if the link points at
    // * itself or an ancestor directory.
    if meta.file_type().is_symlink() {
        let target = std::fs::read_link(from)?;
        use std::os::unix::fs::symlink;
        symlink(&target, to)?;
        return Ok(());
    }
    if meta.is_dir() {
        // copy_tree destination is an internal copy target; create parents
        // securely so no symlink component can redirect the copy.
        if let Some(parent) = to.parent() {
            // copy_tree destination is an internal copy target; create parents
            // securely so no symlink component can redirect the copy.
            std::fs::create_dir_all(parent).map_err(|e| {
                std::io::Error::other(format!("create folder {}: {e}", parent.display()))
            })?;
        }
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            copy_tree(&entry.path(), &to.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(from, to).map(|_| ())
    }
}

/// Percent-encodes a path for a `.trashinfo` record, keeping `/` and the
/// unreserved characters.
#[cfg(all(unix, not(target_os = "macos")))]
fn url_encode_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for b in path.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// `YYYY-MM-DDThh:mm:ss` for now, in UTC.
#[cfg(all(unix, not(target_os = "macos")))]
fn utc_timestamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // * Civil-from-days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn vault() -> (tempfile::TempDir, Vault) {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        fs::create_dir_all(r.join("Sub/Deep")).unwrap();
        fs::create_dir_all(r.join("Inbox")).unwrap();
        fs::write(r.join("A.md"), "a").unwrap();
        fs::write(r.join("Untitled.md"), "").unwrap();
        fs::write(r.join("Sub/S.md"), "s").unwrap();
        fs::write(r.join("Sub/Deep/D.md"), "d").unwrap();
        fs::write(r.join("Sub/pic.png"), "p").unwrap();
        let vault = Vault::open(r).unwrap();
        (dir, vault)
    }

    fn paths(v: &Vault) -> Vec<&str> {
        v.files().iter().map(|f| f.path.as_str()).collect()
    }

    #[test]
    fn new_notes_and_folders_take_the_next_free_number() {
        let (_dir, mut v) = vault();
        assert_eq!(v.available_path("", "Untitled", "md"), "Untitled 1.md");
        v.create_note("Untitled 1.md", "").unwrap();
        assert_eq!(v.available_path("", "Untitled", "md"), "Untitled 2.md");
        assert_eq!(v.available_path("", "Untitled", ""), "Untitled");
        v.create_folder("Untitled").unwrap();
        assert_eq!(v.available_path("", "Untitled", ""), "Untitled 1");
        assert_eq!(v.available_path("Sub", "S", "md"), "Sub/S 1.md");
        assert!(matches!(v.create_note("A.md", "x"), Err(OpError::Exists)));
        assert!(matches!(v.create_folder("Sub"), Err(OpError::Exists)));
        assert!(matches!(
            v.create_note("bad:name.md", ""),
            Err(OpError::InvalidName(_))
        ));
        let changes = v.create_note("New/Nested/N.md", "n").unwrap();
        assert_eq!(changes, vec![FileChange::Added("New/Nested/N.md".into())]);
        assert!(paths(&v).contains(&"New/Nested/N.md"));
        assert!(v.folders().contains(&"Untitled".to_string()));
        assert!(v.folders().contains(&"New/Nested".to_string()));
    }

    #[test]
    fn new_note_location_follows_the_setting() {
        let (_dir, v) = vault();
        assert_eq!(v.new_note_folder(None, None, Some("Sub/S.md")), "");
        assert_eq!(
            v.new_note_folder(Some("current"), None, Some("Sub/S.md")),
            "Sub"
        );
        assert_eq!(v.new_note_folder(Some("current"), None, None), "");
        assert_eq!(
            v.new_note_folder(Some("folder"), Some("Inbox"), None),
            "Inbox"
        );
        assert_eq!(v.new_note_folder(Some("folder"), Some("/"), None), "");
        assert_eq!(
            v.new_note_folder(Some("folder"), Some("Missing/Folder"), None),
            "",
            "a missing folder falls back to the root"
        );
        assert_eq!(v.new_note_path("Missing", "Inbox"), "Inbox/Missing.md");
        assert_eq!(
            v.new_note_path("Deep/Also Missing", "Inbox"),
            "Deep/Also Missing.md"
        );
        assert_eq!(v.new_note_path("Note.md", ""), "Note.md");
    }

    /// Every row of the spec's resolution table, asked of a note at the
    /// root and of `Sub/Nested.md`, which is how the probe asked Obsidian.
    #[test]
    fn an_attachment_lands_where_the_setting_says() {
        let table = [
            ("", "photo.png", "photo.png"),
            ("/", "photo.png", "photo.png"),
            ("./", "photo.png", "Sub/photo.png"),
            (
                "./Attachments",
                "Attachments/photo.png",
                "Sub/Attachments/photo.png",
            ),
            (
                "Attachments",
                "Attachments/photo.png",
                "Attachments/photo.png",
            ),
            (
                "att/deep/new",
                "att/deep/new/photo.png",
                "att/deep/new/photo.png",
            ),
        ];
        for (setting, from_root, from_subfolder) in table {
            assert_eq!(
                attachment_path(Some(setting), "Target.md", "photo.png"),
                from_root,
                "{setting:?} from a root note"
            );
            assert_eq!(
                attachment_path(Some(setting), "Sub/Nested.md", "photo.png"),
                from_subfolder,
                "{setting:?} from Sub/Nested.md"
            );
        }
    }

    /// The key is absent until something sets it, and then it is the root,
    /// as `""` is. The folder on its own is what a caller creating the
    /// chain needs.
    #[test]
    fn an_absent_attachment_folder_is_the_vault_root() {
        assert_eq!(
            attachment_path(None, "Sub/Nested.md", "photo.png"),
            "photo.png"
        );
        assert_eq!(attachment_folder(None, "Sub/Nested.md"), "");
        assert_eq!(
            attachment_folder(Some("./"), "Sub/Deep/Note.md"),
            "Sub/Deep"
        );
        assert_eq!(
            attachment_folder(Some("./Attachments/"), "Sub/Nested.md"),
            "Sub/Attachments"
        );
        assert_eq!(
            attachment_folder(Some("/Attachments/"), "Note.md"),
            "Attachments"
        );
    }

    #[test]
    fn rename_moves_files_and_folders_and_reports_every_file() {
        let (dir, mut v) = vault();
        assert_eq!(
            v.rename("A.md", "Renamed A.md").unwrap(),
            vec![("A.md".to_string(), "Renamed A.md".to_string())]
        );
        assert!(dir.path().join("Renamed A.md").is_file());
        assert!(!paths(&v).contains(&"A.md"));
        assert!(paths(&v).contains(&"Renamed A.md"));
        let moved = v.rename("Sub", "Other/Sub2").unwrap();
        assert_eq!(
            moved,
            vec![
                (
                    "Sub/Deep/D.md".to_string(),
                    "Other/Sub2/Deep/D.md".to_string()
                ),
                ("Sub/S.md".to_string(), "Other/Sub2/S.md".to_string()),
                ("Sub/pic.png".to_string(), "Other/Sub2/pic.png".to_string()),
            ]
        );
        assert_eq!(
            paths(&v),
            vec![
                "Other/Sub2/Deep/D.md",
                "Other/Sub2/S.md",
                "Other/Sub2/pic.png",
                "Renamed A.md",
                "Untitled.md"
            ]
        );
        assert_eq!(
            v.folders(),
            ["Inbox", "Other", "Other/Sub2", "Other/Sub2/Deep"]
        );
        assert!(matches!(
            v.rename("Untitled.md", "Renamed A.md"),
            Err(OpError::DestinationExists)
        ));
        assert!(matches!(
            v.rename("Other", "Other/Sub2/X"),
            Err(OpError::InvalidName(_))
        ));
        assert!(matches!(
            v.rename("Nope.md", "X.md"),
            Err(OpError::NotFound(_))
        ));
        // * A case-only rename keeps the same file.
        v.rename("Renamed A.md", "renamed a.md").unwrap();
        assert!(paths(&v).contains(&"renamed a.md"));
        assert!(v.rename("renamed a.md", "renamed a.md").unwrap().is_empty());
    }

    /// Menghapus folder yang berisi file yang sedang dibuka proses lain
    /// (tidak ada sharing) harus gagal bersih: ada yang terhapus, ada yang
    /// tersisa, dan errornya menyatakan alamatnya. Tidak boleh panic, dan
    /// tidak boleh dianggap berhasil karena sebagian file sudah pergi.
    #[cfg(windows)]
    #[test]
    fn trash_refuses_a_folder_holding_a_locked_file() {
        use std::fs::OpenOptions;
        use std::os::windows::fs::OpenOptionsExt;
        let (dir, mut v) = vault();
        let root = dir.path();
        fs::create_dir_all(root.join("Locked")).unwrap();
        fs::write(root.join("Locked/note.md"), "inside\n").unwrap();
        // * share_mode(0): proses lain memegang catatan, tanpa sharing sama sekali.
        let locked = OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(root.join("Locked/note.md"));
        let locked = match locked {
            Ok(h) => h,
            Err(_) => return,
            // * CI tanpa hak buka file eksklusif: tidak ada yang diuji.
        };
        let err = v.trash("Locked", TrashOption::Local).unwrap_err();
        assert!(
            matches!(err, OpError::Io { .. }),
            "trash folder with locked file must return an io error: {err}"
        );
        // * Folder masih ada dan belum disentang, meski catatan dikunci:
        // * tidak ada penghapusan parsial yang diam-diam.
        assert!(root.join("Locked/note.md").is_file());
        drop(locked);
        assert_eq!(
            fs::read_to_string(root.join("Locked/note.md")).unwrap(),
            "inside\n"
        );
    }

    /// A folder typed with the wrong case still reports every move, so the
    /// caller rewrites links instead of silently dropping them (F2).
    #[test]
    fn rename_plan_maps_typed_case_onto_the_stored_spelling() {
        let (_dir, v) = vault();
        assert_eq!(
            v.rename_plan("sub", "Other"),
            vec![
                ("Sub/Deep/D.md".to_string(), "Other/Deep/D.md".to_string()),
                ("Sub/S.md".to_string(), "Other/S.md".to_string()),
                ("Sub/pic.png".to_string(), "Other/pic.png".to_string()),
            ]
        );
        assert_eq!(
            v.rename_plan("untitled.md", "Renamed.md"),
            vec![("Untitled.md".to_string(), "Renamed.md".to_string())]
        );
    }

    /// Renaming a folder that holds a file another process has open with no
    /// sharing must fail with an I/O error naming the move, and leave the
    /// folder exactly where it was: the file list still holds every note
    /// under the old name, because a rename that half-applied would leave
    /// the index describing a tree the disk no longer has.
    #[cfg(windows)]
    #[test]
    fn renaming_a_folder_holding_a_locked_file_is_refused_not_half_applied() {
        use std::fs::OpenOptions;
        use std::os::windows::fs::OpenOptionsExt;
        let (dir, mut v) = vault();
        let root = dir.path();
        let held = OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(root.join("Sub/S.md"));
        let held = match held {
            Ok(h) => h,
            Err(_) => return, // CI without the right to open exclusively
        };

        let err = v
            .rename("Sub", "Moved")
            .expect_err("the move must be refused");
        assert!(
            matches!(err, OpError::Io { .. }),
            "a locked file in the way must surface as an io error: {err}"
        );
        assert!(
            err.to_string().contains("Moved") || err.to_string().contains("Sub"),
            "{err}"
        );
        // * Nothing moved: the disk and the index still agree.
        assert!(root.join("Sub/S.md").is_file(), "the folder moved anyway");
        assert!(
            !root.join("Moved").exists(),
            "a half-applied rename left a destination"
        );
        let paths: Vec<&str> = paths(&v);
        assert!(
            paths.contains(&"Sub/S.md"),
            "the index dropped the note: {paths:?}"
        );
        assert!(
            paths.contains(&"Sub/Deep/D.md"),
            "the index dropped a nested note: {paths:?}"
        );
        assert!(
            paths.iter().all(|p| !p.starts_with("Moved")),
            "the index lists the new name"
        );
        assert!(
            v.folders().contains(&"Sub".to_string()),
            "the folder list moved early"
        );
        drop(held);
    }

    /// A name the extension splitter has to think about. Trashing and
    /// restoring it must come back to the same file, because a
    /// stem/extension split that takes the wrong half would restore the note
    /// under a different name — or leave the user's file in `.trash` under a
    /// name the restore cannot match.
    #[test]
    fn a_trashed_note_with_an_odd_name_comes_back_under_the_same_name() {
        for name in [
            "notes",               // * No extension at all: the whole name is the stem.
            "archive.tar.gz",      // * Two dots: the last one is the extension.
            "Ω notes ✓.md",        // * Spaces and non-ASCII letters.
            "name with spaces.md", // * Blanks in the middle are just bytes.
        ] {
            let (dir, mut v) = vault();
            let note = dir.path().join(name);
            // * Through the vault API, so the note is in the file list the
            // * way a note the user made is.
            v.create_note(name, "kept bytes\n").unwrap();
            assert!(note.is_file(), "{name} was not created");
            assert_eq!(
                v.trash(name, TrashOption::Local).unwrap(),
                vec![name.to_string()],
                "{name} did not move"
            );
            assert!(
                v.list_trash().unwrap().iter().any(|t| t == name),
                "{name} is not in .trash: {:?}",
                v.list_trash().unwrap()
            );
            assert!(!note.exists(), "{name} stayed behind");

            v.restore_from_trash(name).unwrap();
            assert_eq!(
                fs::read_to_string(&note).unwrap(),
                "kept bytes\n",
                "{name} came back different"
            );
        }
    }

    /// Restoring into a name the vault already holds must not overwrite it:
    /// the trashed copy stays in `.trash` and the note on disk keeps its
    /// bytes, because a restore that silently replaced the current file would
    /// trade the user's newer note for the older one. And a name that tries to
    /// leave the vault — `..`, or an absolute path — is refused before any
    /// file moves, the same way every other write path is.
    #[test]
    fn a_restore_refuses_a_name_the_vault_already_holds_or_that_leaves_it() {
        let (dir, mut v) = vault();
        fs::create_dir_all(dir.path().join(".trash")).unwrap();

        // * A note trashed, and a new note written under the same name.
        v.trash("A.md", TrashOption::Local).unwrap();
        v.create_note("A.md", "the newer note\n").unwrap();
        let keep = fs::read_to_string(dir.path().join("A.md")).unwrap();
        assert!(v.list_trash().unwrap().iter().any(|t| t == "A.md"));

        let err = v.restore_from_trash("A.md").expect_err("the name is taken");
        assert!(matches!(err, OpError::DestinationExists), "{err}");
        // * Neither copy was touched by the refusal.
        assert_eq!(
            fs::read_to_string(dir.path().join("A.md")).unwrap(),
            keep,
            "the current note was replaced by the restore"
        );
        assert!(
            v.list_trash().unwrap().iter().any(|t| t == "A.md"),
            "the trashed copy disappeared"
        );

        // * A name that would leave the vault never moves anything: refused
        // * before any file moves, and nothing appears outside the vault.
        for escape in ["../../outside.md", "../outside.md", "/etc/passwd"] {
            let err = v.restore_from_trash(escape);
            assert!(err.is_err(), "{escape:?} must be refused, not restored");
        }
        assert!(
            !dir.path().join("outside.md").exists(),
            "a restore escaped to the vault root"
        );
        assert!(
            !dir.path().parent().unwrap().join("outside.md").exists(),
            "a restore escaped the vault"
        );
    }

    /// The `System` option is a preference, not a promise: on a desktop with
    /// no system trash (Windows, or a read-only data directory) the vault's
    /// own `.trash` is the fallback, and the note must land there rather than
    /// be reported as failed. A user who picked "move to system trash" and
    /// got an error would think the note was still there — or lose it.
    #[test]
    fn a_system_trash_with_no_system_trash_lands_in_the_vaults_own() {
        let (dir, mut v) = vault();
        let note = dir.path().join("A.md");
        let bytes = fs::read_to_string(&note).unwrap();

        let removed = v
            .trash("A.md", TrashOption::System)
            .expect("the vault's own trash is the fallback, not an error");
        assert_eq!(
            removed,
            vec!["A.md".to_string()],
            "the note did not go anywhere"
        );

        // * Where it landed depends on the desktop: the vault's own `.trash`
        // * when there is no system trash to answer, the freedesktop trash
        // * when there is. Either way the bytes survive and the index
        // * agrees — that is the contract, not which folder took them.
        let trashed = dir.path().join(".trash/A.md");
        if trashed.is_file() {
            assert_eq!(
                fs::read_to_string(&trashed).unwrap(),
                bytes,
                "the bytes were lost"
            );
        }
        assert!(!note.exists(), "the original stayed behind");
        assert!(
            !paths(&v).contains(&"A.md"),
            "the index still lists a note that is in .trash"
        );

        // * And when the fallback took it, it comes back whole — which is
        // * the point of falling back. A desktop's system trash restores
        // * through the desktop, not through here.
        if trashed.is_file() {
            v.restore_from_trash("A.md")
                .expect("the fallback copy restores");
            assert_eq!(
                fs::read_to_string(&note).unwrap(),
                bytes,
                "the restore lost bytes"
            );
            assert!(
                paths(&v).contains(&"A.md"),
                "the restored note is not in the index"
            );
        }
    }

    #[test]
    fn trash_goes_to_the_local_folder_with_numbered_collisions() {
        let (dir, mut v) = vault();
        fs::create_dir_all(dir.path().join(".trash")).unwrap();
        fs::write(dir.path().join(".trash/A.md"), "old").unwrap();
        assert_eq!(v.trash("A.md", TrashOption::Local).unwrap(), vec!["A.md"]);
        assert!(dir.path().join(".trash/A 2.md").is_file());
        v.create_note("A.md", "again").unwrap();
        v.trash("A.md", TrashOption::Local).unwrap();
        assert!(dir.path().join(".trash/A 3.md").is_file());
        let removed = v.trash("Sub", TrashOption::Local).unwrap();
        assert_eq!(removed, vec!["Sub/Deep/D.md", "Sub/S.md", "Sub/pic.png"]);
        assert!(dir.path().join(".trash/Sub/Deep/D.md").is_file());
        assert_eq!(paths(&v), vec!["Untitled.md"]);
        assert_eq!(v.folders(), ["Inbox"]);
        v.trash("Untitled.md", TrashOption::None).unwrap();
        assert!(paths(&v).is_empty());
        assert!(matches!(
            v.trash("Untitled.md", TrashOption::None),
            Err(OpError::NotFound(_))
        ));
    }

    /// A trashed folder comes back whole: every note under it, at its own
    /// path, listed again in both the file list and the folder list. A
    /// restore that left the folder on disk but absent from the lists would
    /// make every note inside it unreachable — the folder would not show, so
    /// the user could not open the notes they can see exist. And the name the
    /// vault already holds is refused, the same as for a note, so a restore
    /// can never nest one user's folder inside another's.
    #[test]
    fn a_trashed_folder_comes_back_with_every_note_in_it() {
        let (dir, mut v) = vault();

        let removed = v.trash("Sub", TrashOption::Local).unwrap();
        assert_eq!(
            removed.len(),
            3,
            "every file under the folder moved: {removed:?}"
        );
        assert!(
            dir.path().join(".trash/Sub/Deep/D.md").is_file(),
            "the folder is not in .trash as a whole"
        );
        assert!(!paths(&v).contains(&"Sub/S.md"), "the note is still listed");

        v.restore_from_trash("Sub")
            .expect("a trashed folder restores");

        // * Files and folders are back where they were, all of them.
        let mut files = paths(&v);
        files.sort();
        assert_eq!(
            files,
            vec![
                "A.md",
                "Sub/Deep/D.md",
                "Sub/S.md",
                "Sub/pic.png",
                "Untitled.md"
            ],
            "the folder did not come back complete"
        );
        let mut folders: Vec<String> = v.folders().to_vec();
        folders.sort();
        assert_eq!(
            folders,
            vec!["Inbox", "Sub", "Sub/Deep"],
            "the folders are not listed"
        );

        // * The bytes survived, and the trashed copy is gone rather than copied.
        assert_eq!(
            fs::read_to_string(dir.path().join("Sub/S.md")).unwrap(),
            "s"
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("Sub/Deep/D.md")).unwrap(),
            "d"
        );
        assert!(
            !dir.path().join(".trash/Sub").exists(),
            "the trashed folder stayed behind as a second copy"
        );

        // * A name the vault already holds is refused, folder or not.
        v.create_folder("Docs").unwrap();
        v.trash("Inbox", TrashOption::Local).unwrap();
        v.create_folder("Inbox").unwrap();
        let err = v
            .restore_from_trash("Inbox")
            .expect_err("the name is taken");
        assert!(matches!(err, OpError::DestinationExists), "{err}");
    }

    /// A note trashed from *inside a folder* comes back to that folder, not to
    /// the vault root. The flat `.trash` keeps only the file name, so a
    /// restore that reads it back naively would land the note at the root:
    /// the folder it lived in would lose it, a same-named note at the root
    /// would refuse the restore, and every relative link in the note would
    /// silently point somewhere else. That looks like a moved note, not a
    /// broken one.
    #[test]
    fn a_note_trashed_inside_a_folder_returns_to_that_folder() {
        let (dir, mut v) = vault();

        v.trash("Sub/S.md", TrashOption::Local).unwrap();
        let in_trash = v.list_trash().unwrap();
        assert!(
            in_trash.iter().any(|t| t == "S.md"),
            "the note is not in .trash: {in_trash:?}"
        );

        v.restore_from_trash("S.md")
            .expect("a trashed note restores");

        assert!(
            dir.path().join("Sub/S.md").is_file(),
            "the note came back to the vault root instead of Sub/"
        );
        assert!(
            !dir.path().join("S.md").exists(),
            "the note landed at the root"
        );
        let mut files = paths(&v);
        files.sort();
        assert_eq!(
            files,
            vec![
                "A.md",
                "Sub/Deep/D.md",
                "Sub/S.md",
                "Sub/pic.png",
                "Untitled.md"
            ],
            "the note is not back where it was"
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("Sub/S.md")).unwrap(),
            "s",
            "the bytes were lost"
        );

        // * The record that said where it came from is gone, and it was never
        // * listed as one of the user's trashed notes.
        assert!(
            !dir.path().join(".trash/.S.md.origin").exists(),
            "the sidecar stayed behind after the restore"
        );
        let listed = v.list_trash().unwrap();
        assert!(
            listed.iter().all(|n| !n.starts_with('.')),
            "the vault's own records showed up as notes: {listed:?}"
        );

        // * A record is data, not instruction: one planted saying the note
        // * came from outside the vault is refused by the same gate as any
        // * other path, and the note stays put in `.trash` rather than
        // * being written somewhere the user did not ask for.
        v.trash("Sub/S.md", TrashOption::Local).unwrap();
        assert!(
            dir.path().join(".trash/S.md").is_file(),
            "the second trash did not go back to the plain name"
        );
        std::fs::write(
            dir.path().join(".trash/.S.md.origin"),
            "../../outside-folder",
        )
        .unwrap();
        let err = v
            .restore_from_trash("S.md")
            .expect_err("the record names a place outside the vault");
        assert!(
            dir.path().join(".trash/S.md").is_file(),
            "the note was moved on the strength of a record that failed to resolve"
        );
        assert!(
            !dir.path().join("../outside-folder").exists(),
            "the note escaped the vault: {err:?}"
        );
    }

    /// The vault's own `.trash` slot can be taken by something that is not a
    /// folder — another tool drops a file there, or a sync client puts a
    /// placeholder. The trash then cannot be made, and the move must fail
    /// with the note exactly where it was: a note half-moved, or worse one
    /// gone with no copy anywhere, is the shape this has to refuse. The
    /// failing folder is named in the error so the user can go and look at
    /// what is squatting on it.
    #[test]
    fn a_squatting_trash_refuses_the_local_option_and_keeps_the_note() {
        let (dir, mut v) = vault();

        let squatter = dir.path().join(".trash");
        std::fs::write(&squatter, "not a folder\n").unwrap();
        let note = dir.path().join("A.md");
        let bytes = fs::read_to_string(&note).unwrap();

        // * Local trash has to go through the vault's own `.trash`, so the
        // * squatter refuses it, and nothing may move on a failed trash.
        // * (The System option on a desktop with a freedesktop trash never
        // * touches `.trash` at all, so a squatter cannot block it; that
        // * behaviour is the next test's subject.)
        let err = v
            .trash("A.md", TrashOption::Local)
            .expect_err("a file is squatting on .trash");
        assert!(
            matches!(err, OpError::Io { .. }),
            "the failure is not reported as an io error naming the folder: {err:?}"
        );
        // * Nothing moved, and the bytes are the user's.
        assert!(note.is_file(), "the note moved on a failed trash");
        assert_eq!(fs::read_to_string(&note).unwrap(), bytes);
        assert!(
            paths(&v).contains(&"A.md"),
            "the index lost a note that is still on disk"
        );

        // * The squatter is untouched: refusing the trash is not a licence to
        // * delete whatever was in the way.
        assert_eq!(fs::read_to_string(&squatter).unwrap(), "not a folder\n");

        // * Once the slot is a folder again, the same trash works — the vault
        // * is not left permanently unable to trash.
        std::fs::remove_file(&squatter).unwrap();
        let removed = v.trash("A.md", TrashOption::Local).unwrap();
        assert_eq!(removed, vec!["A.md".to_string()]);
        assert!(dir.path().join(".trash/A.md").is_file());
    }

    /// An empty note is a note, not a hole: a check anywhere that treated
    /// "no bytes" as "not there" would lose it the moment it was moved —
    /// trashed, restored or renamed — and the user would have no note where
    /// they had one. It has to come back with nothing in it and still be
    /// listed, because that is what a note the user deliberately left blank
    /// looks like.
    #[test]
    fn an_empty_note_survives_a_move_and_stays_a_listed_note() {
        let (dir, mut v) = vault();
        let empty = dir.path().join("Blank.md");
        v.create_note("Blank.md", "").unwrap();
        assert!(empty.is_file(), "an empty note was not created");
        assert!(
            paths(&v).contains(&"Blank.md"),
            "the empty note is not indexed"
        );

        // * Trashed and back, with nothing in it and still indexed.
        v.trash("Blank.md", TrashOption::Local).unwrap();
        assert!(
            dir.path().join(".trash/Blank.md").is_file(),
            "the empty note is not in .trash as a file"
        );
        v.restore_from_trash("Blank.md").unwrap();
        assert!(empty.is_file(), "the empty note did not come back");
        assert_eq!(fs::read_to_string(&empty).unwrap(), "");
        assert!(
            paths(&v).contains(&"Blank.md"),
            "the restored empty note is not indexed"
        );

        // * And renamed, which is where a "no bytes, so nothing to keep"
        // * shortcut would drop it.
        let moved = dir.path().join("Renamed.md");
        v.rename("Blank.md", "Renamed.md").unwrap();
        assert!(moved.is_file(), "the empty note vanished on a rename");
        assert_eq!(fs::read_to_string(&moved).unwrap(), "");
        assert!(
            paths(&v).contains(&"Renamed.md"),
            "the renamed empty note is not indexed"
        );
        assert!(!empty.exists(), "the empty note also stayed behind");
    }

    /// A note the size of a real journal is the same note: nothing in the
    /// write, rename or trash path may drop part of it. A truncation would
    /// look exactly like a note the user had not finished writing, and the
    /// only trace would be the missing tail. Every byte is checked, in both
    /// directions of the move.
    #[test]
    fn a_large_note_keeps_every_byte_through_rename_and_trash() {
        let (dir, mut v) = vault();
        // * A repeating pattern, so a truncation or a repeat cannot pass.
        let text: String = (0..1_048_576u32)
            .map(|i| char::from(b'a' + u8::try_from(i % 26).unwrap()))
            .collect();
        v.create_note("Big.md", &text).unwrap();
        assert_eq!(fs::read_to_string(dir.path().join("Big.md")).unwrap(), text);

        v.rename("Big.md", "Moved.md").unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join("Moved.md")).unwrap(),
            text,
            "the rename lost part of a large note"
        );

        v.trash("Moved.md", TrashOption::Local).unwrap();
        v.restore_from_trash("Moved.md").unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join("Moved.md")).unwrap(),
            text,
            "the trash round trip lost part of a large note"
        );

        // * And overwriting it keeps every byte of the new, larger text: this
        // * is the save path the app uses for note content.
        let bigger = format!("{text}{text}");
        v.write_bytes("Moved.md", bigger.as_bytes())
            .expect("the overwrite worked");
        assert_eq!(
            fs::read_to_string(dir.path().join("Moved.md")).unwrap(),
            bigger,
            "an overwrite lost part of a large note"
        );
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn freedesktop_records_are_well_formed() {
        assert_eq!(
            url_encode_path("/home/me/My Vault/Ä.md"),
            "/home/me/My%20Vault/%C3%84.md"
        );
        let stamp = utc_timestamp();
        assert_eq!(stamp.len(), 19);
        assert_eq!(&stamp[10..11], "T");
        assert!(stamp.starts_with("20"));
    }

    #[test]
    fn list_trash_is_empty_when_there_is_no_trash_folder() {
        let (_dir, v) = vault();
        assert_eq!(v.list_trash().unwrap(), Vec::<String>::new());
    }

    #[test]
    fn list_trash_returns_every_trashed_entry_sorted() {
        let (dir, v) = vault();
        fs::create_dir_all(dir.path().join(".trash")).unwrap();
        fs::write(dir.path().join(".trash/Z.md"), "z").unwrap();
        fs::write(dir.path().join(".trash/A.md"), "a").unwrap();
        fs::write(dir.path().join(".trash/M.md"), "m").unwrap();
        assert_eq!(
            v.list_trash().unwrap(),
            vec!["A.md".to_string(), "M.md".to_string(), "Z.md".to_string(),]
        );
    }

    #[test]
    fn restore_from_trash_puts_the_file_back_at_its_entry_path() {
        let (dir, mut v) = vault();
        fs::create_dir_all(dir.path().join(".trash")).unwrap();
        v.trash("A.md", TrashOption::Local).unwrap();
        assert!(!dir.path().join("A.md").exists());
        assert!(dir.path().join(".trash/A.md").exists());
        v.restore_from_trash("A.md").unwrap();
        assert!(dir.path().join("A.md").exists());
        assert!(!dir.path().join(".trash/A.md").exists());
        assert!(v.exists("A.md"));
    }

    #[test]
    fn restore_from_trash_errors_when_entry_is_missing_or_target_exists() {
        let (dir, mut v) = vault();
        fs::create_dir_all(dir.path().join(".trash")).unwrap();
        assert!(matches!(
            v.restore_from_trash("Gone.md"),
            Err(OpError::NotFound(_))
        ));
        fs::write(dir.path().join(".trash/Untitled.md"), "trashed").unwrap();
        assert!(matches!(
            v.restore_from_trash("Untitled.md"),
            Err(OpError::DestinationExists)
        ));
    }

    // * A restore that refuses must not consume the trashed file: the entry
    //   is the only copy left, so a failed attempt that empties .trash is
    //   data loss wearing an error's clothes.
    #[test]
    fn a_failed_restore_keeps_the_trashed_entry() {
        let (dir, v) = vault();
        fs::create_dir_all(dir.path().join(".trash")).unwrap();
        let mut v = v;
        fs::write(dir.path().join(".trash/Keep.md"), "only copy").unwrap();
        fs::write(dir.path().join("Keep.md"), "in the way").unwrap();
        assert!(matches!(
            v.restore_from_trash("Keep.md"),
            Err(OpError::DestinationExists)
        ));
        assert_eq!(
            fs::read_to_string(dir.path().join(".trash/Keep.md")).unwrap(),
            "only copy"
        );
    }

    // * A rename that finds the destination taken must refuse, and the file
    //   already there must survive byte for byte: the window between the
    //   exists check and fs::rename is where a silent overwrite loses the
    //   only copy of someone else's note (Unix rename replaces; this asserts
    //   the vault never does).
    #[test]
    fn a_refused_rename_never_overwrites_the_destination() {
        let (dir, mut v) = vault();
        fs::write(dir.path().join("A.md"), "mine").unwrap();
        fs::write(dir.path().join("B.md"), "theirs").unwrap();
        assert!(matches!(
            v.rename("A.md", "B.md"),
            Err(OpError::DestinationExists)
        ));
        assert_eq!(
            fs::read_to_string(dir.path().join("B.md")).unwrap(),
            "theirs"
        );
        assert_eq!(fs::read_to_string(dir.path().join("A.md")).unwrap(), "mine");
    }

    #[test]
    fn exists_is_false_for_empty_and_missing_true_for_present() {
        let (_dir, v) = vault();
        assert!(!v.exists(""));
        assert!(!v.exists("Nope.md"));
        assert!(v.exists("A.md"));
        assert!(v.exists("Sub/Deep/D.md"));
    }

    #[test]
    fn trash_option_from_config_maps_local_none_and_falls_back_to_system() {
        assert_eq!(TrashOption::from_config(Some("local")), TrashOption::Local);
        assert_eq!(TrashOption::from_config(Some("none")), TrashOption::None);
        assert_eq!(TrashOption::from_config(None), TrashOption::System);
        assert_eq!(
            TrashOption::from_config(Some("whatever")),
            TrashOption::System
        );
        assert_eq!(TrashOption::from_config(Some("Local")), TrashOption::System);
    }

    // * A backup is a copy with its own folder: every note lands in it, and
    //   the vault it came from is untouched by both the copy and the prune.
    //   It works on a plain folder, so the tests need no `Vault` at all.
    fn plain_vault() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        fs::create_dir_all(root.join("Sub/Deep")).unwrap();
        fs::write(root.join("A.md"), "aa").unwrap();
        fs::write(root.join("Sub/B.md"), "bbb").unwrap();
        fs::write(root.join("Sub/Deep/C.md"), "cccc").unwrap();
        (dir, root)
    }

    #[test]
    fn a_backup_copies_the_notes_and_leaves_the_vault_alone() {
        let (_dir, root) = plain_vault();
        let backups = root.join(".silica").join("backups");
        let report = super::backup_vault(&root, &backups, 7).unwrap();

        assert_eq!(report.files, 3);
        assert_eq!(report.bytes, 9);
        assert!(
            report
                .dir
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("vault-")),
            "named {}",
            report.dir.display()
        );
        assert_eq!(fs::read_to_string(report.dir.join("A.md")).unwrap(), "aa");
        assert_eq!(
            fs::read_to_string(report.dir.join("Sub/Deep/C.md")).unwrap(),
            "cccc"
        );
        // * The vault is the same vault it was.
        assert_eq!(fs::read_to_string(root.join("A.md")).unwrap(), "aa");
        assert!(root.join("Sub").is_dir());
    }

    // * The repo, the backup folder itself and a note that vanished mid-walk
    //   are all outside a backup's remit, and none of them may stop it.
    #[test]
    fn a_backup_skips_the_repo_and_its_own_folder() {
        let (_dir, root) = plain_vault();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        let backups = root.join(".silica").join("backups");
        fs::create_dir_all(backups.join("vault-1")).unwrap();
        fs::write(backups.join("vault-1/A.md"), "old").unwrap();

        let report = super::backup_vault(&root, &backups, 7).unwrap();

        assert_eq!(report.files, 3);
        assert!(!report.dir.join(".git").exists());
        assert!(!report.dir.join(".silica").exists());
        assert!(!report.dir.join("vault-1").exists());
    }

    #[test]
    fn a_second_backup_in_the_same_second_gets_its_own_folder() {
        let (_dir, root) = plain_vault();
        let backups = root.join(".silica").join("backups");
        let first = super::backup_vault(&root, &backups, 7).unwrap().dir;
        let second = super::backup_vault(&root, &backups, 7).unwrap().dir;

        assert_ne!(first, second);
        assert!(first.exists() && second.exists());
    }

    #[test]
    fn backups_are_pruned_to_the_cap_without_eating_the_newest() {
        let (_dir, root) = plain_vault();
        let backups = root.join(".silica").join("backups");
        let first = super::backup_vault(&root, &backups, 2).unwrap().dir;
        let second = super::backup_vault(&root, &backups, 2).unwrap().dir;
        let third = super::backup_vault(&root, &backups, 2).unwrap().dir;

        // * Three runs with a cap of two: the oldest goes, the newest two stay.
        assert!(!first.exists(), "{first:?} should have been pruned");
        assert!(second.exists());
        assert!(third.exists());
        // * The vault's own note was never touched by any of it.
        assert_eq!(fs::read_to_string(root.join("A.md")).unwrap(), "aa");
    }

    #[test]
    fn a_cap_of_zero_still_keeps_the_backup_it_just_made() {
        let (_dir, root) = plain_vault();
        let backups = root.join(".silica").join("backups");
        let report = super::backup_vault(&root, &backups, 0).unwrap();

        assert!(report.dir.exists());
        assert_eq!(report.files, 3);
    }

    // * A torn run (crash mid-copy, no completion marker) is worse than no
    //   backup: prune deletes every incomplete folder and never counts it
    //   toward `keep`, so complete backups survive instead of being eaten by
    //   a useless half-copy that sorts newest.
    #[test]
    fn prune_deletes_torn_backups_and_keeps_only_complete_newest() {
        let (_dir, root) = plain_vault();
        let backups = root.join(".silica").join("backups");
        fs::create_dir_all(&backups).unwrap();

        // Oldest complete backup.
        let old_complete = backups.join("vault-100");
        fs::create_dir_all(&old_complete).unwrap();
        fs::write(old_complete.join(".complete"), "").unwrap();
        fs::write(old_complete.join("A.md"), "aa").unwrap();

        // Torn backup that sorts newest: copied partway, never finished.
        let torn = backups.join("vault-200");
        fs::create_dir_all(&torn).unwrap();
        fs::write(torn.join("A.md"), "half").unwrap(); // no .complete marker

        // Newest complete backup.
        let new_complete = backups.join("vault-300");
        fs::create_dir_all(&new_complete).unwrap();
        fs::write(new_complete.join(".complete"), "").unwrap();
        fs::write(new_complete.join("A.md"), "aa").unwrap();

        super::prune_backups(&backups, 1);

        assert!(!torn.exists(), "torn backup should be deleted");
        assert!(!old_complete.exists(), "oldest complete should be pruned");
        assert!(new_complete.exists(), "newest complete should survive");
    }
}

/// Where a daily note goes, and which one is next to it.
///
/// The format is a moment format and the folder is a setting; both were
/// observed from the running app, and so was the rule that matters most:
/// the configured folder has to exist already, while folders the *format*
/// implies (`YYYY/MM/YYYY-MM-DD`) are created. See
/// [[Specification#Observed daily notes behavior, Obsidian 1.14.4]].
pub mod daily {
    use super::{join, Vault};
    use silica_formats::validate_file_name;
    use silica_templates::{moment, Stamp};

    /// How far either way `step` looks for a neighbouring note, and how far
    /// back it recognises the open note's own date.
    const WINDOW: i64 = 400;

    /// Why a daily note could not be named or created.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum DailyError {
        /// The format holds tokens Silica does not render, so the path it
        /// would write is not the path Obsidian would.
        Unsupported(Vec<String>),
        /// The rendered name is not a legal file name.
        BadName(String),
        /// The configured folder is not in the vault. Obsidian reports
        /// "Failed to create daily note. Folder “X” not found." and writes
        /// nothing.
        NoFolder(String),
    }

    impl std::fmt::Display for DailyError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                DailyError::Unsupported(tokens) => write!(
                    f,
                    "Silica cannot render {} in a daily note format",
                    tokens.join(", ")
                ),
                DailyError::BadName(name) => {
                    write!(f, "A daily note cannot be named {name}")
                }
                DailyError::NoFolder(folder) => {
                    write!(
                        f,
                        "Failed to create daily note. Folder \u{201c}{folder}\u{201d} not found."
                    )
                }
            }
        }
    }

    /// The vault path of the daily note for `at`, without checking whether
    /// anything is there.
    pub fn path(format: &str, folder: &str, at: &Stamp) -> Result<String, DailyError> {
        let unsupported = moment::unsupported_tokens(format);
        if !unsupported.is_empty() {
            return Err(DailyError::Unsupported(unsupported));
        }
        let rendered = moment::format(format, at);
        // * A format may name folders of its own, which are created; each
        // * segment still has to be a legal name.
        for segment in rendered.split('/') {
            if validate_file_name(segment).is_err() {
                return Err(DailyError::BadName(rendered.clone()));
            }
        }
        Ok(join(folder.trim_matches('/'), &format!("{rendered}.md")))
    }

    impl Vault {
        /// The daily note for `at`, and whether it is already there.
        ///
        /// The configured folder must exist: Obsidian refuses rather than
        /// creating it, which is the one way this can fail that a user can
        /// do anything about.
        pub fn daily_note(
            &self,
            format: &str,
            folder: &str,
            at: &Stamp,
        ) -> Result<(String, bool), DailyError> {
            let folder = folder.trim_matches('/');
            if !folder.is_empty() && !self.exists(folder) {
                return Err(DailyError::NoFolder(folder.to_string()));
            }
            let path = path(format, folder, at)?;
            let exists = self.exists(&path);
            Ok((path, exists))
        }

        /// The day of the daily note at `path`, if it is one: the date in a
        /// window around `around` whose name renders to it.
        ///
        /// Rendering forward beats parsing backwards, which a moment format
        /// does not support — `DD-MM-YYYY` and `YYYY/MM/DD` would each need
        /// their own reader.
        pub fn daily_note_day(
            &self,
            format: &str,
            folder: &str,
            path: &str,
            around: &Stamp,
        ) -> Option<Stamp> {
            let base = around.days_from_epoch();
            (-WINDOW..=WINDOW).find_map(|delta| {
                let at = Stamp::from_unix((base + delta) * 86_400, 0);
                let rendered = self::path(format, folder.trim_matches('/'), &at).ok()?;
                (rendered == path).then_some(at)
            })
        }

        /// The nearest existing daily note before or after `from`.
        ///
        /// Obsidian steps over the days that have no note: from a note on
        /// the 10th with the 9th missing, "previous" is the 2nd.
        pub fn step_daily_note(
            &self,
            format: &str,
            folder: &str,
            from: &Stamp,
            forward: bool,
        ) -> Option<String> {
            let folder = folder.trim_matches('/');
            let base = from.days_from_epoch();
            (1..=WINDOW).find_map(|delta| {
                let day = if forward { base + delta } else { base - delta };
                let at = Stamp::from_unix(day * 86_400, 0);
                let candidate = path(format, folder, &at).ok()?;
                self.exists(&candidate).then_some(candidate)
            })
        }
    }
}

#[cfg(test)]
mod daily_tests {
    use super::daily::{self, DailyError};
    use crate::Vault;
    use silica_templates::Stamp;
    use std::fs;

    fn at() -> Stamp {
        Stamp {
            year: 2026,
            month: 10,
            day: 18,
            hour: 9,
            minute: 7,
            second: 3,
        }
    }

    fn vault(folders: &[&str], notes: &[&str]) -> (tempfile::TempDir, Vault) {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".obsidian")).unwrap();
        for folder in folders {
            fs::create_dir_all(dir.path().join(folder)).unwrap();
        }
        for note in notes {
            let path = dir.path().join(note);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "# note\n").unwrap();
        }
        let vault = Vault::open(dir.path()).unwrap();
        (dir, vault)
    }

    #[test]
    fn the_path_is_the_folder_and_the_rendered_format() {
        assert_eq!(
            daily::path("YYYY-MM-DD", "Journal", &at()).unwrap(),
            "Journal/2026-10-18.md"
        );
        assert_eq!(
            daily::path("YYYY-MM-DD", "", &at()).unwrap(),
            "2026-10-18.md"
        );
        // * A format may name folders of its own.
        assert_eq!(
            daily::path("YYYY/MM/YYYY-MM-DD", "Journal", &at()).unwrap(),
            "Journal/2026/10/2026-10-18.md"
        );
        // * A slash around the folder is not part of it.
        assert_eq!(
            daily::path("YYYY-MM-DD", "/Journal/", &at()).unwrap(),
            "Journal/2026-10-18.md"
        );
    }

    #[test]
    fn a_format_silica_cannot_render_is_refused() {
        // * Writing the wrong path would be a note in the wrong place, so a
        // * token the formatter does not know stops the whole thing.
        let err = daily::path("GGGG-[W]ww", "Journal", &at()).unwrap_err();
        assert!(matches!(err, DailyError::Unsupported(_)), "{err:?}");
        assert!(err.to_string().contains("GGGG"));
        // * A format rendering a forbidden character is refused too.
        let err = daily::path("YYYY:MM", "Journal", &at()).unwrap_err();
        assert!(matches!(err, DailyError::BadName(_)), "{err:?}");
    }

    #[test]
    fn the_configured_folder_has_to_exist_already() {
        let (_dir, vault) = vault(&["Journal"], &[]);
        assert_eq!(
            vault.daily_note("YYYY-MM-DD", "Journal", &at()).unwrap(),
            ("Journal/2026-10-18.md".to_string(), false)
        );
        // * Obsidian writes nothing and says so; it does not create it.
        let err = vault
            .daily_note("YYYY-MM-DD", "MadeByProbe", &at())
            .unwrap_err();
        assert_eq!(err, DailyError::NoFolder("MadeByProbe".to_string()));
        assert_eq!(
            err.to_string(),
            "Failed to create daily note. Folder \u{201c}MadeByProbe\u{201d} not found."
        );
        // * The root is always there.
        assert!(vault.daily_note("YYYY-MM-DD", "", &at()).is_ok());
    }

    #[test]
    fn an_existing_note_is_reported_rather_than_replaced() {
        let (_dir, vault) = vault(&["Journal"], &["Journal/2026-10-18.md"]);
        let (path, exists) = vault.daily_note("YYYY-MM-DD", "Journal", &at()).unwrap();
        assert_eq!((path.as_str(), exists), ("Journal/2026-10-18.md", true));
    }

    #[test]
    fn stepping_skips_the_days_that_have_no_note() {
        // * The gappy set the probe used: from the 10th, previous is the 2nd
        // * and next is the 20th.
        let (_dir, vault) = vault(
            &["Journal"],
            &[
                "Journal/2026-03-01.md",
                "Journal/2026-03-02.md",
                "Journal/2026-03-10.md",
                "Journal/2026-03-20.md",
            ],
        );
        let tenth = Stamp {
            year: 2026,
            month: 3,
            day: 10,
            hour: 0,
            minute: 0,
            second: 0,
        };
        assert_eq!(
            vault.step_daily_note("YYYY-MM-DD", "Journal", &tenth, false),
            Some("Journal/2026-03-02.md".to_string())
        );
        assert_eq!(
            vault.step_daily_note("YYYY-MM-DD", "Journal", &tenth, true),
            Some("Journal/2026-03-20.md".to_string())
        );
        // * Nothing beyond the last one.
        let twentieth = Stamp { day: 20, ..tenth };
        assert_eq!(
            vault.step_daily_note("YYYY-MM-DD", "Journal", &twentieth, true),
            None
        );
    }

    #[test]
    fn a_note_s_own_day_is_found_by_rendering_rather_than_parsing() {
        let (_dir, vault) = vault(&["Journal"], &["Journal/18-10-2026.md"]);
        // * A format that does not sort chronologically still resolves.
        let day = vault
            .daily_note_day("DD-MM-YYYY", "Journal", "Journal/18-10-2026.md", &at())
            .expect("the note's own day");
        assert_eq!((day.year, day.month, day.day), (2026, 10, 18));
        // * A note that is not a daily note at all has no day.
        assert_eq!(
            vault.daily_note_day("DD-MM-YYYY", "Journal", "Journal/Meeting.md", &at()),
            None
        );
    }

    #[test]
    fn save_attachment_creates_file_and_folders() {
        let (_dir, mut vault) = vault(&[], &[]);
        let bytes = b"test attachment content";
        let changes = vault
            .save_attachment("Attachments/Sub/image.png", bytes)
            .expect("attachment saved");
        assert_eq!(changes.len(), 1);
        assert_eq!(
            changes[0],
            crate::FileChange::Added("Attachments/Sub/image.png".to_string())
        );
        assert!(vault.exists("Attachments/Sub/image.png"));
        let abs = vault.root().join("Attachments/Sub/image.png");
        let read = std::fs::read(&abs).expect("read attachment");
        assert_eq!(read, bytes);
    }

    #[test]
    fn attachment_folder_resolution_and_collision() {
        use super::attachment_folder;
        assert_eq!(attachment_folder(None, "Notes/Doc.md"), "");
        assert_eq!(attachment_folder(Some("/"), "Notes/Doc.md"), "");
        assert_eq!(attachment_folder(Some("./"), "Notes/Doc.md"), "Notes");
        assert_eq!(
            attachment_folder(Some("./assets"), "Notes/Doc.md"),
            "Notes/assets"
        );
        assert_eq!(attachment_folder(Some("Assets"), "Notes/Doc.md"), "Assets");
        assert_eq!(attachment_folder(Some("./assets"), "Root.md"), "assets");

        let (_dir, mut vault) = vault(
            &["Attachments"],
            &["Attachments/Pasted image 20261101120000.png"],
        );
        let folder = attachment_folder(Some("Attachments"), "Notes/Doc.md");
        let available = vault.available_path(&folder, "Pasted image 20261101120000", "png");
        assert_eq!(available, "Attachments/Pasted image 20261101120000 1.png");
        let changes = vault.save_attachment(&available, b"image2").unwrap();
        assert_eq!(
            changes,
            vec![crate::FileChange::Added(
                "Attachments/Pasted image 20261101120000 1.png".into()
            )]
        );
    }

    // * A symlink is re-created as a symlink during fallback copying: it is
    // * never followed, so the copy cannot reach outside the vault or loop.
    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn copy_tree_preserves_a_file_symlink() {
        use std::os::unix::fs::symlink;

        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("real.txt");
        fs::write(&target, "outside").unwrap();
        symlink(&target, src.path().join("link.txt")).unwrap();

        super::copy_tree(src.path(), dst.path()).unwrap();

        let link = dst.path().join("link.txt");
        assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
        assert_eq!(fs::read_link(&link).unwrap(), target);
        assert_eq!(fs::read_to_string(&target).unwrap(), "outside");
    }

    // * A directory symlink is re-created as a symlink, not traversed: the
    // * file it points at is not copied into the destination.
    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn copy_tree_does_not_recurse_a_directory_symlink() {
        use std::os::unix::fs::symlink;

        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret.txt"), "secret").unwrap();
        symlink(outside.path(), src.path().join("dir")).unwrap();

        super::copy_tree(src.path(), dst.path()).unwrap();

        let dir = dst.path().join("dir");
        let meta = dir.symlink_metadata().unwrap();
        assert!(meta.file_type().is_symlink());
        assert!(!meta.file_type().is_dir());
        assert!(!dst.path().join("secret.txt").exists());
    }

    // * A self-referencing symlink cannot make fallback copying loop forever.
    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn copy_tree_does_not_loop_on_a_symlink_loop() {
        use std::os::unix::fs::symlink;

        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        symlink("itself", src.path().join("loop.md")).unwrap();

        super::copy_tree(src.path(), dst.path()).unwrap();

        let link = dst.path().join("loop.md");
        assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
    }
}
