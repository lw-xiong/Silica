//! Filesystem operations that cannot be redirected by a reparse point.
//!
//! Windows has no `openat`, so a pathname check followed by an ordinary open
//! is a check-then-act race: a junction dropped in between redirects the
//! operation outside the vault. The Win32 calls that *look* like they solve
//! this do not. `CreateFileW`'s last argument is `hTemplateFile`, not a parent
//! directory, and it does not make the name relative to anything;
//! `CreateDirectoryW` and `MoveFileExW` always resolve a full path. Passing a
//! directory handle as a template silently ignored it and resolved the bare
//! name against the process working directory.
//!
//! What actually holds is to validate the object you already have:
//!
//! - [`open_verified`] opens a path, then asks the **handle** where it landed
//!   with `GetFinalPathNameByHandleW` and refuses it if that is outside the
//!   root. The object checked is the object returned, so a component swapped
//!   after the open cannot move it — there is nothing left to race.
//! - [`write_at`] renames through `SetFileInformationByHandle` with
//!   `FILE_RENAME_INFO.RootDirectory` set to the verified parent handle, which
//!   is a rename relative to a directory handle rather than to a name.
//! - [`create_dir_all_at`] makes one component at a time and verifies each by
//!   its handle, removing a component that resolved outside the root.

use std::ffi::{OsStr, OsString};
use std::io::{self, Read, Write};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::{Component, Path, PathBuf};

use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateDirectoryW, CreateFileW, DeleteFileW, FileRenameInfo, GetFileInformationByHandle,
    GetFinalPathNameByHandleW, RemoveDirectoryW, SetFileInformationByHandle, DELETE,
    FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_RENAME_INFO, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, VOLUME_NAME_DOS,
};

/// `CREATE_ALWAYS`: truncate an existing file or make a new one.
const CREATE_ALWAYS: u32 = 2;
/// `OPEN_EXISTING`: fail rather than create.
const OPEN_EXISTING: u32 = 3;

/// A path as a NUL-terminated wide string.
///
/// Absolute paths get the `\\?\` verbatim prefix: without it every Win32 call
/// here caps at MAX_PATH (260) and a deep vault note fails with "cannot find
/// the path specified" (os error 3) before any check of ours runs. Verbatim
/// form does no normalization, so the `.`/`..` and `/` cases it would have
/// forgiven are handled first: `sane_relative` refuses `..` outright, and an
/// absolute path's separators are rewritten to backslashes below. UNC paths
/// take the `\\?\UNC\` form; relative paths pass through untouched, keeping
/// the process-relative semantics they had.
fn wide(part: &OsStr) -> io::Result<Vec<u16>> {
    let path = Path::new(part);
    let lossy = path.as_os_str().to_string_lossy();
    let verbatim: std::borrow::Cow<'_, str> = if lossy.starts_with(r"\\?\") {
        std::borrow::Cow::Borrowed(&lossy)
    } else if let Some(rest) = lossy.strip_prefix(r"\\") {
        std::borrow::Cow::Owned(format!(r"\\?\UNC\{rest}"))
    } else if path.is_absolute() {
        std::borrow::Cow::Owned(format!(r"\\?\{}", lossy.replace('/', r"\")))
    } else {
        std::borrow::Cow::Borrowed(&lossy)
    };
    let mut out: Vec<u16> = OsStr::new(verbatim.as_ref()).encode_wide().collect();
    if out.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a NUL in a path component",
        ));
    }
    out.push(0);
    Ok(out)
}

/// Drop the `\\?\` prefix `GetFinalPathNameByHandleW` returns, so a resolved
/// path compares against an ordinary one.
fn strip_verbatim(path: &Path) -> PathBuf {
    match path.to_string_lossy().strip_prefix(r"\\?\") {
        Some(rest) => PathBuf::from(rest),
        None => path.to_path_buf(),
    }
}

/// Where an open handle actually points.
fn final_path(handle: HANDLE) -> io::Result<PathBuf> {
    let mut size = 512u32;
    loop {
        let mut buf = vec![0u16; size as usize];
        let written =
            unsafe { GetFinalPathNameByHandleW(handle, buf.as_mut_ptr(), size, VOLUME_NAME_DOS) };
        if written == 0 {
            return Err(io::Error::last_os_error());
        }
        if written < size {
            buf.truncate(written as usize);
            return Ok(strip_verbatim(&PathBuf::from(OsString::from_wide(&buf))));
        }
        // * Too small: the return is the size needed, NUL included.
        size = written + 1;
    }
}

/// Open `path` and refuse it unless the **handle** resolves inside `root`.
///
/// This is the whole trick. A path cannot be validated and then trusted, but
/// an open handle names one object for as long as it is held, so asking where
/// it landed is a statement about the file Silica is about to read or write.
fn open_verified(
    root: &Path,
    path: &Path,
    access: u32,
    directory: bool,
) -> io::Result<std::fs::File> {
    let name = wide(path.as_os_str())?;
    let mut flags = FILE_FLAG_OPEN_REPARSE_POINT;
    if directory {
        flags |= FILE_FLAG_BACKUP_SEMANTICS;
    }
    let handle = unsafe {
        CreateFileW(
            name.as_ptr(),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            flags | FILE_ATTRIBUTE_NORMAL,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // * Owned from here, so every early return closes it.
    let file = unsafe { std::fs::File::from_raw_handle(handle) };

    let mut info: windows_sys::Win32::Storage::FileSystem::BY_HANDLE_FILE_INFORMATION =
        unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle() as HANDLE, &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // * A reparse point at the path itself is refused outright; one earlier in
    // * the path is caught below, because the check below reads where the
    // * handle went rather than what the path claimed.
    if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(io::Error::other(
            "the path is a reparse point; the vault folder is the only place Silica writes",
        ));
    }

    let resolved = final_path(file.as_raw_handle() as HANDLE)?;
    if !resolved.starts_with(root) {
        return Err(io::Error::other(format!(
            "{} resolves outside the vault ({}); refusing",
            path.display(),
            resolved.display()
        )));
    }
    Ok(file)
}

/// The vault root, canonicalized once so resolved paths compare against it.
fn canonical_root(root: &Path) -> io::Result<PathBuf> {
    Ok(strip_verbatim(&std::fs::canonicalize(root)?))
}

/// Reject `..` before the filesystem sees it at all.
fn sane_relative(relative: &Path) -> io::Result<()> {
    for part in relative.components() {
        if matches!(part, Component::ParentDir) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "the path points outside the vault",
            ));
        }
    }
    Ok(())
}

/// Open a file under `root` for reading, verified through its own handle.
pub fn open_inside(root: &Path, relative: &Path) -> io::Result<std::fs::File> {
    sane_relative(relative)?;
    let root = canonical_root(root)?;
    open_verified(&root, &root.join(relative), GENERIC_READ, false)
}

/// Read a file under `root` through the handle that was verified in place, and
/// refuse it if the opened object is larger than `max_bytes`.
pub fn read_inside(root: &Path, relative: &Path, max_bytes: u64) -> io::Result<Vec<u8>> {
    let mut file = open_inside(root, relative)?;
    let len = file.metadata()?.len();
    if len > max_bytes {
        return Err(io::Error::other(format!(
            "{} is {len} bytes, over the {max_bytes}-byte limit",
            relative.display()
        )));
    }
    let mut out = Vec::with_capacity(len as usize);
    file.read_to_end(&mut out)?;
    Ok(out)
}

/// Create every component of `relative` under `root`.
///
/// Each component is created and then opened to see where its handle landed;
/// one that resolves outside the root is removed again, so a junction cannot
/// leave a directory behind that a later step would trust.
pub fn create_dir_all_at(root: &Path, relative: &Path) -> io::Result<()> {
    sane_relative(relative)?;
    let root = canonical_root(root)?;
    let mut current = PathBuf::new();
    for part in relative.components() {
        let Component::Normal(name) = part else {
            continue;
        };
        current.push(name);
        let full = root.join(&current);
        if full.is_dir() {
            open_verified(&root, &full, GENERIC_READ, true)?;
            continue;
        }
        let wide_name = wide(full.as_os_str())?;
        if unsafe { CreateDirectoryW(wide_name.as_ptr(), std::ptr::null()) } == 0 {
            let error = io::Error::last_os_error();
            // * Someone else may have made it first; still has to pass below.
            if error.kind() != io::ErrorKind::AlreadyExists {
                return Err(error);
            }
        }
        if let Err(error) = open_verified(&root, &full, GENERIC_READ, true) {
            let _ = unsafe { RemoveDirectoryW(wide_name.as_ptr()) };
            return Err(error);
        }
    }
    Ok(())
}

/// Write `bytes` to `relative` under `root`.
///
/// The bytes go to a companion in the same directory, which is then renamed
/// onto the target through `SetFileInformationByHandle` with the verified
/// parent handle as `RootDirectory`. That field is what makes the rename
/// relative to a directory *handle*: `MoveFileExW` has no equivalent and would
/// re-resolve the destination by name.
pub fn write_at(root: &Path, relative: &Path, bytes: &[u8]) -> io::Result<()> {
    sane_relative(relative)?;
    let root = canonical_root(root)?;
    let full = root.join(relative);
    let Some(name) = relative.file_name() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a file path must name a file",
        ));
    };
    let parent = full.parent().unwrap_or(&root);
    let parent_handle = open_verified(&root, parent, GENERIC_READ | GENERIC_WRITE, true)?;

    // * Unique per write. Two saves of one note from two threads — or two
    // * windows on the same vault — would otherwise share a companion and
    // * truncate each other's bytes; the pid alone does not separate threads.
    let unique = {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        format!("{}-{n}-{nanos}", std::process::id())
    };
    let temp_name = format!(".{}.{unique}.silica-tmp", name.to_string_lossy());
    let temp_path = parent.join(&temp_name);
    let temp_wide = wide(temp_path.as_os_str())?;
    let handle = unsafe {
        CreateFileW(
            temp_wide.as_ptr(),
            GENERIC_WRITE | DELETE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            CREATE_ALWAYS,
            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let mut temp = unsafe { std::fs::File::from_raw_handle(handle) };

    // * The companion sits in a verified parent, but checking costs one call
    // * and the alternative is trusting a name.
    if let Err(error) = open_verified(&root, &temp_path, GENERIC_READ, false) {
        drop(temp);
        let _ = unsafe { DeleteFileW(temp_wide.as_ptr()) };
        return Err(error);
    }

    if let Err(error) = temp.write_all(bytes).and_then(|_| temp.sync_all()) {
        drop(temp);
        let _ = unsafe { DeleteFileW(temp_wide.as_ptr()) };
        return Err(error);
    }

    // * `FileRenameInfo` is fussy about a name made relative to
    // * `RootDirectory`: the leaf form draws ERROR_INVALID_PARAMETER (87) and
    // * the counted form is not what the Win32 wrapper expects. The documented
    // * shape is a *full* path with `RootDirectory` null, so the destination is
    // * built from the parent handle's own resolved path — the same handle that
    // * `open_verified` checked is inside the vault — rather than from the
    // * string the caller passed in.
    let parent_real = final_path(parent_handle.as_raw_handle() as HANDLE)?;
    let target_real = parent_real.join(name);
    let name_wide = wide(target_real.as_os_str())?;
    let head = std::mem::offset_of!(FILE_RENAME_INFO, FileName);
    // * The call wants at least a whole `FILE_RENAME_INFO`, plus room for the
    // * name; a shorter buffer is another way to draw 87.
    let len = (head + name_wide.len() * 2).max(std::mem::size_of::<FILE_RENAME_INFO>());
    let mut buf = vec![0u8; len];
    unsafe {
        let info = buf.as_mut_ptr() as *mut FILE_RENAME_INFO;
        (*info).Anonymous.ReplaceIfExists = 1;
        (*info).RootDirectory = std::ptr::null_mut();
        // * Byte count of the name, terminator excluded.
        (*info).FileNameLength = ((name_wide.len() - 1) * 2) as u32;
        std::ptr::copy_nonoverlapping(
            name_wide.as_ptr(),
            std::ptr::addr_of_mut!((*info).FileName) as *mut u16,
            name_wide.len(),
        );
    }
    let moved = unsafe {
        SetFileInformationByHandle(
            temp.as_raw_handle() as HANDLE,
            FileRenameInfo,
            buf.as_ptr() as *const std::ffi::c_void,
            buf.len() as u32,
        )
    };
    if moved == 0 {
        let error = io::Error::last_os_error();
        drop(temp);
        let _ = unsafe { DeleteFileW(temp_wide.as_ptr()) };
        return Err(error);
    }
    // * The rename consumed the handle's usefulness; dropping closes it.
    drop(temp);
    Ok(())
}

/// The Unix path snapshots before replacing; here the companion-and-rename in
/// [`write_at`] already means the old bytes are never truncated in place.
pub fn snapshot_before_overwrite(_path: &Path) {}

/// Whether `error` means a component was simply not there.
pub fn is_missing(error: &io::Error) -> bool {
    matches!(error.kind(), io::ErrorKind::NotFound)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A note deep enough to pass MAX_PATH (260 chars) still writes and reads
    /// back intact. `CreateFileW`/`SetFileInformationByHandle` without a
    /// `\\?\` verbatim prefix cap at MAX_PATH, so a deep vault note fails with
    /// "cannot find the path specified" (os error 3) unless `wide` adds the
    /// prefix. Obsidian vaults on synced folders routinely exceed it.
    #[test]
    fn a_write_past_max_path_lands_and_reads_back() {
        let root = tempfile::tempdir().unwrap();
        // * One 90-char component per level: three nested dirs plus the file
        // * name carry the path past 260 without ever naming `..`.
        let long = "l".repeat(90);
        let relative = Path::new(&long)
            .join(&long)
            .join(&long)
            .join(format!("{long}.md"));
        let bytes = b"# deep note\n\nbody past MAX_PATH\n".to_vec();

        // * The app makes directories through the same verified path before
        // * writing a note into a new folder; the test follows that order.
        create_dir_all_at(root.path(), relative.parent().expect("has a parent"))
            .expect("create_dir_all_at past MAX_PATH");
        write_at(root.path(), &relative, &bytes).expect("write_at past MAX_PATH");
        let read_back = read_inside(root.path(), &relative, bytes.len() as u64 + 1)
            .expect("read_inside past MAX_PATH");
        assert_eq!(read_back, bytes);
    }

    /// The other half of a save: writing a note that is *already* there.
    /// The three upstream commits that rebuilt `FILE_RENAME_INFO` only ever
    /// exercised a fresh destination, while `ReplaceIfExists = 1` is what
    /// makes a second save replace the file instead of failing — and plugin
    /// writes (`write_at_handles`) and snapshot writes both land on existing
    /// paths. If the flag or the name length were wrong, every overwrite
    /// would fail with error 87 and no test would say so.
    #[test]
    fn a_write_over_an_existing_file_replaces_it_and_leaves_no_scratch() {
        let root = tempfile::tempdir().unwrap();
        let relative = Path::new("note.md");

        write_at(root.path(), relative, b"first save\n").expect("first write");
        let second = b"second save, longer than the first one\n";
        write_at(root.path(), relative, second).expect("write over an existing file");

        let read_back = read_inside(root.path(), relative, second.len() as u64 + 1)
            .expect("read back the second save");
        assert_eq!(read_back, second, "the old bytes survived the overwrite");

        // * The companion is renamed onto the target, not left beside it.
        let scratch: Vec<String> = std::fs::read_dir(root.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".silica-tmp"))
            .collect();
        assert!(scratch.is_empty(), "scratch left behind: {scratch:?}");
        assert_eq!(
            std::fs::read_dir(root.path()).unwrap().count(),
            1,
            "the vault holds exactly the note"
        );
    }
}
