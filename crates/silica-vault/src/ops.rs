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

use crate::{FileChange, Vault};
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

fn io<'p>(action: &'static str, path: &'p Path) -> impl FnOnce(std::io::Error) -> OpError + 'p {
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
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent).map_err(io("create folder", parent))?;
        }
        crate::write_no_follow(&abs, text.as_bytes()).map_err(io("create", &abs))?;
        Ok(self.refresh_path(&abs))
    }

    /// Creates an empty folder (and its parents).
    pub fn create_folder(&mut self, path: &str) -> Result<(), OpError> {
        validate_path(path)?;
        if self.exists(path) {
            return Err(OpError::Exists);
        }
        let abs = self.write_path(path)?;
        std::fs::create_dir_all(&abs).map_err(io("create folder", &abs))?;
        self.refresh_path(&abs);
        Ok(())
    }

    /// Files under `path` when it is a folder, or `path` itself when it is a
    /// file in the list.
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
        // A case-only rename of the same entry is allowed; anything else
        // already at the destination is not.
        let same_entry = from.eq_ignore_ascii_case(to)
            && std::fs::canonicalize(&from_abs).ok() == std::fs::canonicalize(&to_abs).ok();
        if to_abs.exists() && !same_entry {
            return Err(OpError::DestinationExists);
        }
        if to.starts_with(&format!("{from}/")) {
            return Err(OpError::InvalidName("Cannot move a folder into itself."));
        }
        if let Some(parent) = to_abs.parent() {
            std::fs::create_dir_all(parent).map_err(io("create folder", parent))?;
        }
        let moved = self.rename_plan(from, to);
        std::fs::rename(&from_abs, &to_abs).map_err(io("rename", &from_abs))?;
        self.refresh_path(&from_abs);
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
                    // No system trash here (another platform, or a read-only
                    // data directory): the vault's own trash is the fallback.
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
        std::fs::rename(&abs, &target_abs).map_err(io("move to .trash", &abs))
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
    if from.is_dir() {
        std::fs::create_dir_all(to)?;
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
    // Civil-from-days (Howard Hinnant's algorithm).
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
        // A case-only rename keeps the same file.
        v.rename("Renamed A.md", "renamed a.md").unwrap();
        assert!(paths(&v).contains(&"renamed a.md"));
        assert!(v.rename("renamed a.md", "renamed a.md").unwrap().is_empty());
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
        // A format may name folders of its own, which are created; each
        // segment still has to be a legal name.
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
        // A format may name folders of its own.
        assert_eq!(
            daily::path("YYYY/MM/YYYY-MM-DD", "Journal", &at()).unwrap(),
            "Journal/2026/10/2026-10-18.md"
        );
        // A slash around the folder is not part of it.
        assert_eq!(
            daily::path("YYYY-MM-DD", "/Journal/", &at()).unwrap(),
            "Journal/2026-10-18.md"
        );
    }

    #[test]
    fn a_format_silica_cannot_render_is_refused() {
        // Writing the wrong path would be a note in the wrong place, so a
        // token the formatter does not know stops the whole thing.
        let err = daily::path("GGGG-[W]ww", "Journal", &at()).unwrap_err();
        assert!(matches!(err, DailyError::Unsupported(_)), "{err:?}");
        assert!(err.to_string().contains("GGGG"));
        // A format rendering a forbidden character is refused too.
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
        // Obsidian writes nothing and says so; it does not create it.
        let err = vault
            .daily_note("YYYY-MM-DD", "MadeByProbe", &at())
            .unwrap_err();
        assert_eq!(err, DailyError::NoFolder("MadeByProbe".to_string()));
        assert_eq!(
            err.to_string(),
            "Failed to create daily note. Folder \u{201c}MadeByProbe\u{201d} not found."
        );
        // The root is always there.
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
        // The gappy set the probe used: from the 10th, previous is the 2nd
        // and next is the 20th.
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
        // Nothing beyond the last one.
        let twentieth = Stamp { day: 20, ..tenth };
        assert_eq!(
            vault.step_daily_note("YYYY-MM-DD", "Journal", &twentieth, true),
            None
        );
    }

    #[test]
    fn a_note_s_own_day_is_found_by_rendering_rather_than_parsing() {
        let (_dir, vault) = vault(&["Journal"], &["Journal/18-10-2026.md"]);
        // A format that does not sort chronologically still resolves.
        let day = vault
            .daily_note_day("DD-MM-YYYY", "Journal", "Journal/18-10-2026.md", &at())
            .expect("the note's own day");
        assert_eq!((day.year, day.month, day.day), (2026, 10, 18));
        // A note that is not a daily note at all has no day.
        assert_eq!(
            vault.daily_note_day("DD-MM-YYYY", "Journal", "Journal/Meeting.md", &at()),
            None
        );
    }
}
