//! End-to-end tests that drive the built `silica` binary over its control
//! socket: open a note, edit it, save it, reopen it, and verify the bytes
//! round-trip.
//!
//! These need a graphical session (Wayland or X11) to launch GPUI. They run by
//! default when a display exists; on a headless machine they skip instead of
//! failing, so a headless CI run is not broken by them.
//!
//! The flows covered: opening, editing and saving a note (above), and below
//! the shell's own overlays and their memory — the quick switcher, the
//! command palette, the search dock and the `workspace.json` layout record a
//! relaunch reads back.

use serde_json::{json, Value};
use silica_control::Client;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

/// The binary cargo builds for this package.
fn silica_bin() -> Option<PathBuf> {
    env::var("CARGO_BIN_EXE_silica").ok().map(PathBuf::from)
}

/// A running silica instance connected to its control socket.
struct App {
    _child: std::process::Child,
    socket: PathBuf,
    client: Client,
}

impl Drop for App {
    // ! A test that panics unwinds past every `app.call(...)` and would
    // ! otherwise leak the launched GUI process: on Windows there is no
    // ! kill-on-drop for `Child`, so four debug builds kept burning CPU for
    // ! minutes after a failed run (observed 2026-11-06). Killing in Drop
    // ! runs on panic unwind too, so a failing test leaves no orphan behind.
    fn drop(&mut self) {
        let _ = self._child.kill();
        let _ = self._child.wait();
    }
}

impl App {
    fn launch(vault: &Path) -> Option<Self> {
        let bin = silica_bin()?;
        let socket = vault.join("silica-e2e.sock");
        let _ = fs::remove_file(&socket);
        let log = vault.join("silica-e2e.log");
        let log_file = fs::File::create(&log).ok()?;
        let log_file_err = log_file.try_clone().ok()?;
        let mut child = Command::new(bin)
            .arg(vault)
            .arg("--control")
            .arg(&socket)
            .stdout(std::process::Stdio::null())
            // A launch that dies leaves its panic in this log instead of
            // vanishing, so a failing run says why.
            .stderr(std::process::Stdio::from(log_file_err))
            .spawn()
            .ok()?;
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            match Client::connect(&socket, Duration::from_millis(200)) {
                Ok(client) => {
                    let mut app = Self {
                        _child: child,
                        socket,
                        client,
                    };
                    // ! Typing goes to the focused window. On a bare X server
                    // ! (xvfb in CI) nothing is focused yet, so the keystrokes
                    // ! would be dropped and the buffer would stay empty.
                    let _ = app.client.call("window", json!({"action": "activate"}));
                    let _ = app.client.call("focus", json!({}));
                    return Some(app);
                }
                Err(_) => {
                    if child.try_wait().ok().flatten().is_some() {
                        // The process exited before opening the socket — most
                        // likely there is no graphical session to run GPUI in.
                        return None;
                    }
                    if Instant::now() >= deadline {
                        return None;
                    }
                    thread::sleep(Duration::from_millis(100));
                }
            }
        }
    }

    fn call(&mut self, method: &str, params: Value) -> Value {
        match self.client.call(method, params) {
            Ok(v) => v,
            Err(e) => panic!("{method} failed: {e}"),
        }
    }

    /// Wait until the vault is scanned and the view has painted quietly.
    fn settle(&mut self) {
        self.call("wait_idle", json!({"frames": 3, "timeout_ms": 20000}));
    }

    /// The open note's full text, polling for up to twenty seconds: the
    /// editor loads the file asynchronously after open, and a cold debug
    /// build can take a while to paint the first frame.
    fn text(&mut self) -> String {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let v = self.call("state", json!({"text": true}));
            let t = v
                .get("editor")
                .and_then(|e| e.get("text"))
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string();
            if !t.is_empty() || Instant::now() >= deadline {
                return t;
            }
            thread::sleep(Duration::from_millis(50));
        }
    }

    fn run_command(&mut self, id: &str) {
        self.call("command", json!({"id": id}));
    }

    /// Blocks until the vault has finished scanning and indexing. A search
    /// runs over the index, and a launch has not filled it yet, so one
    /// driven straight after `settle` can answer with nothing.
    fn loaded(&mut self) {
        self.call("wait_loaded", json!({"timeout_ms": 20000}));
    }

    /// The overlay now up — its kind, query, selected row and rows — from
    /// the state dump, so the switcher and the palette can be told apart
    /// from what they are showing.
    fn overlay(&mut self) -> Value {
        self.call("state", json!({}))
            .get("modal")
            .cloned()
            .unwrap_or(Value::Null)
    }

    /// What holds the keyboard now, from the state dump: a modal's field
    /// names itself, so a test can tell where typed text landed.
    fn focused(&mut self) -> Value {
        self.call("state", json!({}))
            .get("focus")
            .cloned()
            .unwrap_or(Value::Null)
    }

    /// The left dock's Search view, from the state dump.
    fn search_dock(&mut self) -> Value {
        self.call("state", json!({}))
            .pointer("/docks/left/search")
            .cloned()
            .unwrap_or(Value::Null)
    }

    /// How the main area is split into tab groups, from the state dump.
    fn groups(&mut self) -> Value {
        self.call("state", json!({}))
            .get("groups")
            .cloned()
            .unwrap_or(Value::Null)
    }

    /// Closes the app the way a user does. `Drop` kills the child, which
    /// never runs the quit path — and `workspace.json` is written on the
    /// way out, so a killed process leaves the record unwritten.
    fn quit(&mut self) {
        let reply = self.client.call("quit", json!({}));
        assert!(reply.is_ok(), "quit failed: {reply:?}");
        // The reply is written before the app exits, so waiting for the
        // child is what proves it is gone before the next launch binds the
        // same socket path.
        let deadline = Instant::now() + Duration::from_secs(20);
        while self._child.try_wait().ok().flatten().is_none() {
            assert!(Instant::now() < deadline, "silica did not exit after quit");
            thread::sleep(Duration::from_millis(50));
        }
    }
}

/// A scratch vault with one note, cleaned up when dropped.
struct Vault {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl Vault {
    fn new() -> Self {
        let _dir = tempfile::tempdir().expect("a temp dir");
        let root = _dir.path().to_path_buf();
        fs::create_dir_all(&root).expect("the vault folder");
        fs::write(root.join("Note.md"), "original\n").expect("a note");
        Self { _dir, root }
    }
}

/// One display, one app at a time: two graphical tests launching together
/// collide on the focus the keystrokes need, so every test holds this for
/// its whole body whatever harness flags are in play.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Polls the search dock until its result list answers `files`: the query
/// runs on the background executor, so the rows land a beat after the
/// keystrokes that asked for them.
fn wait_for_files(app: &mut App, files: u64) -> Value {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let dock = app.search_dock();
        let answer = dock.get("files").and_then(Value::as_u64).unwrap_or(0);
        if answer == files {
            return dock;
        }
        assert!(
            Instant::now() < deadline,
            "the search dock never answered with {files} files: {dock:?}"
        );
        thread::sleep(Duration::from_millis(100));
    }
}

/// Whether the graphical end-to-end tests should run.
///
/// They run by default whenever a display exists, so an ordinary `cargo test`
/// on a desktop covers the GUI workflows instead of silently skipping them.
/// `SILICA_E2E=0` forces them off, and a headless machine skips without
/// failing because there is no display to launch GPUI on.
fn e2e_enabled() -> bool {
    match env::var("SILICA_E2E").as_deref() {
        Ok("0") => return false,
        Ok("1") => return true,
        _ => {}
    }
    env::var_os("WAYLAND_DISPLAY").is_some() || env::var_os("DISPLAY").is_some()
}

#[test]
fn open_save_and_reopen_round_trips_bytes() {
    if !e2e_enabled() {
        eprintln!("skipped: no display available for the graphical end-to-end tests");
        return;
    }
    let _serial = serial();
    let vault = Vault::new();
    let mut app = match App::launch(&vault.root) {
        Some(a) => a,
        None => {
            eprintln!("skipped: no graphical session to launch silica in");
            return;
        }
    };
    app.settle();
    app.call("open_path", json!({"path": "Note.md"}));
    app.settle();
    assert_eq!(app.text(), "original\n", "the note opens with its bytes");

    // Edit through the real typing path, then save through the real save
    // path. Whatever the cursor does, the saved bytes must equal the buffer.
    app.call("focus", json!({"set": "editor"}));
    app.call("type", json!({"text": "appended\n"}));
    app.settle();
    let buffered = app.text();
    assert!(
        buffered.contains("original") && buffered.contains("appended"),
        "typing changed the buffer: {buffered:?}"
    );
    app.run_command("editor:save-file");
    app.settle();
    assert_eq!(app.text(), buffered, "the buffer holds the saved text");

    // The bytes on disk must be exactly what was saved — nothing more,
    // nothing less.
    let vault_root = app.socket.parent().expect("the vault folder").to_path_buf();
    let on_disk = fs::read_to_string(vault_root.join("Note.md")).expect("the saved note");
    assert_eq!(
        on_disk, buffered,
        "the file on disk is byte-for-byte the saved buffer"
    );

    // Save again with no change: still byte-identical.
    app.run_command("editor:save-file");
    app.settle();
    let again = fs::read_to_string(vault_root.join("Note.md")).expect("the note again");
    assert_eq!(again, buffered, "a no-op save does not drift the bytes");
}

#[test]
fn overwriting_a_note_snapshots_its_previous_bytes() {
    if !e2e_enabled() {
        eprintln!("skipped: no display available for the graphical end-to-end tests");
        return;
    }
    let _serial = serial();
    let vault = Vault::new();
    let mut app = match App::launch(&vault.root) {
        Some(a) => a,
        None => {
            eprintln!("skipped: no graphical session to launch silica in");
            return;
        }
    };
    app.settle();
    app.call("open_path", json!({"path": "Note.md"}));
    app.settle();
    assert_eq!(app.text(), "original\n");

    // Focus the editor first: on a bare X server (xvfb in CI) nothing has
    // keyboard focus after a window opens, so keystrokes would be dropped.
    app.call("focus", json!({"set": "editor"}));
    app.call("type", json!({"text": "first edit\n"}));
    app.settle();
    app.run_command("editor:save-file");
    app.settle();

    let vault_root = app.socket.parent().expect("the vault folder").to_path_buf();

    // Capture the bytes on disk before the next save, so we can recognize
    // them in the snapshot folder whatever the cursor did.
    let pre_second = fs::read_to_string(vault_root.join("Note.md")).expect("the note");

    app.call("type", json!({"text": "second edit\n"}));
    app.settle();
    app.run_command("editor:save-file");
    app.settle();

    let snapshots = vault_root.join(".trash").join(".snapshots");
    let files: Vec<String> = fs::read_dir(&snapshots)
        .expect("the snapshot folder exists")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(!files.is_empty(), "no snapshot was made on overwrite");

    // At least one snapshot must hold the bytes that were on disk before the
    // last save.
    let has_expected = files.iter().any(|name| {
        fs::read_to_string(snapshots.join(name))
            .map(|c| c == pre_second)
            .unwrap_or(false)
    });
    assert!(
        has_expected,
        "a snapshot holds the pre-save bytes: {files:?}"
    );

    // And the current file is the newest version.
    let on_disk = fs::read_to_string(vault_root.join("Note.md")).expect("the note");
    assert!(
        on_disk.contains("second edit"),
        "the newest bytes are on disk: {on_disk:?}"
    );
}

/// F2 on a focused tree row turns the row's own label into the field with
/// the name selected; typing replaces it and Enter takes it. The file on
/// disk gets the new name, and the note stays open under it.
#[test]
fn renaming_a_tree_row_renames_the_file_in_place() {
    if !e2e_enabled() {
        eprintln!("skipped: no display available for the graphical end-to-end tests");
        return;
    }
    let _serial = serial();
    let vault = Vault::new();
    fs::write(vault.root.join("Beta.md"), "beta\n").expect("a second note");
    let mut app = match App::launch(&vault.root) {
        Some(a) => a,
        None => {
            eprintln!("skipped: no graphical session to launch silica in");
            return;
        }
    };
    app.settle();
    app.call("open_path", json!({"path": "Beta.md"}));
    app.settle();
    // The tree, with the focus ring on the open file's row.
    app.run_command("file-explorer:open");
    app.settle();
    // F2 opens the field; typing goes through the control's IME-commit path
    // because the focused view is the field, not the editor.
    app.call("keys", json!({"keys": ["f2"]}));
    app.settle();
    app.call("type", json!({"text": "Gamma"}));
    app.settle();
    app.call("keys", json!({"keys": ["enter"]}));
    app.settle();

    let root = app.socket.parent().expect("the vault folder").to_path_buf();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if root.join("Gamma.md").exists() && !root.join("Beta.md").exists() {
            break;
        }
        if Instant::now() >= deadline {
            let listed: Vec<String> = fs::read_dir(&root)
                .expect("the vault folder")
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            panic!("the file was not renamed in place: {listed:?}");
        }
        thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(
        fs::read_to_string(root.join("Gamma.md")).expect("the renamed note"),
        "beta\n",
        "the renamed file keeps its bytes"
    );
    assert_eq!(
        app.text(),
        "beta\n",
        "the note stays open under its new name"
    );
}

/// A note opened while an editing mode is on screen builds no reading-view
/// model; the model is built on the way into Reading view, so a reader who
/// switches there meets the note fully rendered, not a blank pane.
#[test]
fn the_reading_model_is_built_on_the_way_into_reading_view() {
    if !e2e_enabled() {
        eprintln!("skipped: no display available for the graphical end-to-end tests");
        return;
    }
    let _serial = serial();
    let vault = Vault::new();
    let mut app = match App::launch(&vault.root) {
        Some(a) => a,
        None => {
            eprintln!("skipped: no graphical session to launch silica in");
            return;
        }
    };
    app.settle();
    app.call("open_path", json!({"path": "Note.md"}));
    app.settle();
    let blocks = |app: &mut App| {
        app.call("state", json!({}))
            .get("reading")
            .and_then(|r| r.get("blocks"))
            .and_then(|b| b.as_u64())
            .unwrap_or(0)
    };
    // * Opened in the default editing mode: no model was built.
    assert_eq!(blocks(&mut app), 0, "an editing mode built a reading model");
    app.run_command("markdown:toggle-preview");
    app.settle();
    assert!(blocks(&mut app) > 0, "Reading view shows no blocks");
}

#[test]
fn the_view_header_bookmark_action_prompts_for_a_title() {
    if !e2e_enabled() {
        eprintln!("skipped: no display available for the graphical end-to-end tests");
        return;
    }
    let _serial = serial();
    let vault = Vault::new();
    let mut app = match App::launch(&vault.root) {
        Some(a) => a,
        None => {
            eprintln!("skipped: no graphical session to launch silica in");
            return;
        }
    };
    app.settle();
    app.call("open_path", json!({"path": "Note.md"}));
    app.settle();
    app.call("window", json!({"action": "activate"}));
    app.settle();
    // Type a heading into the open note, then bookmark the current heading
    // through the command palette path — the same start_bookmark the view
    // header's bookmark button calls.
    app.call("focus", json!({"set": "editor"}));
    app.settle();
    app.call("type", json!({"text": "# My Heading\\n"}));
    app.settle();
    app.call(
        "command",
        json!({"id": "bookmarks:bookmark-current-heading"}),
    );
    app.settle();
    let modal = app
        .call("state", json!({}))
        .get("modal")
        .cloned()
        .unwrap_or(json!(null));
    assert_eq!(
        modal.get("kind").and_then(|k| k.as_str()),
        Some("prompt"),
        "the bookmark command opened no prompt: {modal:?}"
    );
    assert_eq!(
        modal
            .get("pending_op")
            .and_then(|p| p.get("op"))
            .and_then(|o| o.as_str()),
        Some("add-bookmark"),
        "{modal:?}"
    );
    let placeholder = modal
        .get("pending_op")
        .and_then(|p| p.get("placeholder"))
        .and_then(|p| p.as_str())
        .unwrap_or("");
    assert!(
        placeholder.contains("My Heading"),
        "the prompt bookmarks the current heading: {modal:?}"
    );
}

/// Quick Capture: the command opens the floating window, typing goes into
/// its field, Enter saves the note to the vault, and Escape closes the
/// window without writing anything. The window is driven through the
/// `window` param, because input dispatch is bound to the main window by
/// default and a second window is unreachable without naming it.
#[test]
fn quick_capture_saves_a_note_and_escape_closes_the_window() {
    if !e2e_enabled() {
        eprintln!("skipped: no display available for the graphical end-to-end tests");
        return;
    }
    let _serial = serial();
    let vault = Vault::new();
    let mut app = match App::launch(&vault.root) {
        Some(a) => a,
        None => {
            eprintln!("skipped: no graphical session to launch silica in");
            return;
        }
    };
    app.settle();
    app.run_command("quick-capture:open");
    app.settle();

    // The field holds the keyboard in the capture window, so the text
    // lands in it rather than in the editor behind it.
    app.call(
        "type",
        json!({"text": "Captured note", "window": "capture"}),
    );
    app.settle();
    app.call("keys", json!({"keys": ["enter"], "window": "capture"}));
    app.settle();

    let vault_root = app.socket.parent().expect("the vault folder").to_path_buf();
    assert_eq!(
        fs::read_to_string(vault_root.join("Captured note.md")).expect("the captured note"),
        "# Captured note\n",
        "Enter saved the field's text as a note"
    );

    // Escape closes the window: a dispatch into a closed window is the
    // error that proves it is gone, and nothing else was written.
    app.call("keys", json!({"keys": ["escape"], "window": "capture"}));
    app.settle();
    let after = app
        .client
        .call("keys", json!({"keys": ["enter"], "window": "capture"}));
    assert!(
        after.is_err(),
        "the capture window is still open after Escape: {after:?}"
    );
    let entries: Vec<String> = fs::read_dir(&vault_root)
        .expect("the vault folder")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        entries.iter().filter(|n| n.ends_with(".md")).count(),
        2,
        "Escape wrote nothing new: {entries:?}"
    );
}

/// The command run while the window is still open does not stack a second
/// capture box: typing still lands in the live window and saves, which is
/// what a stacked duplicate would break (the newest window holds the
/// keyboard, the older one keeps its stale field).
#[test]
fn a_second_capture_command_reuses_the_open_window() {
    if !e2e_enabled() {
        eprintln!("skipped: no display available for the graphical end-to-end tests");
        return;
    }
    let _serial = serial();
    let vault = Vault::new();
    let mut app = match App::launch(&vault.root) {
        Some(a) => a,
        None => {
            eprintln!("skipped: no graphical session to launch silica in");
            return;
        }
    };
    app.settle();
    app.run_command("quick-capture:open");
    app.settle();
    app.call(
        "type",
        json!({"text": "first capture", "window": "capture"}),
    );
    app.call("keys", json!({"keys": ["enter"], "window": "capture"}));
    app.settle();

    // The window stays open after a save — the hint says Escape closes, so
    // the second command is meant to raise it, not open a twin.
    app.run_command("quick-capture:open");
    app.settle();
    app.call(
        "type",
        json!({"text": "second capture", "window": "capture"}),
    );
    app.settle();
    app.call("keys", json!({"keys": ["enter"], "window": "capture"}));
    app.settle();

    let vault_root = app.socket.parent().expect("the vault folder").to_path_buf();
    assert_eq!(
        fs::read_to_string(vault_root.join("first capture.md")).expect("the first note"),
        "# first capture\n",
    );
    assert_eq!(
        fs::read_to_string(vault_root.join("second capture.md")).expect("the second note"),
        "# second capture\n",
        "the raised window still holds the keyboard: a stacked twin would not"
    );
}

/// Task Dashboard: the command opens the window, every task in the vault
/// is listed, and clicking a row opens its source note at the task's own
/// line. The line is what the click is for.
#[test]
fn the_task_dashboard_opens_a_clicked_task_at_its_own_line() {
    if !e2e_enabled() {
        eprintln!("skipped: no display available for the graphical end-to-end tests");
        return;
    }
    let _serial = serial();
    let vault = Vault::new();
    // * The task sits below the fold: a scroll to a line that is already
    //   visible cannot be told apart from no scroll at all.
    let mut note = String::from("# Tasks\n\n");
    for ix in 0..40 {
        note.push_str(&format!("filler line {ix}\n\n"));
    }
    note.push_str("- [ ] first task\n- [x] second task\n");
    fs::write(vault.root.join("Tasks.md"), &note).expect("a note with tasks");
    let mut app = match App::launch(&vault.root) {
        Some(a) => a,
        None => {
            eprintln!("skipped: no graphical session to launch silica in");
            return;
        }
    };
    app.settle();
    app.run_command("task-dashboard:open");
    app.settle();

    // * The scan lands on a background thread, so the rows can be a frame
    //   behind the window: wait for one to answer the keyboard rather than
    //   assuming the list is already filled. The arrows and Enter are the
    //   pointer-free path to a row, which is what keeps this test honest on
    //   a bare X server where a click's pixel offset means nothing.
    let mut opened = false;
    for _attempt in 0..5 {
        app.call("keys", json!({"keys": ["down"], "window": "dashboard"}));
        app.call("keys", json!({"keys": ["enter"], "window": "dashboard"}));
        app.settle();
        let current = app
            .call("state", json!({}))
            .get("file")
            .and_then(|f| f.get("path"))
            .and_then(|p| p.as_str())
            .unwrap_or_default()
            .to_string();
        if current == "Tasks.md" {
            opened = true;
            break;
        }
    }
    assert!(
        opened,
        "no dashboard row opened its note: the keys never landed"
    );

    // The note opened, and the task's own line is on screen: the row's
    // click is meant to leave the reader looking at the task, which is the
    // visible range — whether the viewport had to scroll at all depends on
    // how tall the window is, which is not what is being asserted.
    let state = app.call("state", json!({"text": true}));
    let current = state
        .get("file")
        .and_then(|f| f.get("path"))
        .and_then(|p| p.as_str())
        .unwrap_or_default()
        .to_string();
    assert_eq!(current, "Tasks.md", "the keys opened no note: {state:?}");
    // The caret is on the task's own line: reveal_offset puts it there and
    // brings it into view on the next paint.
    let task_line = state
        .get("editor")
        .and_then(|e| e.get("text"))
        .and_then(|t| t.as_str())
        .and_then(|text| text.lines().position(|l| l.contains("first task")))
        .map(|ix| ix as u64)
        .unwrap_or(u64::MAX);
    let caret = state
        .get("editor")
        .and_then(|e| e.get("cursor"))
        .and_then(|c| c.get("line"))
        .and_then(|l| l.as_u64())
        .unwrap_or(u64::MAX);
    assert_eq!(
        caret, task_line,
        "the caret is not on the task's line: {state:?}"
    );
}

/// The graph export writes inside the vault only: a socket caller naming a
/// path outside it gets an error, not picture bytes over an arbitrary file.
#[test]
fn a_graph_export_cannot_leave_the_vault() {
    if !e2e_enabled() {
        eprintln!("skipped: no display available for the graphical end-to-end tests");
        return;
    }
    let _serial = serial();
    let vault = Vault::new();
    let mut app = match App::launch(&vault.root) {
        Some(a) => a,
        None => {
            eprintln!("skipped: no graphical session to launch silica in");
            return;
        }
    };
    app.settle();
    let vault_root = app.socket.parent().expect("the vault folder").to_path_buf();
    let escaped = vault_root
        .parent()
        .expect("the temp folder outside the vault")
        .join("escaped-graph.png");

    // * The raw client call, not `app.call`: an error is the answer being
    //   asserted here, and `app.call` turns one into a panic.
    let refused = app.client.call("graph_png", json!({"path": escaped}));
    assert!(
        refused.is_err(),
        "an export naming a path outside the vault must be refused: {refused:?}"
    );
    assert!(
        !escaped.exists(),
        "the refused export still wrote the file: {escaped:?}"
    );
}

/// The quick switcher opens the note a query matches: the overlay is the
/// switcher, its field holds the keyboard, the query narrows the rows to
/// that one note, and Enter opens it and takes the overlay down.
#[test]
fn the_quick_switcher_opens_a_note_from_a_query() {
    if !e2e_enabled() {
        eprintln!("skipped: no display available for the graphical end-to-end tests");
        return;
    }
    let _serial = serial();
    let vault = Vault::new();
    fs::write(vault.root.join("Travel.md"), "flight times\n").expect("a second note");
    let mut app = match App::launch(&vault.root) {
        Some(a) => a,
        None => {
            eprintln!("skipped: no graphical session to launch silica in");
            return;
        }
    };
    app.settle();
    app.run_command("switcher:open");
    app.settle();
    let overlay = app.overlay();
    assert_eq!(
        overlay["kind"], "switcher",
        "the switcher did not open: {overlay:?}"
    );
    let focus = app.focused();
    assert_eq!(
        focus["name"], "switcher-input",
        "the switcher's field does not hold the keyboard: {focus:?}"
    );

    // The query goes into the field, which narrows the rows: a term one
    // note matches must leave that note and nothing else.
    app.call("type", json!({"text": "Travel"}));
    app.settle();
    let overlay = app.overlay();
    assert_eq!(
        overlay["query"], "Travel",
        "the query never reached the field: {overlay:?}"
    );
    assert_eq!(
        overlay["rows"],
        json!(["Travel"]),
        "the rows are not the note the query matches: {overlay:?}"
    );

    // Enter opens the selected row and closes the overlay.
    app.call("keys", json!({"keys": ["enter"]}));
    app.settle();
    let state = app.call("state", json!({"text": true}));
    assert_eq!(
        state["file"]["path"], "Travel.md",
        "Enter opened no note: {state:?}"
    );
    assert_eq!(
        state["modal"]["kind"], "none",
        "the switcher stayed open after Enter: {state:?}"
    );
    assert_eq!(
        app.text(),
        "flight times\n",
        "the note opened is not the one the query matched"
    );
}

/// The command palette runs the command its query selects: the palette
/// takes the keyboard, the query puts one command on top, and Enter runs
/// it — which is the open note switching to Reading view, what
/// "Toggle reading view" does.
#[test]
fn the_command_palette_runs_the_command_its_query_selects() {
    if !e2e_enabled() {
        eprintln!("skipped: no display available for the graphical end-to-end tests");
        return;
    }
    let _serial = serial();
    let vault = Vault::new();
    let mut app = match App::launch(&vault.root) {
        Some(a) => a,
        None => {
            eprintln!("skipped: no graphical session to launch silica in");
            return;
        }
    };
    app.settle();
    app.call("open_path", json!({"path": "Note.md"}));
    app.settle();
    app.run_command("command-palette:open");
    app.settle();
    let overlay = app.overlay();
    assert_eq!(
        overlay["kind"], "palette",
        "the palette did not open: {overlay:?}"
    );
    let focus = app.focused();
    assert_eq!(
        focus["name"], "palette-input",
        "the palette's field does not hold the keyboard: {focus:?}"
    );

    app.call("type", json!({"text": "Toggle reading view"}));
    app.settle();
    let overlay = app.overlay();
    assert_eq!(
        overlay["query"], "Toggle reading view",
        "the query never reached the field: {overlay:?}"
    );
    let selected = overlay["selected"].as_u64().expect("a selected row") as usize;
    let rows = overlay["rows"].as_array().expect("rows");
    assert_eq!(
        rows.get(selected).and_then(Value::as_str),
        Some("Toggle reading view"),
        "the query did not put the command on top: {overlay:?}"
    );

    app.call("keys", json!({"keys": ["enter"]}));
    app.settle();
    let state = app.call("state", json!({}));
    assert_eq!(
        state["modal"]["kind"], "none",
        "the palette stayed open after Enter: {state:?}"
    );
    assert_eq!(
        state["file"]["mode"], "reading",
        "Enter ran no command: {state:?}"
    );
}

/// The search dock filters the vault as the query is typed: the dock takes
/// the keyboard, a term one note holds answers with that note alone, and
/// the other note's term answers with it instead.
#[test]
fn the_search_dock_filters_the_vault_by_the_query() {
    if !e2e_enabled() {
        eprintln!("skipped: no display available for the graphical end-to-end tests");
        return;
    }
    let _serial = serial();
    let vault = Vault::new();
    fs::write(vault.root.join("Alpha.md"), "the alpha text\n").expect("a second note");
    fs::write(vault.root.join("Beta.md"), "the beta text\n").expect("a third note");
    let mut app = match App::launch(&vault.root) {
        Some(a) => a,
        None => {
            eprintln!("skipped: no graphical session to launch silica in");
            return;
        }
    };
    app.settle();
    app.run_command("global-search:open");
    app.settle();
    let state = app.call("state", json!({}));
    assert_eq!(
        state["docks"]["left"]["tab"], "search",
        "the dock did not open on the Search tab: {state:?}"
    );
    let focus = app.focused();
    assert_eq!(
        focus["name"], "search-input",
        "the dock's field does not hold the keyboard: {focus:?}"
    );

    // The index has to be there before the query runs over it.
    app.loaded();
    app.call("type", json!({"text": "alpha"}));
    let dock = wait_for_files(&mut app, 1);
    assert_eq!(
        dock["query"], "alpha",
        "the query never reached the field: {dock:?}"
    );
    assert_eq!(
        dock["files"], 1,
        "the query did not narrow the dock to one note: {dock:?}"
    );
    assert_eq!(
        dock["results"][0]["path"], "Alpha.md",
        "the note left is not the one holding the term: {dock:?}"
    );

    // Back to an empty field, then the other note's term: the dock answers
    // with that note instead, which is the filter at work rather than a
    // list that happens to hold one row.
    app.call(
        "keys",
        json!({"keys": ["backspace", "backspace", "backspace", "backspace", "backspace"]}),
    );
    let dock = wait_for_files(&mut app, 0);
    assert_eq!(
        dock["query"], "",
        "backspace did not empty the field: {dock:?}"
    );
    app.call("type", json!({"text": "beta"}));
    let dock = wait_for_files(&mut app, 1);
    assert_eq!(
        dock["files"], 1,
        "the second term did not narrow the dock: {dock:?}"
    );
    assert_eq!(
        dock["results"][0]["path"], "Beta.md",
        "the note left is not the one holding the second term: {dock:?}"
    );
}

/// A layout change reaches `workspace.json` and comes back: a split makes a
/// second group, closing the app writes the record, and a relaunch reads it
/// back as the split it was rather than the single group a fresh vault
/// opens with.
#[test]
fn a_split_layout_comes_back_from_workspace_json_on_relaunch() {
    if !e2e_enabled() {
        eprintln!("skipped: no display available for the graphical end-to-end tests");
        return;
    }
    let _serial = serial();
    let vault = Vault::new();
    let mut app = match App::launch(&vault.root) {
        Some(a) => a,
        None => {
            eprintln!("skipped: no graphical session to launch silica in");
            return;
        }
    };
    app.settle();
    app.loaded();
    app.call("open_path", json!({"path": "Note.md"}));
    app.settle();
    app.run_command("workspace:split-vertical");
    app.settle();
    let groups = app.groups();
    assert_eq!(
        groups["items"].as_array().map(Vec::len),
        Some(2),
        "the split made no second group: {groups:?}"
    );
    assert_eq!(
        groups["layout"]["direction"], "vertical",
        "the split is not the one asked for: {groups:?}"
    );
    assert_eq!(
        groups["layout"]["children"].as_array().map(Vec::len),
        Some(2),
        "the split has no two leaves: {groups:?}"
    );

    // Quitting is what writes `workspace.json`: killing the process would
    // leave the record unwritten.
    app.quit();

    let saved = fs::read_to_string(vault.root.join(".obsidian").join("workspace.json"))
        .expect("workspace.json was written on the way out");
    let record: Value = serde_json::from_str(&saved).expect("the layout record is JSON");
    assert_eq!(
        record["main"]["type"], "split",
        "the record holds no split: {record:?}"
    );
    assert_eq!(
        record["main"]["direction"], "vertical",
        "the record's split is not the one asked for: {record:?}"
    );
    assert_eq!(
        record["main"]["children"].as_array().map(Vec::len),
        Some(2),
        "the record's split has no two leaves: {record:?}"
    );

    // The same vault, launched again: the split comes back off the record.
    let mut app = match App::launch(&vault.root) {
        Some(a) => a,
        None => {
            eprintln!("skipped: no graphical session to launch silica in");
            return;
        }
    };
    app.settle();
    let groups = app.groups();
    assert_eq!(
        groups["items"].as_array().map(Vec::len),
        Some(2),
        "the relaunch restored no split: {groups:?}"
    );
    assert_eq!(
        groups["layout"]["children"].as_array().map(Vec::len),
        Some(2),
        "the relaunch's layout has no two leaves: {groups:?}"
    );
    let state = app.call("state", json!({"text": true}));
    assert_eq!(
        state["file"]["path"], "Note.md",
        "the relaunch opened no note: {state:?}"
    );
    assert_eq!(app.text(), "original\n", "the restored note lost its bytes");
}
