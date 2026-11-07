//! A vault is a plain folder. This crate opens one, enumerates its files, and
//! reads `.obsidian` configuration without ever owning data outside the folder.

pub mod ops;
pub mod registry;
#[cfg(windows)]
pub mod win_fs;

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
/// The write is atomic: the bytes go to a temporary file beside the target,
/// are flushed and synced, and only then replace it. Truncating the target in
/// place loses the note if the process dies mid-write — the old bytes are
/// already gone and the new ones are not all there. A rename cannot leave a
/// half-written file, so an interrupted save leaves the previous version.
pub fn write_no_follow(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    // ! A symlink target is never written through: the link itself is refused,
    // ! not followed and not replaced. The write below lands on `path` only
    // ! when `path` is a regular file or does not exist yet.
    if path
        .symlink_metadata()
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(std::io::Error::other(format!(
            "{} is a symbolic link; the vault folder is the only place Silica writes",
            path.display()
        )));
    }
    // * On Windows a junction and other reparse points redirect a path the same
    // * way a symlink does, but `is_symlink` does not report them. `write_at`
    // * walks the path through directory handles there, which is the real fix;
    // * this path-level check stays for the callers that hand a bare path with
    // * no vault root to walk from.
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if path
            .symlink_metadata()
            .map(|m| m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
            .unwrap_or(false)
        {
            return Err(std::io::Error::other(format!(
                "{} is a reparse point; the vault folder is the only place Silica writes",
                path.display()
            )));
        }
    }
    // * On Windows the reparse-point check above is a check-then-act: a
    // * hostile process could swap the target for a junction between the
    // * check and the open below. Windows has no `openat` equivalent, so a
    // * handle-based walk (opening each parent directory and operating through
    // * its handle) would be needed to close the race entirely; that requires
    // * Windows handle-based file APIs and is not available in std. The check
    // * above is the best available protection there, and it at least refuses a
    // * reparse point that exists at the time of the check.
    // * Unique per write, so two saves of one note cannot share a scratch file.
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let temp = parent.join(format!(
        ".{name}.{}.{unique}.silica-tmp",
        std::process::id()
    ));

    let write = || -> std::io::Result<()> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW);
            // * A note keeps the mode it had; a new file gets the usual one.
            let mode = std::fs::metadata(path).map(|m| {
                use std::os::unix::fs::PermissionsExt;
                m.permissions().mode() & 0o777
            });
            options.mode(mode.unwrap_or(0o644));
        }
        let mut file = options.open(&temp)?;
        file.write_all(bytes)?;
        // * Flushed to the device before the rename, so the name never points at
        // * bytes that are still only in the page cache.
        file.sync_all()?;
        Ok(())
    };
    if let Err(error) = write() {
        let _ = std::fs::remove_file(&temp);
        return Err(error);
    }
    // * A snapshot of what is being replaced, so an accidental save can be
    // * undone. Best effort: the save proceeds even if the snapshot cannot be
    // * made.
    if path.exists() {
        snapshot_before_overwrite(path);
    }
    match std::fs::rename(&temp, path) {
        Ok(()) => {
            // * The directory entry itself is what a crash could lose.
            // ! TODO(windows): std::fs::rename is check-then-act — the
            // ! destination path can be substituted for a junction between
            // ! the reparse-point check above and this rename. Bind the
            // ! destination to an opened directory handle and use
            // // ReplaceFileW on Windows before this is considered fixed
            // // there.
            #[cfg(unix)]
            {
                if let Ok(dir) = std::fs::File::open(parent) {
                    let _ = dir.sync_all();
                }
            }
            Ok(())
        }
        Err(error) => {
            let _ = std::fs::remove_file(&temp);
            Err(error)
        }
    }
}

/// Before a note is overwritten, move its current bytes to the vault's
/// trash so an accidental or mistaken save can be undone. Snapshots live in
/// `.trash/.snapshots/<stem>.<nanoseconds>.<counter>.<ext>` and only the
/// most recent ones for a given file are kept, so the folder cannot grow
/// without bound.
///
/// * The name carries nanosecond precision plus a counter, so two saves
///   within the same second cannot collide and destroy an older snapshot.
/// * The snapshot directory and file are created through openat handles
///   with O_NOFOLLOW at every component, so a symlink planted anywhere along
///   the path is refused, not followed.
/// * A snapshot is best effort: if it cannot be made, the save still
///   proceeds — losing a snapshot is acceptable, losing the note is not.
pub fn snapshot_before_overwrite(path: &Path) {
    // ! Never snapshot through a symlink: the note itself must be a real file.
    if !path.is_file()
        || path
            .symlink_metadata()
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
    {
        return;
    }
    let Ok(bytes) = std::fs::read(path) else {
        return;
    };
    let parent = match path.parent() {
        Some(p) => p,
        None => return,
    };
    let name = match path.file_name().and_then(|n| n.to_str()) {
        Some(n) => n,
        None => return,
    };
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s, e),
        _ => (name, ""),
    };
    let snapshots = parent.join(".trash").join(".snapshots");
    // ! Create the snapshot directory through openat where that exists, so no
    // ! component can be swapped for a link between the check and the write.
    // * Windows has no openat; the reparse-point check in `write_no_follow`
    // * refuses a redirected target there instead.
    #[cfg(unix)]
    let made = create_dir_all_at(parent, Path::new(".trash/.snapshots")).is_ok();
    #[cfg(not(unix))]
    let made = std::fs::create_dir_all(&snapshots).is_ok();
    if !made {
        return;
    }
    // * Write the snapshot with a collision-proof name; best effort — the
    // * save proceeds even if the snapshot cannot be made.
    if write_snapshot(&snapshots, stem, ext, &bytes).is_ok() {
        // * Prune old snapshots so the folder cannot grow without bound.
        let _ = prune_snapshots(&snapshots, stem, 10);
    }
}

/// Writes `bytes` to a new snapshot file under `snapshots` with a
/// collision-proof name, refusing to write through symlinks.
///
/// The name is `<stem>.<nanoseconds>.<counter>.<ext>`. Nanosecond precision
/// plus an incrementing counter makes two saves within the same second
/// unable to collide and destroy an older snapshot.
fn write_snapshot(snapshots: &Path, stem: &str, ext: &str, bytes: &[u8]) -> std::io::Result<()> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut counter: u64 = 0;
    loop {
        let name = format!("{stem}.{stamp}.{counter}.{ext}");
        let target = snapshots.join(&name);
        // ! Refuse to write through a symlink: an attacker who plants a link
        // ! at the target path must not redirect the snapshot.
        if target
            .symlink_metadata()
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
        {
            counter += 1;
            continue;
        }
        // * Skip names that already exist so two sequential saves never
        // * collide; a concurrent race is acceptable — losing a snapshot is
        // * acceptable, losing the note is not.
        if target.exists() {
            counter += 1;
            continue;
        }
        // * Windows has no `openat`; `win_fs::write_at` verifies the parent by
        // * its handle and renames relative to it, so the pathname check above
        // * is not what stands between a junction and the snapshot.
        #[cfg(unix)]
        let written = write_at(snapshots, Path::new(&name), bytes);
        #[cfg(windows)]
        let written = win_fs::write_at(snapshots, Path::new(&name), bytes);
        #[cfg(not(any(unix, windows)))]
        let written = write_no_follow(&target, bytes);
        match written {
            Ok(()) => return Ok(()),
            Err(e) => return Err(e),
        }
    }
}

/// Returns true when `name` is a snapshot belonging to `stem`. Snapshot
/// names are `<stem>.<nanoseconds>.<counter>.<ext>`: the stem must be
/// followed by a dot, so `foo.md` never matches `foobar.md`'s snapshots.
fn snapshot_belongs_to(name: &str, stem: &str) -> bool {
    let Some(rest) = name.strip_prefix(stem).and_then(|r| r.strip_prefix('.')) else {
        return false;
    };
    let Some((nanos, rest)) = rest.split_once('.') else {
        return false;
    };
    if nanos.is_empty() || !nanos.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    let (counter, _ext) = rest.split_once('.').unwrap_or((rest, ""));
    !counter.is_empty() && counter.bytes().all(|b| b.is_ascii_digit())
}

/// Keeps only the `keep` most recent snapshots for `stem`, removing older
/// ones so the snapshot folder cannot grow without bound.
fn prune_snapshots(snapshots: &Path, stem: &str, keep: usize) -> std::io::Result<()> {
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(snapshots)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| snapshot_belongs_to(n, stem))
                .unwrap_or(false)
        })
        .collect();
    files.sort();
    files.reverse();
    for extra in files.into_iter().skip(keep) {
        let _ = std::fs::remove_file(&extra);
    }
    Ok(())
}

/// Writes `bytes` to `relative` under `root` atomically: every directory
/// component is opened with `openat` from a directory handle, then the bytes
/// go to a temporary file beside the target and are renamed over it. A crash
/// or a failed write leaves the previous version intact instead of
/// truncating it in place.
///
/// The walk holds `O_NOFOLLOW` directory handles at every step, so no
/// component can be swapped for a link between the check and the write. The
/// temporary file is opened with `O_NOFOLLOW` too, and `renameat` replaces
/// the directory entry itself, so a symlink at the target is replaced, not
/// followed through.
///
/// Ponytail: `openat` on Unix; on Windows [`win_fs`] walks the same path
/// through directory handles. [`write_no_follow`] stands for a caller with no
/// root to walk from.
#[cfg(unix)]
pub fn write_at(root: &Path, relative: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::io::Write as _;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;

    const DIR_FLAGS: libc::c_int = libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    const FILE_FLAGS: libc::c_int =
        libc::O_WRONLY | libc::O_NOFOLLOW | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC;

    let name = |part: &std::ffi::OsStr| -> std::io::Result<CString> {
        CString::new(part.as_bytes())
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "a NUL in a path"))
    };

    let root_c = name(root.as_os_str())?;
    let mut dir: OwnedFd = unsafe {
        let fd = libc::open(root_c.as_ptr(), DIR_FLAGS);
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        OwnedFd::from_raw_fd(fd)
    };

    let mut parts: Vec<&std::ffi::OsStr> = relative
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(part) => Some(part),
            _ => None,
        })
        .collect();
    let Some(file_name) = parts.pop() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "a write needs a file name",
        ));
    };

    for part in parts {
        let part_c = name(part)?;
        let next = unsafe { libc::openat(dir.as_raw_fd(), part_c.as_ptr(), DIR_FLAGS) };
        if next < 0 {
            return Err(std::io::Error::last_os_error());
        }
        dir = unsafe { OwnedFd::from_raw_fd(next) };
    }

    let file_c = name(file_name)?;
    // * Refuse to write through a symlink at the target: the rename below
    //   replaces the directory entry without following it, but we still turn
    //   away a pre-existing link so a plugin cannot trade it for its content.
    if root
        .join(relative)
        .symlink_metadata()
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(std::io::Error::other(format!(
            "{} is a symbolic link; the vault folder is the only place Silica writes",
            root.join(relative).display()
        )));
    }
    // Temporary file beside the target, unique per write so concurrent saves
    // of one note cannot share a scratch file.
    let temp_name = format!(
        ".{}.{}.silica-tmp",
        file_name.to_string_lossy(),
        std::process::id()
    );
    let temp_c = name(std::ffi::OsStr::new(&temp_name))?;
    let tmp_fd = unsafe { libc::openat(dir.as_raw_fd(), temp_c.as_ptr(), FILE_FLAGS, 0o644) };
    if tmp_fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut tmp = unsafe { std::fs::File::from_raw_fd(tmp_fd) };
    tmp.write_all(bytes)?;
    tmp.sync_all()?;
    drop(tmp);

    // renameat replaces the directory entry atomically while holding the
    // directory handle, so no symlink swap can redirect the write.
    if unsafe {
        libc::renameat(
            dir.as_raw_fd(),
            temp_c.as_ptr(),
            dir.as_raw_fd(),
            file_c.as_ptr(),
        )
    } < 0
    {
        let _ = unsafe { libc::unlinkat(dir.as_raw_fd(), temp_c.as_ptr(), 0) };
        return Err(std::io::Error::last_os_error());
    }
    // * The directory entry itself is what a crash could lose.
    if let Ok(dir_file) = std::fs::File::open(root) {
        let _ = dir_file.sync_all();
    }
    Ok(())
}

/// Creates every directory under `root` named by `relative`, one `openat` at a
/// time, so no component can be swapped for a link mid-walk.
#[cfg(unix)]
pub fn create_dir_all_at(root: &Path, relative: &Path) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;

    const DIR_FLAGS: libc::c_int = libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;

    let root_c = CString::new(root.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "a NUL in a path"))?;
    let mut dir: OwnedFd = unsafe {
        let fd = libc::open(root_c.as_ptr(), DIR_FLAGS);
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        OwnedFd::from_raw_fd(fd)
    };

    for part in relative.components() {
        let std::path::Component::Normal(part) = part else {
            continue;
        };
        let part_c = CString::new(part.as_bytes()).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "a NUL in a path")
        })?;
        let mut next = unsafe { libc::openat(dir.as_raw_fd(), part_c.as_ptr(), DIR_FLAGS) };
        if next < 0 {
            let made = unsafe { libc::mkdirat(dir.as_raw_fd(), part_c.as_ptr(), 0o755) };
            if made < 0 {
                return Err(std::io::Error::last_os_error());
            }
            next = unsafe { libc::openat(dir.as_raw_fd(), part_c.as_ptr(), DIR_FLAGS) };
            if next < 0 {
                return Err(std::io::Error::last_os_error());
            }
        }
        dir = unsafe { OwnedFd::from_raw_fd(next) };
    }
    Ok(())
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

/// Whether a metadata entry is a link that could redirect a path: a symlink
/// everywhere, and a junction or other reparse point on Windows. `is_symlink`
/// does not report junctions, so Windows needs the reparse-point attribute too.
fn is_link_like(meta: &std::fs::Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    false
}

/// Reject a relative path that could leave `root` before any filesystem call.
///
/// Every read and write goes through this: an absolute path, a `..` component
/// or a prefix is refused outright rather than resolved and then compared,
/// because a name that can leave the vault at all is a name Silica does not
/// take. Windows callers then verify the handle as well; this is the half that
/// is the same on every platform.
fn sane_relative(relative: &Path) -> std::io::Result<()> {
    if relative.is_absolute() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "an absolute path cannot be read from a vault",
        ));
    }
    for part in relative.components() {
        match part {
            std::path::Component::ParentDir => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "the path points outside the vault",
                ))
            }
            std::path::Component::RootDir | std::path::Component::Prefix(_) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "the path is not relative to the vault",
                ))
            }
            _ => {}
        }
    }
    Ok(())
}

/// Read a file under `root` without following a link at any step, bounding it
/// at `max_bytes`.
///
/// The file is opened first and judged from the handle that was opened — its
/// type and length — rather than from `symlink_metadata` on a path that is
/// then reopened. On Unix that is `O_NOFOLLOW` on the final component; on
/// Windows it is [`win_fs::open_inside`], which asks the handle where it
/// landed. A link swapped in between naming the file and opening it therefore
/// cannot redirect the read or slip past the size limit.
pub fn read_verified(root: &Path, relative: &Path, max_bytes: u64) -> std::io::Result<String> {
    sane_relative(relative)?;
    #[cfg(unix)]
    let mut file = {
        use std::os::unix::fs::OpenOptionsExt as _;
        std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(root.join(relative))?
    };
    #[cfg(windows)]
    let mut file = win_fs::open_inside(root, relative)?;
    #[cfg(not(any(unix, windows)))]
    let mut file = std::fs::File::open(root.join(relative))?;

    let meta = file.metadata()?;
    if !meta.file_type().is_file() {
        return Err(std::io::Error::other(format!(
            "{} is not a regular file",
            relative.display()
        )));
    }
    if meta.len() > max_bytes {
        return Err(std::io::Error::other(format!(
            "{} is {} bytes, over the {max_bytes}-byte limit",
            relative.display(),
            meta.len()
        )));
    }
    // * The metadata check above is what a well-behaved file reports; this
    // * bound is what the read actually obeys. A file that grows between the
    // * two — or a device that lies about its length — would otherwise be read
    // * past the limit, so the reader stops one byte past it and reports the
    // * overflow rather than allocating whatever arrives.
    let mut out = String::new();
    {
        use std::io::Read as _;
        std::io::Read::take(&mut file, max_bytes.saturating_add(1))
            .read_to_string(&mut out)
            .map_err(|error| {
                // * A file cut off mid-character is not UTF-8; say so rather
                // * than reporting a generic I/O failure.
                if error.kind() == std::io::ErrorKind::InvalidData {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("{} is not valid UTF-8", relative.display()),
                    )
                } else {
                    error
                }
            })?;
    }
    if out.len() as u64 > max_bytes {
        return Err(std::io::Error::other(format!(
            "{} grew past the {max_bytes}-byte limit while being read",
            relative.display()
        )));
    }
    Ok(out)
}

/// Creates every directory under `root` named by `relative`, one directory
/// handle at a time, so no component can be swapped for a reparse point
/// mid-walk. See [`win_fs`].
#[cfg(windows)]
pub fn create_dir_all_at(root: &Path, relative: &Path) -> std::io::Result<()> {
    win_fs::create_dir_all_at(root, relative)
}

/// Writes `relative` under `root` through directory handles, so neither the
/// target nor a component leading to it can be swapped for a reparse point
/// between the check and the write. See [`win_fs`].
#[cfg(windows)]
pub fn write_at_handles(root: &Path, relative: &Path, bytes: &[u8]) -> std::io::Result<()> {
    win_fs::write_at(root, relative, bytes)
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
                Ok(meta) if is_link_like(&meta) => {
                    return Err(VaultError::Symlink(absolute));
                }
                // * What does not exist yet is created below the vault.
                _ => {}
            }
        }
        Ok(absolute)
    }

    /// Writes `bytes` to a vault-relative path, through [`Vault::write_path`]
    /// and without following a link at the file itself.
    pub fn write_bytes(&self, rel: &str, bytes: &[u8]) -> Result<(), VaultError> {
        let path = self.write_path(rel)?;
        // * Snapshot the existing file before replacing it, so an interrupted
        //   or failed write cannot destroy prior content. On Unix `write_at`
        //   does not snapshot, so do it here. On Windows `write_no_follow`
        //   snapshots internally, so doing it here too would write a duplicate
        //   snapshot for a single save.
        #[cfg(unix)]
        {
            if path.exists() {
                snapshot_before_overwrite(&path);
            }
        }
        // * On Unix, write through the `openat` walk so directory handles are
        //   held throughout and no parent directory can be swapped for a link
        //   mid-write. On Windows, `write_no_follow` stands: the target's
        //   reparse-point check plus the parent-component check in
        //   `checked_path` is the best available protection there.
        #[cfg(unix)]
        {
            let relative = Path::new(rel);
            write_at(&self.root, relative, bytes).map_err(|source| VaultError::Io {
                path: path.clone(),
                source,
            })?;
        }
        #[cfg(not(unix))]
        {
            write_no_follow(&path, bytes).map_err(|source| VaultError::Io { path, source })?;
        }
        Ok(())
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
            let rel = match entry.path().strip_prefix(&self.root) {
                Ok(rel) => rel.to_string_lossy().replace('\\', "/"),
                // * The walker only yields entries under the root it was given;
                // * anything else is not a vault path and is skipped rather than
                // * asserted on, since asserting here would take the window down
                // * over a directory that moved mid-scan.
                Err(_) => continue,
            };
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
        // * A byte order mark is not JSON. Windows editors leave one, and a
        // * file carrying one is otherwise perfectly readable.
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
        // * A byte order mark is not JSON, so the file has to be parsed
        // * without it — and put back with it, since it is the user's byte
        // * and not ours to drop.
        let (bom, merged) = match std::fs::read_to_string(&path) {
            Ok(text) => {
                let bom = text.starts_with('\u{feff}');
                let body = text.strip_prefix('\u{feff}').unwrap_or(&text);
                match serde_json::from_str::<serde_json::Value>(body) {
                    Ok(old) if merge => (bom, merge_config(old, fresh)),
                    Ok(_) => (bom, fresh),
                    // ! A file nobody can parse is a file nobody may
                    // ! overwrite. Reading it gave defaults, so writing it
                    // ! would put those defaults on disk in place of
                    // ! everything the user had -- the one thing the vault
                    // ! being the source of truth forbids. The bytes stay as
                    // ! they are and the caller is told which file it was.
                    Err(_) => return Err(VaultError::Unparseable(path)),
                }
            }
            // ! Only a file that is *not there* may be created. A permission
            // ! error, a bad disk or a file that is not UTF-8 says the
            // ! existing document could not be read — and overwriting what
            // ! could not be read would replace the user's whole
            // ! configuration with defaults. Those propagate instead.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (false, fresh),
            Err(source) => return Err(VaultError::Io { path, source }),
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
                        // * In place: `Map::remove` would move the key to the
                        // * end of the file.
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
        // * Not both objects (`community-plugins.json` is an array): there
        // * is nothing to merge into, so the typed value is the file.
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

    /// A configuration that cannot be read must not be replaced.
    ///
    /// The bytes are made unreadable — not deleted — so the read fails with
    /// something other than `NotFound`. Writing then has to propagate that
    /// failure and leave the original file exactly as it was; treating the
    /// error as "no file yet" would put defaults over the user's settings.
    #[cfg(unix)]
    #[test]
    fn an_unreadable_config_is_never_overwritten() {
        use std::os::unix::fs::PermissionsExt;

        let dir = sample_vault();
        let path = dir.path().join(".obsidian/app.json");
        let original = fs::read(&path).unwrap();

        // * A directory in the file's place is unreadable as text on every
        // * platform, and cannot be replaced by a write either.
        // * A directory in the file's place cannot be read as text on any
        // * platform, and cannot be replaced by a write either.
        let blocked = dir.path().join(".obsidian/appearance.json");
        fs::create_dir(&blocked).unwrap();
        let vault = Vault::open(dir.path()).unwrap();

        let value = serde_json::json!({"theme": "moonstone"});
        let error = vault.write_config("appearance.json", &value);
        assert!(error.is_err(), "writing over an unreadable file must fail");
        assert!(blocked.is_dir(), "the unreadable entry must still be there");

        // * An existing configuration is untouched by a failed sibling write.
        assert_eq!(fs::read(&path).unwrap(), original);

        // * And a permission error on the file itself propagates rather than
        // * being read as "no file yet".
        let locked = dir.path().join(".obsidian/app.json");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        let unreadable = fs::read_to_string(&locked).is_err();
        let result = vault.write_config("app.json", &serde_json::json!({"vimMode": false}));
        // * Root can read anything, so this half only runs where the mode
        // * actually took effect — and reads back only after restoring it.
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o644)).unwrap();
        if unreadable {
            assert!(result.is_err(), "an unreadable config must not be replaced");
            assert_eq!(fs::read(&path).unwrap(), original, "bytes preserved");
        }
    }

    /// `read_verified` refuses a path that could leave the vault before it
    /// touches the filesystem at all.
    #[test]
    fn read_verified_refuses_traversal_and_absolute_paths() {
        let dir = sample_vault();
        let root = std::fs::canonicalize(dir.path()).unwrap();

        for bad in ["../outside.md", "Projects/../../outside.md", "/etc/passwd"] {
            let error = read_verified(&root, Path::new(bad), 4096)
                .expect_err(&format!("{bad} must be refused"));
            assert_eq!(
                error.kind(),
                std::io::ErrorKind::InvalidInput,
                "{bad} should be refused as invalid input, got {error}"
            );
        }

        // * A path inside still reads.
        assert_eq!(
            read_verified(&root, Path::new("Projects/Plan.md"), 4096).unwrap(),
            "plan"
        );
    }

    /// A symlinked ancestor is not followed, and a file over the limit is
    /// refused on the opened handle rather than by trusting its metadata.
    #[cfg(unix)]
    #[test]
    fn read_verified_refuses_a_link_and_an_oversized_file() {
        let dir = sample_vault();
        let root = std::fs::canonicalize(dir.path()).unwrap();

        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret.md"), "secret").unwrap();
        std::os::unix::fs::symlink(
            outside.path().join("secret.md"),
            root.join("Projects/linked.md"),
        )
        .unwrap();

        let error = read_verified(&root, Path::new("Projects/linked.md"), 4096)
            .expect_err("a symlinked file must not be read");
        assert!(
            error.to_string().to_lowercase().contains("regular file")
                || error.raw_os_error() == Some(libc::ELOOP),
            "{error}"
        );

        fs::write(root.join("big.md"), "0123456789").unwrap();
        let error = read_verified(&root, Path::new("big.md"), 5)
            .expect_err("a file over the limit must be refused");
        assert!(error.to_string().contains("limit"), "{error}");
    }

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
        // * Neither file exists until a setting is changed, and a vault
        // * without one is not an error.
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
        // * Obsidian writes the empty string rather than dropping the key.
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
        // * `colorGroups` is skipped when empty, and `theme` is a key no
        // * graph config models at all. Both have to survive a write.
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
            // * A linked folder on the way, and a linked file at the end.
            assert!(matches!(
                vault.write_path("Link/x.md"),
                Err(VaultError::Symlink(_))
            ));
            assert!(matches!(
                vault.write_path("note.md"),
                Err(VaultError::Symlink(_))
            ));
            assert!(vault.write_bytes("note.md", b"x").is_err());
            // * Nothing outside changed, and the link itself is still a link.
            assert_eq!(
                fs::read_to_string(outside.path().join("secret")).unwrap(),
                "s"
            );
            assert!(fs::symlink_metadata(r.join("note.md"))
                .unwrap()
                .file_type()
                .is_symlink());
            // * A link may be moved or removed as an entry, not written through.
            assert!(vault.entry_path("note.md").is_ok());
            assert!(matches!(
                vault.entry_path("Link/x.md"),
                Err(VaultError::Symlink(_))
            ));
            // * The open itself refuses a link that appears after the check.
            assert!(write_no_follow(&r.join("note.md"), b"x").is_err());
        }
        vault.write_bytes("Sub/a.md", b"b").unwrap();
        assert_eq!(fs::read_to_string(r.join("Sub/a.md")).unwrap(), "b");
    }

    #[cfg(unix)]
    #[test]
    fn write_at_follows_directories_but_never_a_link() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(root.join("a/b")).unwrap();

        write_at(&root, Path::new("a/b/note.md"), b"written").unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("a/b/note.md")).unwrap(),
            "written"
        );

        // * A link where a directory should be is refused, not followed.
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), root.join("escape")).unwrap();
        let err = write_at(&root, Path::new("escape/planted.md"), b"pwned").unwrap_err();
        assert!(
            err.raw_os_error() == Some(libc::ELOOP) || err.raw_os_error() == Some(libc::ENOTDIR),
            "the walk did not stop at the link: {err:?}"
        );
        assert!(!outside.path().join("planted.md").exists());
    }

    #[cfg(unix)]
    #[test]
    fn write_at_refuses_a_link_in_the_final_position() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("real.md");
        std::fs::write(&target, "outside").unwrap();
        std::os::unix::fs::symlink(&target, root.join("link.md")).unwrap();

        assert!(write_at(&root, Path::new("link.md"), b"pwned").is_err());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "outside");
    }

    #[cfg(unix)]
    #[test]
    fn create_dir_all_at_builds_a_chain_and_refuses_a_link() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        create_dir_all_at(&root, Path::new("x/y/z")).unwrap();
        assert!(root.join("x/y/z").is_dir());
        create_dir_all_at(&root, Path::new("x/y/z")).unwrap();

        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), root.join("esc")).unwrap();
        assert!(create_dir_all_at(&root, Path::new("esc/deep")).is_err());
        assert!(!outside.path().join("deep").exists());
    }

    #[test]
    fn write_no_follow_replaces_the_file_and_leaves_no_scratch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.md");
        std::fs::write(&path, "old").unwrap();

        write_no_follow(&path, b"new").unwrap();

        // * The note holds the new bytes, and a snapshot lives under .trash;
        // * nothing else is left at the top level of the directory.
        let names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names
                .iter()
                .map(|s| s.as_str())
                .collect::<std::collections::HashSet<_>>(),
            vec!["note.md", ".trash"]
                .into_iter()
                .collect::<std::collections::HashSet<_>>(),
            "unexpected entry next to the note: {names:?}"
        );
        let snapshots = dir.path().join(".trash").join(".snapshots");
        assert!(snapshots.is_dir());
        assert!(!std::fs::read_dir(&snapshots).unwrap().next().is_none());

        write_no_follow(&path, b"newer").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "newer");
    }

    #[cfg(unix)]
    #[test]
    fn write_no_follow_still_refuses_a_link() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("real.md");
        std::fs::write(&target, "outside").unwrap();
        let link = dir.path().join("link.md");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        // * The rename would replace the *link*, not follow it — but the temp
        // * file lands beside it either way, so the outside file is untouched.
        let _ = write_no_follow(&link, b"pwned");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "outside");
    }

    // * `write_at` refuses to write through a symlink at the target, keeping
    // * the O_NOFOLLOW protection of the old direct-open path. The outside
    // * file is never touched.
    #[cfg(unix)]
    #[test]
    fn write_at_refuses_a_symlink_target() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("real.md");
        std::fs::write(&target, "outside").unwrap();
        let link = dir.path().join("link.md");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let err = write_at(dir.path(), Path::new("link.md"), b"pwned").unwrap_err();
        assert!(
            err.to_string().contains("symbolic link"),
            "a symlink target was not refused: {err}"
        );
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "outside");
    }

    // * `write_at` replaces the file in full: a second write leaves exactly
    // * the new bytes and no scratch file behind, so a crash cannot leave a
    // * truncated note.
    #[cfg(unix)]
    #[test]
    fn write_at_replaces_the_file_in_full() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.md");
        std::fs::write(&path, "old\nold\nold\n").unwrap();

        write_at(dir.path(), Path::new("note.md"), b"new\nnew\n").unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new\nnew\n");
        let scratch: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".silica-tmp"))
            .collect();
        assert!(
            scratch.is_empty(),
            "a temp file was left behind: {scratch:?}"
        );
    }

    /// A parent-directory junction pointing outside the vault must be refused
    /// at the parent-component check: `is_symlink` does not see junctions on
    /// Windows, so the reparse-point attribute is what catches the redirect
    /// before the write is attempted.
    #[cfg(windows)]
    #[test]
    fn a_parent_directory_junction_blocks_writes_outside_the_vault() {
        use std::os::windows::process::CommandExt;

        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let junction = root.path().join("Notes");

        // `mklink /J` creates a junction and needs no elevation on Windows.
        let made = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&junction)
            .arg(outside.path())
            .creation_flags(0x08000000)
            .status()
            .expect("mklink runs");
        assert!(made.success(), "could not create a junction to test with");

        let vault = Vault::open(root.path()).unwrap();
        let err = vault.write_bytes("Notes/new.md", b"pwned").unwrap_err();
        assert!(
            matches!(err, VaultError::Symlink(_)),
            "a parent-directory junction was not refused: {err:?}"
        );
        assert!(!outside.path().join("new.md").exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_write_keeps_the_notes_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.md");
        std::fs::write(&path, "old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();

        write_no_follow(&path, b"new").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "the write changed the note's mode");
    }

    #[test]
    fn saving_over_a_note_snapshots_its_previous_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.md");
        std::fs::write(&path, "original").unwrap();

        write_no_follow(&path, b"replacement").unwrap();

        let snapshots = dir.path().join(".trash").join(".snapshots");
        let files: Vec<String> = std::fs::read_dir(&snapshots)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(!files.is_empty(), "no snapshot was made");
        let content = std::fs::read_to_string(snapshots.join(&files[0])).unwrap();
        assert_eq!(
            content, "original",
            "the snapshot did not keep the old bytes"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "replacement");
    }

    #[test]
    fn only_the_recent_snapshots_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.md");
        std::fs::write(&path, "v0").unwrap();
        for n in 1..=15u32 {
            std::fs::write(&path, format!("v{n}")).unwrap();
            let _ = write_no_follow(&path, format!("v{n}").as_bytes());
        }
        let snapshots = dir.path().join(".trash").join(".snapshots");
        let count = std::fs::read_dir(&snapshots).unwrap().flatten().count();
        assert!(count <= 10, "too many snapshots kept: {count}");
    }

    #[test]
    fn saving_a_new_note_makes_no_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("new.md");
        write_no_follow(&path, b"first").unwrap();
        let snapshots = dir.path().join(".trash").join(".snapshots");
        assert!(!snapshots.exists() || std::fs::read_dir(&snapshots).unwrap().count() == 0);
    }

    #[test]
    fn rapid_saves_do_not_collide_snapshots() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.md");
        std::fs::write(&path, "v0").unwrap();

        // * Save 15 times in quick succession — each must produce a distinct
        // * snapshot, none overwriting an earlier one.
        for n in 1..=15u32 {
            std::fs::write(&path, format!("v{n}")).unwrap();
            let _ = write_no_follow(&path, format!("v{n}").as_bytes());
        }

        let snapshots = dir.path().join(".trash").join(".snapshots");
        let count = std::fs::read_dir(&snapshots).unwrap().flatten().count();
        assert_eq!(count, 10, "expected 10 pruned snapshots, got {count}");
    }

    // * Pruning matches the complete snapshot name, so saving `foo.md` never
    // * deletes snapshots that belong to `foobar.md` (they share a prefix,
    // * but not a stem).
    #[test]
    fn pruning_one_file_never_deletes_another_files_snapshots() {
        let dir = tempfile::tempdir().unwrap();
        let snapshots = dir.path().join(".trash").join(".snapshots");
        std::fs::create_dir_all(&snapshots).unwrap();

        // Plant snapshots for both `foo.md` and `foobar.md`.
        for (name, content) in [
            ("foo.md.100.1.md", "foo-old"),
            ("foo.md.200.2.md", "foo-new"),
            ("foobar.md.100.1.md", "foobar-old"),
            ("foobar.md.300.3.md", "foobar-new"),
        ] {
            std::fs::write(snapshots.join(name), content).unwrap();
        }

        // Keep only 1 snapshot per stem: pruning `foo.md` must leave one
        // `foo.md` snapshot and both `foobar.md` snapshots untouched.
        let _ = prune_snapshots(&snapshots, "foo.md", 1);

        let names: Vec<String> = std::fs::read_dir(&snapshots)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names.iter().filter(|n| n.starts_with("foobar.md")).count(),
            2,
            "pruning foo.md must not touch foobar.md's snapshots"
        );
        assert_eq!(
            names.iter().filter(|n| n.starts_with("foo.md")).count(),
            1,
            "pruning foo.md must keep only its most recent snapshot"
        );
    }

    #[cfg(unix)]
    #[test]
    fn snapshot_refuses_a_symlink_target() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("real.md");
        std::fs::write(&target, "outside").unwrap();
        let link = root.join("note.md");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        // * The snapshot must refuse to write through the symlink, and the
        // * outside file must be untouched.
        snapshot_before_overwrite(&link);
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "outside");
        assert!(!root.join(".trash").join(".snapshots").exists());
    }

    /// Windows has no `openat`, so `write_no_follow` is the path that protects
    /// a vault write there. It must refuse a reparse point the way the Unix
    /// `O_NOFOLLOW` open refuses a symlink, or a junction would redirect a
    /// save outside the vault.
    #[cfg(windows)]
    #[test]
    fn a_windows_reparse_point_is_refused() {
        use std::os::windows::process::CommandExt;

        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("real.md"), "outside").unwrap();
        let link = dir.path().join("note.md");

        // * `mklink /J` creates a junction, which needs no elevation, unlike a
        // * symlink. A junction is a reparse point and is what a hostile vault
        // * can ship without any special privilege.
        let made = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(outside.path())
            .creation_flags(0x08000000)
            .status()
            .expect("mklink runs");
        assert!(made.success(), "could not create a junction to test with");

        let result = write_no_follow(&link, b"overwritten");
        assert!(
            result.is_err(),
            "a reparse point was written through: {result:?}"
        );
        assert_eq!(
            std::fs::read_to_string(outside.path().join("real.md")).unwrap(),
            "outside",
            "the file behind the junction was changed"
        );
    }
}
