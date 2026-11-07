//! The compatibility host: the JavaScript runtime one tier-1 plugin runs in.
//!
//! One `Runtime` and one `Context` per plugin. A QuickJS context is not `Send`,
//! so the host is driven from wherever it was created and never moved; the
//! runtime is capped and every load and every call runs under a deadline, so a
//! plugin that loops cannot take the app with it.
//!
//! The `obsidian` module a plugin sees is `shim.js`. Anything that touches the
//! file system or the parse index comes back through the `__silica_raw_*`
//! globals installed here — reads answered from `silica-cache`, writes going
//! through the same path checks the app uses.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use rquickjs::{Context, Ctx, Exception, Function, Runtime};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use silica_cache::CachedMetadata;
use silica_formats::config::PluginManifest;

const SHIM: &str = include_str!("shim.js");

/// Creates `dir` and its parents by walking from the vault root with
/// `openat`, so a component substituted for a link between the path check and
/// the `mkdir` cannot redirect it.
fn make_dirs_checked(shared: &Rc<RefCell<Shared>>, dir: &Path) -> std::io::Result<()> {
    let shared = shared.borrow();
    let relative = dir.strip_prefix(&shared.root).unwrap_or(dir);
    silica_vault::create_dir_all_at(&shared.root, relative)
}

/// Byte budget for one plugin's heap. Exceeding it stops the plugin with an
/// error instead of letting it grow without bound inside the app.
const HEAP_BYTES: usize = 64 * 1024 * 1024;

/// How long a plugin may take to load. Bundles are big but loading is not
/// supposed to compute anything.
pub const LOAD_DEADLINE: Duration = Duration::from_secs(5);

/// How long one command may run before the host stops it.
pub const CALL_DEADLINE: Duration = Duration::from_secs(10);

/// A command a plugin added, as the app needs it to list and run it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandRegistration {
    pub id: String,
    pub name: String,
    /// Key of the callback held in the plugin's context; `None` for a command
    /// registered with no callback, which cannot be run.
    #[serde(default)]
    pub callback: Option<String>,
}

/// A ribbon item a plugin added.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RibbonRegistration {
    pub icon: String,
    pub title: String,
    #[serde(default)]
    pub callback: Option<String>,
}

/// One row of a plugin's settings tab, in the shape the app draws natively.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SettingRow {
    /// `setting` for a Setting, `heading` or `text` for the elements a tab
    /// builds directly.
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub desc: String,
    /// `none`, `toggle`, `text`, `dropdown` or `button`.
    #[serde(default)]
    pub control: String,
    #[serde(default)]
    pub value: Value,
    #[serde(default)]
    pub options: Vec<SettingOption>,
    #[serde(default)]
    pub placeholder: Option<String>,
    #[serde(default)]
    pub disabled: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SettingOption {
    pub value: String,
    pub label: String,
}

/// A settings tab a plugin added, with the rows its own `display()` produced.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct TabRegistration {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub rows: Vec<SettingRow>,
}

/// A code block processor a plugin registered, by language.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CodeBlockRegistration {
    pub language: String,
    #[serde(default)]
    pub callback: Option<String>,
    #[serde(default)]
    pub order: i64,
}

/// A markdown post-processor a plugin registered, by the callback key it
/// announced and the order it asked to run in.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PostProcessorRegistration {
    pub order: i64,
    #[serde(default)]
    pub callback: Option<String>,
}

/// What a plugin registered while loading. Empty plus a `failure` means the
/// plugin did not load, which is what the app reports as the reason.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Registrations {
    #[serde(default)]
    pub commands: Vec<CommandRegistration>,
    #[serde(default)]
    pub ribbon: Vec<RibbonRegistration>,
    #[serde(default)]
    pub tabs: Vec<TabRegistration>,
    #[serde(default)]
    pub code_blocks: Vec<CodeBlockRegistration>,
    #[serde(default)]
    pub post_processors: Vec<PostProcessorRegistration>,
    #[serde(default)]
    pub failure: Option<String>,
}

/// A vault file as a plugin sees it: path, name, size, mtime.
#[derive(Debug, Clone, PartialEq)]
pub struct FileEntry {
    path: String,
    name: String,
    basename: String,
    extension: String,
    size: u64,
    mtime_ms: i64,
}

/// State the JS closures reach back into. Shared by handle because every
/// closure the shim uses lives on the same thread as the host.
struct Shared {
    root: PathBuf,
    data_path: PathBuf,
    /// The plugin's own id, as the vault knows it.
    id: String,
    /// The vault's files, re-walked if anything has changed since the last
    /// read. When `shared_files` is set, this mirrors it instead of walking
    /// the directory tree again — one walk serves every plugin.
    files: Vec<FileEntry>,
    /// Set by anything that changes the vault, so the next read of the file
    /// list walks it again instead of answering from a stale snapshot.
    files_stale: bool,
    /// One file list shared across every compatibility plugin, so N plugins
    /// do not walk the vault N times. Set by the loader before each plugin
    /// starts; `files()` returns from it instead of walking.
    shared_files: Option<Rc<RefCell<Vec<FileEntry>>>>,
    notices: Vec<String>,
    active_file: Option<String>,
    /// The note open in the editor. Writes to it are refused: a plugin writing
    /// under an editor with unsaved changes would lose them.
    open_note: Option<String>,
    /// Post-processors the plugin registered, in the order it asked for them.
    post_processors: Vec<(i64, String)>,
    /// Timers the plugin scheduled through `setTimeout`/`setInterval`, keyed
    /// by the id handed back to JS. A slot is `None` once cancelled or, for
    /// one-shot timers, once fired.
    timers: Vec<Option<Timer>>,
}

/// One scheduled callback: the bound key to run, when it is due, and its
/// repeat period (`None` for a one-shot `setTimeout`).
struct Timer {
    key: String,
    at: Instant,
    period: Option<Duration>,
}

impl Shared {
    /// Registers a timer and returns its id. `delay_ms` clamps to zero, so a
    /// plugin that passes a negative or NaN delay gets an immediate run
    /// rather than an error.
    fn add_timer(&mut self, delay_ms: f64, repeat: bool, key: String) -> u32 {
        let delay = if delay_ms.is_finite() && delay_ms > 0.0 {
            Duration::from_secs_f64(delay_ms / 1000.0)
        } else {
            Duration::ZERO
        };
        let timer = Timer {
            key,
            at: Instant::now() + delay,
            period: repeat.then_some(delay),
        };
        // * Reuse cancelled slots so the id space stays small.
        if let Some(slot) = self.timers.iter().position(|t| t.is_none()) {
            self.timers[slot] = Some(timer);
            return slot as u32;
        }
        self.timers.push(Some(timer));
        (self.timers.len() - 1) as u32
    }

    fn cancel_timer(&mut self, id: u32) {
        if let Some(slot) = self.timers.get_mut(id as usize) {
            *slot = None;
        }
    }

    /// The bound keys of timers that are due now, rescheduling the repeating
    /// ones. Called with the borrow held briefly; the callbacks themselves
    /// run through the context afterwards.
    fn due_timers(&mut self) -> Vec<String> {
        let now = Instant::now();
        let mut due = Vec::new();
        for slot in self.timers.iter_mut() {
            let Some(timer) = slot else { continue };
            if timer.at > now {
                continue;
            }
            due.push(timer.key.clone());
            match timer.period {
                Some(period) => {
                    timer.at = now + period;
                }
                None => *slot = None,
            }
        }
        due
    }
}

impl Shared {
    /// The vault's files, re-walked if anything has changed since the last
    /// read. Ponytail: the walk is the honest source while the plugin crate
    /// has no handle on the app's index; for a vault large enough for that to
    /// matter, the app should hand its index over instead.
    fn files(&mut self) -> &[FileEntry] {
        // * One walk serves every plugin: if a shared file list was handed in,
        //   mirror it instead of walking the vault again.
        if let Some(shared) = &self.shared_files {
            let borrowed = shared.borrow();
            if self.files != *borrowed {
                self.files = borrowed.clone();
            }
            return &self.files;
        }
        if self.files_stale {
            self.files = collect_files(&self.root.clone());
            self.files_stale = false;
        }
        &self.files
    }

    fn mark_changed(&mut self) {
        self.files_stale = true;
    }

    /// Vault-relative path to a real path inside the vault, or the reason it
    /// is not one. Traversal out of the vault and writes over the open note
    /// are refused here rather than by each caller.
    fn resolve(&self, path: &str) -> Result<PathBuf, String> {
        let cleaned = path.replace('\\', "/");
        let trimmed = cleaned.trim_start_matches('/');
        if trimmed.is_empty() {
            return Err("an empty path was given".into());
        }
        let mut out = self.root.clone();
        for part in trimmed.split('/') {
            match part {
                "" | "." => continue,
                ".." => return Err(format!("'{path}' points outside the vault")),
                part => out.push(part),
            }
        }
        if !out.starts_with(&self.root) {
            return Err(format!("'{path}' points outside the vault"));
        }
        // ! A lexical check cannot see a symbolic link: `notes/plan.md` is
        // ! inside the vault as a string, and the `notes` beside it may point
        // ! anywhere. Every component is inspected instead, so a link anywhere
        // ! along the way is refused rather than followed.
        let mut walked = self.root.clone();
        for part in trimmed.split('/').filter(|p| !p.is_empty() && *p != ".") {
            walked.push(part);
            match std::fs::symlink_metadata(&walked) {
                Ok(meta) if meta.file_type().is_symlink() => {
                    return Err(format!(
                        "'{path}' goes through a symbolic link, which a plugin may not follow"
                    ));
                }
                // * A component that is not there yet is the normal case for a
                // * file a plugin is about to create; the rest of the walk
                // * cannot exist either.
                Err(_) => break,
                _ => {}
            }
        }
        Ok(out)
    }

    fn writable(&self, path: &str) -> Result<PathBuf, String> {
        let resolved = self.resolve(path)?;
        let relative = resolved
            .strip_prefix(&self.root)
            .unwrap_or(&resolved)
            .to_string_lossy()
            .replace('\\', "/");
        // ! Both paths are canonicalised before comparing, because the editor's
        // ! idea of the open note and the plugin's idea of the file it is about
        // ! to write are two strings that can name one file: a case-insensitive
        // ! filesystem folds them, and a resolved link can too. Comparison
        // ! ignores case as well, which over-refuses on a case-sensitive
        // ! filesystem — the safe direction, since the alternative is losing
        // ! unsaved edits.
        let open = self.open_note.as_deref().map(|note| {
            self.root
                .join(note)
                .canonicalize()
                .unwrap_or_else(|_| self.root.join(note))
        });
        let same = match open {
            Some(open) => {
                let target = resolved.canonicalize().unwrap_or_else(|_| resolved.clone());
                target == open
                    || target.to_string_lossy().to_lowercase()
                        == open.to_string_lossy().to_lowercase()
            }
            None => false,
        };
        if same {
            return Err(format!(
                "'{relative}' is open in the editor; a plugin cannot write it while its unsaved changes are here"
            ));
        }
        Ok(resolved)
    }

    /// Refuses an operation whose target is the open note **or a folder that
    /// contains it**.
    ///
    /// [`Self::writable`] compares whole paths, which answers "is this the
    /// note" but not "is this the note's folder". Deleting or renaming a
    /// folder moves every note under it, and one of those may have unsaved
    /// edits the editor still holds.
    fn writable_tree(&self, path: &str) -> Result<PathBuf, String> {
        let resolved = self.writable(path)?;
        if let Some(note) = self.open_note.as_deref() {
            let note = self.root.join(note);
            // * Same fold as `writable`: the two strings can name one path.
            let fold = |p: &Path| p.to_string_lossy().to_lowercase();
            let (target, note) = match (resolved.canonicalize(), note.canonicalize()) {
                (Ok(target), Ok(note)) => (target, note),
                _ => (resolved.clone(), note),
            };
            let (target, note) = (fold(&target), fold(&note));
            let inside =
                note.starts_with(&format!("{target}/")) || note.starts_with(&format!("{target}\\"));
            if inside {
                return Err(format!(
                    "'{path}' contains the note open in the editor; a plugin cannot move it while its unsaved changes are here"
                ));
            }
        }
        Ok(resolved)
    }

    /// Writes `bytes` to `resolved`, going through the vault's `openat` walk
    /// so a directory cannot be swapped for a link between the check that
    /// produced `resolved` and the write itself. The existing file is
    /// snapshotted before it is replaced, so a plugin write cannot destroy
    /// prior content if the process crashes mid-write.
    ///
    /// Windows has no `openat`; the same guarantee comes from walking the
    /// path through directory handles there, which is what
    /// [`silica_vault::write_at_handles`] does.
    fn write_checked(&self, resolved: &Path, bytes: &[u8]) -> std::io::Result<()> {
        let relative = resolved.strip_prefix(&self.root).unwrap_or(resolved);
        #[cfg(unix)]
        {
            // * Snapshot the existing file before replacing it: the atomic
            //   rename below never truncates in place, but the snapshot keeps
            //   the previous bytes recoverable.
            if resolved.exists() {
                silica_vault::snapshot_before_overwrite(resolved);
            }
            silica_vault::write_at(&self.root, relative, bytes)
        }
        #[cfg(windows)]
        {
            // * Same snapshot as Unix: `write_at_handles` replaces the file
            //   whole and leaves no way back otherwise, and a plugin writing
            //   an existing note is the normal case, not the edge one.
            if resolved.exists() {
                silica_vault::snapshot_before_overwrite(resolved);
            }
            silica_vault::write_at_handles(&self.root, relative, bytes)
        }
        #[cfg(all(not(unix), not(windows)))]
        {
            silica_vault::write_no_follow(resolved, bytes)
        }
    }

    fn entry(&mut self, path: &str) -> Option<String> {
        let cleaned = path.replace('\\', "/");
        let trimmed = cleaned.trim_start_matches('/').to_string();
        let found = self
            .files()
            .iter()
            .find(|f| f.path.eq_ignore_ascii_case(&trimmed))
            .cloned();
        found.map(|f| {
            json!({
                "path": f.path,
                "name": f.name,
                "basename": f.basename,
                "extension": f.extension,
                "size": f.size,
                "mtime": f.mtime_ms,
            })
            .to_string()
        })
    }
}

/// One loaded plugin: its runtime, its registrations, and the state its
/// closures write into.
pub struct PluginHost {
    /// Kept alive for as long as the context is; never read directly.
    _runtime: Runtime,
    context: Context,
    shared: Rc<RefCell<Shared>>,
    /// When the running script is to be stopped. The runtime's interrupt
    /// handler reads it, so one handler serves both loading and calls.
    deadline: Rc<Cell<Instant>>,
    registrations: Registrations,
}

impl PluginHost {
    /// Loads one plugin. A plugin that cannot be loaded comes back with
    /// `registrations().failure` set rather than as an error: a broken plugin
    /// is a reason, not a crash.
    pub fn load(
        vault: &Path,
        plugin_dir: &Path,
        manifest: &PluginManifest,
        open_note: Option<String>,
        shared_files: Option<Rc<RefCell<Vec<FileEntry>>>>,
    ) -> PluginHost {
        let id = if manifest.id.is_empty() {
            plugin_dir
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default()
        } else {
            manifest.id.clone()
        };
        // * Without a shared list, the first files() call must walk the vault.
        let files_stale = shared_files.is_none();
        let shared = Rc::new(RefCell::new(Shared {
            id,
            root: vault.to_path_buf(),
            data_path: plugin_dir.join("data.json"),
            files: Vec::new(),
            files_stale,
            shared_files,
            notices: Vec::new(),
            active_file: open_note.clone(),
            open_note,
            post_processors: Vec::new(),
            timers: Vec::new(),
        }));

        let runtime = match Runtime::new() {
            Ok(runtime) => runtime,
            Err(error) => {
                return Self::broken(
                    shared,
                    format!("its JavaScript runtime could not start: {error}"),
                )
            }
        };
        runtime.set_memory_limit(HEAP_BYTES);
        // * Loading is bounded: the shim registers nothing that computes, so a
        // * plugin still running after the deadline is looping, not loading.
        let deadline = Rc::new(Cell::new(Instant::now() + LOAD_DEADLINE));
        {
            let deadline = Rc::clone(&deadline);
            runtime.set_interrupt_handler(Some(Box::new(move || Instant::now() > deadline.get())));
        }

        let context = Context::full(&runtime).expect("a context for a fresh runtime");
        let mut registrations = Registrations::default();

        let source = match std::fs::read_to_string(plugin_dir.join("main.js")) {
            Ok(source) => source,
            Err(error) => {
                registrations.failure = Some(format!("its main.js could not be read: {error}"));
                return Self {
                    _runtime: runtime,
                    context,
                    shared,
                    deadline,
                    registrations,
                };
            }
        };

        let manifest_json = serde_json::to_string(manifest).unwrap_or_else(|_| "{}".into());

        // * The shim, the plugin and the bootstrap run in one borrow. The
        // * promise jobs the plugin queued are drained between borrows, because
        // * `Context::with` holds the runtime and draining inside it is a
        // * re-entrant borrow of the same cell.
        let setup = context.with(|ctx| -> Result<(), String> {
            install(ctx.clone(), Rc::clone(&shared), manifest_json)
                .map_err(|error| format!("its JavaScript could not be set up: {error}"))?;
            ctx.eval::<(), _>(SHIM.as_bytes())
                .map_err(|error| format!("the compatibility shim failed to load: {error}"))?;
            ctx.eval::<(), _>(source.as_bytes())
                .map_err(|error| format!("it threw while loading: {}", describe(&ctx, error)))
        });
        if let Err(reason) = setup {
            registrations.failure = Some(reason);
            return Self {
                _runtime: runtime,
                context,
                shared,
                deadline,
                registrations,
            };
        }

        let bootstrap = context.with(|ctx| {
            ctx.eval::<String, _>(BOOTSTRAP.as_bytes())
                .map_err(|error| format!("onload threw: {}", describe(&ctx, error)))
        });
        if let Err(reason) = bootstrap {
            registrations.failure = Some(reason);
            return Self {
                _runtime: runtime,
                context,
                shared,
                deadline,
                registrations,
            };
        }

        drain_jobs(&runtime);

        let taken = context.with(|ctx| {
            ctx.eval::<String, _>(b"__silica_take()").map_err(|error| {
                format!(
                    "its registrations could not be read: {}",
                    describe(&ctx, error)
                )
            })
        });
        match taken {
            Ok(json) => match serde_json::from_str::<Registrations>(&json) {
                Ok(parsed) => registrations = parsed,
                Err(error) => {
                    registrations.failure =
                        Some(format!("its registrations could not be read: {error}"));
                }
            },
            Err(reason) => registrations.failure = Some(reason),
        }

        Self {
            _runtime: runtime,
            context,
            shared,
            deadline,
            registrations,
        }
    }

    fn broken(shared: Rc<RefCell<Shared>>, reason: String) -> PluginHost {
        let runtime = Runtime::new().expect("a runtime for a failed plugin");
        let context = Context::full(&runtime).expect("a context for a failed plugin");
        PluginHost {
            _runtime: runtime,
            context,
            shared,
            deadline: Rc::new(Cell::new(Instant::now() + CALL_DEADLINE)),
            registrations: Registrations {
                failure: Some(reason),
                ..Registrations::default()
            },
        }
    }

    pub fn registrations(&self) -> &Registrations {
        &self.registrations
    }

    /// Notices the plugin has raised, drained. The app shows them and calls
    /// again; the plugin keeps running either way.
    pub fn take_notices(&self) -> Vec<String> {
        std::mem::take(&mut self.shared.borrow_mut().notices)
    }

    /// The plugin's own id, as the vault knows it.
    pub fn id(&self) -> String {
        self.shared.borrow().id.clone()
    }

    /// One sentence on what loading did, for the Settings row: what it
    /// registered, or the reason it stopped. A plugin that loaded and one
    /// that threw are the same label until something asks this.
    pub fn detail(&self) -> String {
        if let Some(reason) = &self.registrations.failure {
            return format!("It did not run: {reason}");
        }
        let commands = self.registrations.commands.len();
        let tabs = self.registrations.tabs.len();
        let mut said = format!(
            "Ran here: {commands} command{}",
            if commands == 1 { "" } else { "s" }
        );
        if tabs > 0 {
            said.push_str(&format!(
                ", {tabs} settings tab{}",
                if tabs == 1 { "" } else { "s" }
            ));
        }
        said.push('.');
        said
    }

    /// Whether this plugin registered a command with this id.
    pub fn has_command(&self, id: &str) -> bool {
        self.registrations
            .commands
            .iter()
            .any(|command| command.id == id)
    }

    /// Marks the plugin's view of the vault as out of date. The app calls
    /// this after writing a note itself, so a plugin that lists files next
    /// sees what the app just wrote.
    pub fn invalidate(&self) {
        self.shared.borrow_mut().mark_changed();
    }

    pub fn failed(&self) -> bool {
        self.registrations.failure.is_some()
    }

    /// Runs a callback the plugin registered, by the key the registrations
    /// carry. Bounded by `CALL_DEADLINE`.
    pub fn run(&self, key: &str, value: Option<&Value>) -> Result<(), String> {
        let key = serde_json::to_string(key).unwrap_or_else(|_| "\"\"".into());
        let value = match value {
            Some(value) => serde_json::to_string(value).unwrap_or_else(|_| "null".into()),
            None => "null".into(),
        };
        let call = format!("__silica_run({key}, {value})");
        self.deadline.set(Instant::now() + CALL_DEADLINE);
        let result = self.context.with(|ctx| {
            ctx.eval::<(), _>(call.as_bytes())
                .map_err(|error| describe(&ctx, error))
        });
        // * An async command does its work in the jobs after the call returns.
        drain_jobs(&self._runtime);
        result
    }

    /// Runs every timer the plugin has scheduled that is due now, then
    /// reschedules (or drops) it. Called before a command or post-processor
    /// runs, which is the pump this synchronous runtime has: a debounced
    /// callback or an interval fires on the plugin's next interaction rather
    /// than on a background thread, which the context cannot leave.
    pub fn pump_timers(&self) {
        let due = self.shared.borrow_mut().due_timers();
        if due.is_empty() {
            return;
        }
        self.deadline.set(Instant::now() + CALL_DEADLINE);
        self.context.with(|ctx| {
            for key in due {
                let call = format!("__silica_run({:?}, null)", key);
                if let Err(error) = ctx.eval::<(), _>(call.as_bytes()) {
                    let message = describe(&ctx, error);
                    self.shared.borrow_mut().notices.push(message);
                }
            }
        });
        drain_jobs(&self._runtime);
    }

    /// Runs a command the plugin registered, by id.
    /// The note the editor has open, as the app sees it. Set before a
    /// command runs, because the host is built long before any note is open
    /// and the protection has to hold at the moment of the write, not at the
    /// moment of loading.
    pub fn set_open_note(&self, note: Option<String>) {
        self.shared.borrow_mut().open_note = note;
    }

    pub fn run_command(&self, id: &str) -> Result<(), String> {
        self.pump_timers();
        let command = self
            .registrations
            .commands
            .iter()
            .find(|c| c.id == id)
            .ok_or_else(|| format!("'{id}' is not a command this plugin registered"))?;
        match &command.callback {
            Some(key) => self.run(key, None),
            None => Err(format!(
                "'{}' was registered without a callback",
                command.id
            )),
        }
    }

    /// Runs a post-processor callback the plugin registered, by the key its
    /// registration carries. Bounded by `CALL_DEADLINE`. The render pass
    /// that calls this for each registered post-processor is not wired yet;
    /// the method exists so registrations are real even before a frame
    /// consumes them.
    pub fn run_post_processor(&self, key: &str) -> Result<(), String> {
        self.pump_timers();
        self.run(key, None)
    }

    /// The metadata cache entry for a note, in Obsidian's shape.
    pub fn metadata(&self, path: &str) -> Result<Value, String> {
        let shared = self.shared.borrow();
        let resolved = shared.resolve(path)?;
        let text = std::fs::read_to_string(&resolved).map_err(|error| error.to_string())?;
        Ok(obsidian_metadata(&silica_cache::index_markdown(&text)))
    }
}

/// Installs the `__silica_raw_*` globals. These are the whole boundary: each
/// one is a small function over `Shared`, and nothing else is exposed.
fn install<'js>(
    ctx: Ctx<'js>,
    shared: Rc<RefCell<Shared>>,
    manifest_json: String,
) -> rquickjs::Result<()> {
    let globals = ctx.globals();

    macro_rules! set {
        ($name:literal, $f:expr) => {{
            let function = Function::new(ctx.clone(), $f)?;
            globals.set($name, function)?
        }};
    }

    let s = Rc::clone(&shared);
    set!("__silica_raw_notice", move |message: String| {
        s.borrow_mut().notices.push(message);
    });

    // Inject platform info so the shim's `Platform` object reports the real
    // host instead of pretending to be Linux everywhere.
    #[cfg(target_os = "windows")]
    let platform_json = r#"{"isDesktopApp":true,"isMobileApp":false,"isMobile":false,"isDesktop":true,"isWin":true,"isMacOS":false,"isLinux":false}"#;
    #[cfg(target_os = "macos")]
    let platform_json = r#"{"isDesktopApp":true,"isMobileApp":false,"isMobile":false,"isDesktop":true,"isWin":false,"isMacOS":true,"isLinux":false}"#;
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let platform_json = r#"{"isDesktopApp":true,"isMobileApp":false,"isMobile":false,"isDesktop":true,"isWin":false,"isMacOS":false,"isLinux":true}"#;
    globals.set("__silica_platform", platform_json)?;

    let s = Rc::clone(&shared);
    set!("__silica_raw_read_data", move || -> String {
        std::fs::read_to_string(s.borrow().data_path.clone()).unwrap_or_default()
    });

    let s = Rc::clone(&shared);
    set!(
        "__silica_raw_write_data",
        move |ctx: Ctx<'js>, text: String| -> rquickjs::Result<()> {
            let path = s.borrow().data_path.clone();
            if let Some(parent) = path.parent() {
                make_dirs_checked(&s, parent)
                    .map_err(|error| Exception::throw_message(&ctx, &error.to_string()))?;
            }
            s.borrow()
                .write_checked(&path, text.as_bytes())
                .map_err(|error| Exception::throw_message(&ctx, &error.to_string()))
        }
    );

    let s = Rc::clone(&shared);
    set!("__silica_raw_files", move || -> String {
        let mut shared = s.borrow_mut();
        let list: Vec<Value> = shared
            .files()
            .iter()
            .map(|f| {
                json!({
                    "path": f.path,
                    "name": f.name,
                    "basename": f.basename,
                    "extension": f.extension,
                    "size": f.size,
                    "mtime": f.mtime_ms,
                })
            })
            .collect();
        Value::Array(list).to_string()
    });

    let s = Rc::clone(&shared);
    set!("__silica_raw_file", move |path: String| -> String {
        s.borrow_mut().entry(&path).unwrap_or_default()
    });

    let s = Rc::clone(&shared);
    set!("__silica_raw_active_file", move || -> String {
        s.borrow().active_file.clone().unwrap_or_default()
    });

    let s = Rc::clone(&shared);
    set!("__silica_raw_meta_cache", move |path: String| -> String {
        let resolved = match s.borrow().resolve(&path) {
            Ok(resolved) => resolved,
            Err(_) => return String::new(),
        };
        match std::fs::read_to_string(resolved) {
            Ok(text) => obsidian_metadata(&silica_cache::index_markdown(&text)).to_string(),
            Err(_) => String::new(),
        }
    });

    // * Resolved outgoing links of one file: target path -> occurrence count.
    //   Reads the file, resolves each link against the vault, groups hits.
    let s = Rc::clone(&shared);
    set!(
        "__silica_raw_resolved_links",
        move |path: String| -> String {
            let shared = s.borrow();
            let resolved_path = match shared.resolve(&path) {
                Ok(p) => p,
                Err(_) => return String::new(),
            };
            let root = shared.root.clone();
            drop(shared);
            let Ok(text) = std::fs::read_to_string(&resolved_path) else {
                return String::new();
            };
            let meta = silica_cache::index_markdown(&text);
            let mut out: serde_json::Map<String, Value> = serde_json::Map::new();
            let links: Vec<&str> = meta
                .links
                .iter()
                .map(|l| l.link.as_str())
                .chain(meta.embeds.iter().map(|l| l.link.as_str()))
                .chain(meta.frontmatter_links.iter().map(|l| l.link.as_str()))
                .collect();
            for link in links {
                if let Some(target) = vault_link_target(&root, link, &path) {
                    let entry = out.entry(target).or_insert_with(|| Value::from(0));
                    *entry = Value::from(entry.as_u64().unwrap_or(0) + 1);
                }
            }
            Value::Object(out).to_string()
        }
    );

    // * Unresolved outgoing links of one file: link text -> occurrence count.
    let s = Rc::clone(&shared);
    set!(
        "__silica_raw_unresolved_links",
        move |path: String| -> String {
            let shared = s.borrow();
            let resolved_path = match shared.resolve(&path) {
                Ok(p) => p,
                Err(_) => return String::new(),
            };
            let root = shared.root.clone();
            drop(shared);
            let Ok(text) = std::fs::read_to_string(&resolved_path) else {
                return String::new();
            };
            let meta = silica_cache::index_markdown(&text);
            let mut out: serde_json::Map<String, Value> = serde_json::Map::new();
            let links: Vec<&str> = meta
                .links
                .iter()
                .map(|l| l.link.as_str())
                .chain(meta.embeds.iter().map(|l| l.link.as_str()))
                .chain(meta.frontmatter_links.iter().map(|l| l.link.as_str()))
                .collect();
            for link in links {
                if vault_link_target(&root, link, &path).is_none() {
                    let entry = out.entry(link.to_string()).or_insert_with(|| Value::from(0));
                    *entry = Value::from(entry.as_u64().unwrap_or(0) + 1);
                }
            }
            Value::Object(out).to_string()
        }
    );

    // * Backlinks to one file: every vault note that links to it, grouped by
    //   source. Scans all markdown notes — O(vault), honest but not cached.
    //   Ponytail: for large vaults the app should hand its backlink index
    //   over instead of re-scanning here.
    let s = Rc::clone(&shared);
    set!(
        "__silica_raw_backlinks",
        move |path: String| -> String {
            let root = s.borrow().root.clone();
            drop(s.borrow());
            let mut out: Vec<Value> = Vec::new();
            if let Ok(entries) = walk_markdown_files(&root) {
                for entry in entries {
                    let Ok(text) = std::fs::read_to_string(&entry) else {
                        continue;
                    };
                    let meta = silica_cache::index_markdown(&text);
                    let source = entry.to_string_lossy().replace('\\', "/");
                    let hits: Vec<String> = meta
                        .links
                        .iter()
                        .map(|l| l.link.as_str())
                        .chain(meta.embeds.iter().map(|l| l.link.as_str()))
                        .chain(meta.frontmatter_links.iter().map(|l| l.link.as_str()))
                        .filter(|link| {
                            vault_link_target(&root, link, &source)
                                .as_deref()
                                == Some(path.as_str())
                        })
                        .map(|s| s.to_string())
                        .collect();
                    if !hits.is_empty() {
                        out.push(json!({
                            "source": source,
                            "links": hits,
                        }));
                    }
                }
            }
            serde_json::to_string(&out).unwrap_or_default()
        }
    );

    // * All tags across the vault: name, occurrence count, files containing it.
    //   Scans all markdown notes — O(vault).
    //   Ponytail: the app should hand its tag index over for large vaults.
    let s = Rc::clone(&shared);
    set!(
        "__silica_raw_get_tags",
        move || -> String {
            let root = s.borrow().root.clone();
            drop(s.borrow());
            let mut tags: std::collections::BTreeMap<String, (usize, Vec<String>)> =
                std::collections::BTreeMap::new();
            if let Ok(entries) = walk_markdown_files(&root) {
                for entry in entries {
                    let Ok(text) = std::fs::read_to_string(&entry) else {
                        continue;
                    };
                    let meta = silica_cache::index_markdown(&text);
                    let source = entry.to_string_lossy().replace('\\', "/");
                    for tag in &meta.tags {
                        let name = tag.tag.as_str();
                        let entry = tags.entry(name.to_string()).or_default();
                        entry.0 += 1;
                        entry.1.push(source.clone());
                    }
                }
            }
            let out: Vec<Value> = tags
                .into_iter()
                .map(|(name, (count, files))| {
                    json!({"name": name, "count": count, "files": files})
                })
                .collect();
            serde_json::to_string(&out).unwrap_or_default()
        }
    );

    let s = Rc::clone(&shared);
    set!("__silica_raw_vault_read", move |ctx: Ctx<'js>,
                                          path: String|
          -> rquickjs::Result<
        String,
    > {
        let resolved = s
            .borrow()
            .resolve(&path)
            .map_err(|error| Exception::throw_message(&ctx, &error))?;
        std::fs::read_to_string(&resolved).map_err(|error| {
            Exception::throw_message(&ctx, &format!("'{path}' could not be read: {error}"))
        })
    });

    let s = Rc::clone(&shared);
    set!("__silica_raw_vault_read_bytes", move |ctx: Ctx<'js>,
                                               path: String|
          -> rquickjs::Result<
        String,
    > {
        let resolved = s
            .borrow()
            .resolve(&path)
            .map_err(|error| Exception::throw_message(&ctx, &error))?;
        let bytes = std::fs::read(&resolved).map_err(|error| {
            Exception::throw_message(&ctx, &format!("{path} could not be read: {error}"))
        })?;
        Ok(to_base64(&bytes))
    });

    let s = Rc::clone(&shared);
    set!(
        "__silica_raw_vault_write",
        move |ctx: Ctx<'js>, path: String, text: String| -> rquickjs::Result<()> {
            s.borrow_mut().mark_changed();
            let resolved = s
                .borrow()
                .writable(&path)
                .map_err(|error| Exception::throw_message(&ctx, &error))?;
            s.borrow()
                .write_checked(&resolved, text.as_bytes())
                .map_err(|error| {
                    Exception::throw_message(
                        &ctx,
                        &format!("'{path}' could not be written: {error}"),
                    )
                })
        }
    );

    let s = Rc::clone(&shared);
    set!("__silica_raw_vault_append", move |ctx: Ctx<'js>,
                                            path: String,
                                            text: String|
          -> rquickjs::Result<
        (),
    > {
        s.borrow_mut().mark_changed();
        let resolved = s
            .borrow()
            .writable(&path)
            .map_err(|error| Exception::throw_message(&ctx, &error))?;
        // * A failed read is not an empty file. Substituting "" here would
        // * replace the file's real contents with only the appended text, so a
        // * file that cannot be decoded stops the append instead.
        let mut existing = match std::fs::read(&resolved) {
            Ok(bytes) => match String::from_utf8(bytes) {
                Ok(text) => text,
                Err(_) => {
                    return Err(Exception::throw_message(
                        &ctx,
                        &format!("'{path}' is not valid UTF-8, so it cannot be appended to"),
                    ))
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => {
                return Err(Exception::throw_message(
                    &ctx,
                    &format!("'{path}' could not be read before appending: {error}"),
                ))
            }
        };
        existing.push_str(&text);
        s.borrow()
            .write_checked(&resolved, existing.as_bytes())
            .map_err(|error| {
                Exception::throw_message(&ctx, &format!("'{path}' could not be written: {error}"))
            })
    });

    let s = Rc::clone(&shared);
    set!("__silica_raw_vault_create", move |ctx: Ctx<'js>,
                                            path: String,
                                            text: String|
          -> rquickjs::Result<
        (),
    > {
        s.borrow_mut().mark_changed();
        let resolved = s
            .borrow()
            .writable(&path)
            .map_err(|error| Exception::throw_message(&ctx, &error))?;
        if resolved.exists() {
            return Err(Exception::throw_message(
                &ctx,
                &format!("'{path}' already exists; a plugin can only create a file that does not"),
            ));
        }
        if let Some(parent) = resolved.parent() {
            make_dirs_checked(&s, parent)
                .map_err(|error| Exception::throw_message(&ctx, &error.to_string()))?;
        }
        s.borrow()
            .write_checked(&resolved, text.as_bytes())
            .map_err(|error| {
                Exception::throw_message(&ctx, &format!("'{path}' could not be written: {error}"))
            })
    });

    let s = Rc::clone(&shared);
    set!(
        "__silica_raw_vault_create_folder",
        move |ctx: Ctx<'js>, path: String| -> rquickjs::Result<()> {
            s.borrow_mut().mark_changed();
            let resolved = s
                .borrow()
                .resolve(&path)
                .map_err(|error| Exception::throw_message(&ctx, &error))?;
            make_dirs_checked(&s, &resolved).map_err(|error| {
                Exception::throw_message(&ctx, &format!("'{path}' could not be created: {error}"))
            })
        }
    );

    let s = Rc::clone(&shared);
    set!("__silica_raw_vault_delete", move |ctx: Ctx<'js>,
                                            path: String|
          -> rquickjs::Result<
        (),
    > {
        s.borrow_mut().mark_changed();
        let resolved = s
            .borrow()
            .writable_tree(&path)
            .map_err(|error| Exception::throw_message(&ctx, &error))?;
        // * A plugin deletes into the vault's trash rather than unlinking, so a
        // * mistaken delete is recoverable.
        let shared_root = s.borrow().root.clone();
        let trash = shared_root.join(".trash");
        // ! The name has to be unique per source path. Flattening `a/b.md` to
        // ! `a-b.md` made it collide with a real `a-b.md`, so deleting the
        // ! second overwrote the first trashed note — two different notes, one
        // ! survivor, and the user told nothing. A numbered suffix keeps both,
        // ! and makes deleting the same note twice keep both versions.
        let stem = resolved
            .strip_prefix(&shared_root)
            .unwrap_or(&resolved)
            .to_string_lossy()
            .replace(['/', '\\'], "-");
        std::fs::create_dir_all(&trash)
            .map_err(|error| Exception::throw_message(&ctx, &error.to_string()))?;
        // ! `.trash` itself must be a real directory inside the vault: a link
        // ! planted there would move the note out of the vault entirely.
        let trash_meta = std::fs::symlink_metadata(&trash)
            .map_err(|error| Exception::throw_message(&ctx, &error.to_string()))?;
        if !trash_meta.file_type().is_dir() {
            return Err(Exception::throw_message(
                &ctx,
                "the vault's .trash is not a plain directory; refusing to move a note into it",
            ));
        }
        let mut target = trash.join(&stem);
        let mut counter = 1u32;
        while std::fs::symlink_metadata(&target).is_ok() {
            target = trash.join(format!("{stem}.{counter}"));
            counter += 1;
            if counter > 10_000 {
                return Err(Exception::throw_message(
                    &ctx,
                    &format!("'{path}' could not be trashed: too many copies already there"),
                ));
            }
        }
        // ! Cross-directory rename fails on Windows; copy then delete instead.
        std::fs::copy(&resolved, &target).map_err(|error| {
            Exception::throw_message(&ctx, &format!("'{path}' could not be deleted: {error}"))
        })?;
        std::fs::remove_file(&resolved).map_err(|error| {
            Exception::throw_message(&ctx, &format!("'{path}' could not be deleted: {error}"))
        })
    });

    let s = Rc::clone(&shared);
    set!("__silica_raw_vault_rename", move |ctx: Ctx<'js>,
                                            from: String,
                                            to: String|
          -> rquickjs::Result<
        (),
    > {
        s.borrow_mut().mark_changed();
        // ! The source needs the folder guard too: renaming a folder moves
        // ! every note inside it, the open one among them.
        let source = s
            .borrow()
            .writable_tree(&from)
            .map_err(|error| Exception::throw_message(&ctx, &error))?;
        let target = s
            .borrow()
            .writable(&to)
            .map_err(|error| Exception::throw_message(&ctx, &error))?;
        std::fs::rename(&source, &target).map_err(|error| {
            Exception::throw_message(&ctx, &format!("'{from}' could not be renamed: {error}"))
        })
    });

    let s = Rc::clone(&shared);
    set!("__silica_raw_execute", move |_id: String| -> bool {
        // * Commands the app itself owns cannot be reached from a plugin's own
        // * runtime without a round trip to the app, which a synchronous call
        // * cannot make. Reported as not run rather than silently ignored.
        let _ = &s;
        false
    });

    let s = Rc::clone(&shared);
    set!(
        "__silica_raw_post_processor",
        move |order: i64, key: String| {
            s.borrow_mut().post_processors.push((order, key));
        }
    );

    // * Timers. QuickJS ships no event loop, so the host keeps the schedule
    //   and pumps due callbacks before commands and post-processors run.
    let s = Rc::clone(&shared);
    set!("__silica_raw_set_timer", move |delay_ms: f64,
                                         repeat: bool,
                                         key: String|
          -> u32 {
        s.borrow_mut().add_timer(delay_ms, repeat, key)
    });
    let s = Rc::clone(&shared);
    set!("__silica_raw_cancel_timer", move |id: u32| {
        s.borrow_mut().cancel_timer(id);
    });

    globals.set("__silica_manifest", manifest_json)?;
    Ok(())
}

/// The bootstrap: instantiate the exported class, hand it the app, call
/// `onload`. Errors are recorded as the plugin's failure by the caller.
const BOOTSTRAP: &str = r#"
(function () {
  var exported = (module.exports && module.exports.default) || module.exports;
  if (typeof exported !== "function") {
    __silica_fail("main.js does not export a plugin class");
    return "";
  }
  var manifest = JSON.parse(__silica_manifest);
  var plugin = new exported(__silica_app(manifest), manifest);
  if (typeof plugin.onload !== "function") {
    __silica_fail("the plugin class has no onload");
    return "";
  }
  // * `onload` is usually async and the registrations happen after its first
  // * await, so the promise is caught here and its jobs are drained by the
  // * caller before the registrations are read.
  Promise.resolve(plugin.onload()).catch(function (error) {
    __silica_fail("onload threw: " + __silica_message(error));
  });
  return "";
})()
"#;

/// Runs the promise jobs a plugin queued. A plugin that awaits before it
/// registers anything is ordinary, and skipping this loses everything after
/// the first `await`.
fn drain_jobs(runtime: &Runtime) {
    while runtime.is_job_pending() {
        // * A job that throws has already been recorded as the plugin's
        // * failure; a job that loops is stopped by the load deadline.
        if runtime.execute_pending_job().is_err() {
            break;
        }
    }
}

/// A thrown JavaScript value as a sentence, with the JS stack line kept so a
/// plugin author can tell where it came from.
fn describe(ctx: &Ctx<'_>, error: rquickjs::Error) -> String {
    if error.is_exception() {
        if let Some(exception) = ctx.catch().as_exception() {
            let mut out = exception.message().unwrap_or_else(|| "an error".into());
            if let Some(stack) = exception.stack() {
                if let Some(line) = stack.lines().nth(1) {
                    out.push_str(" (");
                    out.push_str(line.trim());
                    out.push(')');
                }
            }
            return out;
        }
    }
    error.to_string()
}

/// Base64-encode `bytes` so QuickJS can receive binary file contents as text.
/// A plugin decodes with `atob()`. No external dependency: the alphabet is
/// RFC 4648, output is padded with `=`.
fn to_base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        let triple = ((b0 as u32) << 16) | ((b1 as u32) << 8) | (b2 as u32);
        out.push(ALPHABET[((triple >> 18) & 63) as usize] as char);
        out.push(ALPHABET[((triple >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[((triple >> 6) & 63) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[(triple & 63) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// The vault's files, as `TAbstractFile` reports them. Hidden folders are
/// skipped, which keeps `.obsidian` and `.trash` out of what a plugin sees.
/// One vault walk: every file under `root`, as `FileEntry`s. Shared across
/// plugins so N plugins do not walk the vault N times.
pub(crate) fn collect_files(root: &Path) -> Vec<FileEntry> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if kind.is_dir() {
                stack.push(path);
                continue;
            }
            if !kind.is_file() {
                continue;
            }
            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            let extension = Path::new(&name)
                .extension()
                .map(|e| e.to_string_lossy().into_owned())
                .unwrap_or_default();
            let metadata = entry.metadata().ok();
            out.push(FileEntry {
                basename: Path::new(&name)
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| name.clone()),
                path: relative,
                name,
                extension,
                size: metadata.as_ref().map(|m| m.len()).unwrap_or(0),
                mtime_ms: metadata
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0),
            });
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// Every markdown file under `root`, as absolute paths. Hidden folders are
/// skipped.
fn walk_markdown_files(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if kind.is_dir() {
                stack.push(path);
                continue;
            }
            if !kind.is_file() {
                continue;
            }
            if matches!(path.extension(), Some(ext) if ext == "md") {
                out.push(path);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Resolves a wikilink `link` written in `source` against the vault at
/// `root`, returning the vault-relative target path if the file exists.
/// Tries `<root>/<link>.md` and `<root>/<source_dir>/<link>.md`.
fn vault_link_target(root: &Path, link: &str, source: &str) -> Option<String> {
    let target = link.split('|').next()?.split('#').next()?;
    let target = target.trim();
    if target.is_empty() {
        return None;
    }
    let direct = root.join(format!("{target}.md"));
    if direct.is_file() {
        return rel_path(root, &direct);
    }
    let source_dir = Path::new(source).parent();
    if let Some(dir) = source_dir {
        let relative = root.join(dir.join(format!("{target}.md")));
        if relative.is_file() {
            return rel_path(root, &relative);
        }
    }
    None
}

/// Vault-relative path of `file` under `root`, with `/` separators.
fn rel_path(root: &Path, file: &Path) -> Option<String> {
    file.strip_prefix(root)
        .ok()
        .map(|p| p.to_string_lossy().replace('\\', "/").to_string())
}

/// The parse index, translated into the shape Obsidian's `metadataCache`
/// answers with. The field names and the position shape are Obsidian's; the
/// index itself is the one the reader already builds.
fn obsidian_metadata(meta: &CachedMetadata) -> Value {
    fn pos(position: &silica_cache::Pos) -> Value {
        json!({ "line": position.line, "col": position.col, "offset": position.offset })
    }
    fn loc(position: &silica_cache::Loc) -> Value {
        json!({ "start": pos(&position.start), "end": pos(&position.end) })
    }
    fn link(link: &silica_cache::LinkCache) -> Value {
        json!({
            "link": link.link,
            "original": link.original,
            "displayText": link.display_text,
            "position": loc(&link.position),
        })
    }

    json!({
        "links": meta.links.iter().map(link).collect::<Vec<_>>(),
        "embeds": meta.embeds.iter().map(link).collect::<Vec<_>>(),
        "tags": meta.tags.iter().map(|tag| json!({
            "tag": tag.tag,
            "position": loc(&tag.position),
        })).collect::<Vec<_>>(),
        "headings": meta.headings.iter().map(|heading| json!({
            "heading": heading.heading,
            "level": heading.level,
            "position": loc(&heading.position),
        })).collect::<Vec<_>>(),
        "sections": meta.sections.iter().map(|section| {
            let mut out = json!({
                "type": section.kind.as_str(),
                "position": loc(&section.position),
            });
            if let Some(id) = &section.id {
                out["id"] = json!(id);
            }
            out
        }).collect::<Vec<_>>(),
        "frontmatter": meta.frontmatter.as_ref().map(yaml_to_json),
        "frontmatterPosition": meta.frontmatter_position.as_ref().map(loc),
        "frontmatterLinks": meta.frontmatter_links.iter().map(|link| json!({
            "key": link.key,
            "link": link.link,
            "original": link.original,
            "displayText": link.display_text,
        })).collect::<Vec<_>>(),
        "listItems": meta.list_items.iter().map(|item| {
            let mut out = json!({
                "parent": item.parent,
                "position": loc(&item.position),
            });
            if let Some(task) = item.task {
                out["task"] = json!(task.to_string());
            }
            if let Some(id) = &item.id {
                out["id"] = json!(id);
            }
            out
        }).collect::<Vec<_>>(),
        "blocks": meta.blocks.iter().map(|(id, block)| (id.clone(), json!({
            "id": block.id,
            "position": loc(&block.position),
        }))).collect::<serde_json::Map<_, _>>(),
        "footnotes": meta.footnotes.iter().map(|footnote| json!({
            "id": footnote.id,
            "position": loc(&footnote.position),
        })).collect::<Vec<_>>(),
    })
}

/// YAML frontmatter as JSON. A mapping keeps its keys, a scalar stands alone.
fn yaml_to_json(value: &serde_yaml::Value) -> Value {
    match value {
        serde_yaml::Value::Null => Value::Null,
        serde_yaml::Value::Bool(b) => json!(b),
        serde_yaml::Value::Number(n) => json!(n),
        serde_yaml::Value::String(s) => json!(s),
        serde_yaml::Value::Sequence(items) => {
            Value::Array(items.iter().map(yaml_to_json).collect())
        }
        serde_yaml::Value::Mapping(map) => Value::Object(
            map.iter()
                .map(|(key, value)| {
                    let key = match key {
                        serde_yaml::Value::String(s) => s.clone(),
                        other => serde_json::to_value(yaml_to_json(other))
                            .map(|v| v.to_string())
                            .unwrap_or_default()
                            .trim_matches('"')
                            .to_string(),
                    };
                    (key, yaml_to_json(value))
                })
                .collect(),
        ),
        serde_yaml::Value::Tagged(tagged) => yaml_to_json(&tagged.value),
    }
}

#[cfg(test)]
mod path_tests {
    use super::*;

    /// A vault on disk plus a `Shared` whose open note is `open_note`.
    fn vault(open_note: Option<&str>) -> (tempfile::TempDir, Rc<RefCell<Shared>>) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let shared = Shared {
            root: root.clone(),
            data_path: root.join(".obsidian/plugins/example/data.json"),
            id: "example".into(),
            files: Vec::new(),
            files_stale: false,
            shared_files: None,
            notices: Vec::new(),
            active_file: None,
            open_note: open_note.map(str::to_string),
            post_processors: Vec::new(),
            timers: Vec::new(),
        };
        (dir, Rc::new(RefCell::new(shared)))
    }

    /// The doc on `write_checked` says the existing file is snapshotted
    /// before it is replaced, so a plugin write cannot destroy prior
    /// content. Unix snapshots in its own branch; the Windows branch calls
    /// `write_at_handles`, which does not. A plugin that rewrites a note —
    /// the normal case, not the fresh-file one — therefore leaves the user
    /// with no way back to the bytes it replaced.
    #[test]
    fn a_plugin_write_over_an_existing_note_keeps_a_snapshot_of_the_old_bytes() {
        let (dir, shared) = vault(None);
        let root = dir.path().canonicalize().unwrap();
        let note = root.join("note.md");
        std::fs::write(&note, "the user's own draft\n").unwrap();
        let resolved = shared.borrow().writable("note.md").unwrap();

        shared
            .borrow()
            .write_checked(&resolved, b"what the plugin wrote\n")
            .expect("a plugin write over an existing note");

        assert_eq!(
            std::fs::read_to_string(&note).unwrap(),
            "what the plugin wrote\n"
        );
        let snapshots = root.join(".trash").join(".snapshots");
        let kept: Vec<String> = std::fs::read_dir(&snapshots)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            !kept.is_empty(),
            "no snapshot of the replaced bytes: {snapshots:?}"
        );
        let content = std::fs::read_to_string(snapshots.join(&kept[0])).unwrap();
        assert_eq!(
            content, "the user's own draft\n",
            "the snapshot kept the wrong bytes"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_folder_cannot_be_written_through() {
        let (dir, shared) = vault(None);
        let root = dir.path().canonicalize().unwrap();
        let outside = tempfile::tempdir().unwrap();
        // * `escape` is inside the vault by every string measure and points out.
        std::os::unix::fs::symlink(outside.path(), root.join("escape")).unwrap();

        let err = shared
            .borrow()
            .writable("escape/planted.md")
            .expect_err("a write through a symlink must be refused");
        assert!(err.contains("symbolic link"), "{err}");
        assert!(!outside.path().join("planted.md").exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_file_cannot_be_written() {
        let (_dir, shared) = vault(None);
        let root = shared.borrow().root.clone();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("real.md");
        std::fs::write(&target, "outside").unwrap();
        std::os::unix::fs::symlink(&target, root.join("link.md")).unwrap();

        assert!(shared.borrow().writable("link.md").is_err());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "outside");
    }

    #[test]
    fn the_open_note_cannot_be_written_under_another_capitalisation() {
        let (_dir, shared) = vault(Some("Notes/Plan.md"));
        let root = shared.borrow().root.clone();
        std::fs::create_dir_all(root.join("notes")).unwrap();
        std::fs::write(root.join("notes/plan.md"), "saved").unwrap();

        // * Same file as the open note on a case-insensitive filesystem.
        let err = shared
            .borrow()
            .writable("notes/plan.md")
            .expect_err("a differently capitalised open note must be refused");
        assert!(err.contains("open in the editor"), "{err}");
        // * The exact spelling is still refused too.
        assert!(shared.borrow().writable("Notes/Plan.md").is_err());
        // * A different note is not.
        assert!(shared.borrow().writable("Other.md").is_ok());
    }

    #[test]
    fn a_folder_holding_the_open_note_cannot_be_moved() {
        let (_dir, shared) = vault(Some("Notes/Plan.md"));
        let root = shared.borrow().root.clone();
        std::fs::create_dir_all(root.join("Notes")).unwrap();
        std::fs::write(root.join("Notes/Plan.md"), "saved").unwrap();

        // * The folder, not the file: `writable` would allow this.
        let err = shared
            .borrow()
            .writable_tree("Notes")
            .expect_err("the open note's folder must be refused");
        assert!(err.contains("contains the note"), "{err}");
        // * A sibling folder is fine.
        std::fs::create_dir_all(root.join("Other")).unwrap();
        assert!(shared.borrow().writable_tree("Other").is_ok());
        // * So is a folder whose name only shares a prefix.
        std::fs::create_dir_all(root.join("Notes-old")).unwrap();
        assert!(shared.borrow().writable_tree("Notes-old").is_ok());
    }

    #[test]
    fn traversal_and_absolute_paths_still_bounce() {
        let (_dir, shared) = vault(None);
        assert!(shared.borrow().writable("../outside.md").is_err());
        assert!(
            shared.borrow().writable("/etc/passwd").is_ok(),
            "a leading slash is vault-relative"
        );
        assert!(!shared
            .borrow()
            .writable("/etc/passwd")
            .unwrap()
            .starts_with("/etc"));
    }
}

/// The QuickJS environment a plugin actually gets.
///
/// These run in a real `Runtime` rather than reading `shim.js` as text: what
/// matters is what the engine exposes, and a global that looks absent in the
/// source can still be reached through a prototype or a built-in.
#[cfg(test)]
mod capability_tests {
    use super::*;

    /// Evaluate `script` in a fresh runtime with the shim installed.
    fn run(script: &str) -> Result<String, String> {
        let runtime = Runtime::new().map_err(|e| e.to_string())?;
        runtime.set_memory_limit(HEAP_BYTES);
        let deadline = Rc::new(Cell::new(Instant::now() + LOAD_DEADLINE));
        {
            let deadline = Rc::clone(&deadline);
            runtime.set_interrupt_handler(Some(Box::new(move || Instant::now() > deadline.get())));
        }
        let context = Context::full(&runtime).map_err(|e| e.to_string())?;
        let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
        let root = dir.path().canonicalize().map_err(|e| e.to_string())?;
        let _shared = Rc::new(RefCell::new(Shared {
            id: "cap".into(),
            root: root.clone(),
            data_path: root.join("data.json"),
            files: Vec::new(),
            files_stale: false,
            shared_files: None,
            notices: Vec::new(),
            active_file: None,
            open_note: None,
            post_processors: Vec::new(),
            timers: Vec::new(),
        }));
        context.with(|ctx| {
            // Inject the platform the tests run on, matching the host's cfg,
            // before the shim evaluates (it reads __silica_platform at load).
            #[cfg(target_os = "windows")]
            let platform_json = r#"{"isDesktopApp":true,"isMobileApp":false,"isMobile":false,"isDesktop":true,"isWin":true,"isMacOS":false,"isLinux":false}"#;
            #[cfg(target_os = "macos")]
            let platform_json = r#"{"isDesktopApp":true,"isMobileApp":false,"isMobile":false,"isDesktop":true,"isWin":false,"isMacOS":true,"isLinux":false}"#;
            #[cfg(not(any(target_os = "windows", target_os = "macos")))]
            let platform_json = r#"{"isDesktopApp":true,"isMobileApp":false,"isMobile":false,"isDesktop":true,"isWin":false,"isMacOS":false,"isLinux":true}"#;
            ctx.globals().set("__silica_platform", platform_json)
                .map_err(|e| format!("platform: {e}"))?;
            ctx.eval::<(), _>(SHIM.as_bytes())
                .map_err(|e| format!("shim: {e}"))?;
            ctx.eval::<String, _>(script.as_bytes())
                .map_err(|e| format!("{e}"))
        })
    }

    #[test]
    fn the_engine_exposes_no_process_global() {
        let out = run("typeof process");
        assert_eq!(out.as_deref(), Ok("undefined"));
    }

    #[test]
    fn require_throws_rather_than_loading_a_module() {
        let out = run("
            try { require('fs'); 'loaded' } catch (e) { 'refused' }
        ");
        assert_eq!(out.as_deref(), Ok("refused"));
    }

    /// `require('obsidian')` is the module a plugin imports — the shim's own
    /// `require` returns it for that one name and refuses everything else.
    #[test]
    fn the_obsidian_module_is_what_require_returns() {
        let out = run("
            const mod = require('obsidian');
            (mod && typeof mod.Plugin === 'function') ? 'ok' : 'missing'
        ");
        assert_eq!(
            out.as_deref(),
            Ok("ok"),
            "the shim module must be reachable"
        );
    }

    /// The supported APIs still work, so the checks above are not passing
    /// because the whole environment is empty.
    #[test]
    fn the_supported_api_still_works() {
        let out = run("
            const mod = require('obsidian');
            ['Plugin', 'PluginSettingTab', 'Setting', 'Notice', 'moment', 'Platform']
              .every((n) => mod[n] !== undefined) ? 'ok' : 'missing'
        ");
        assert_eq!(out.as_deref(), Ok("ok"));
    }
}

/// The limits a plugin runs under: memory, and a wall-clock deadline per call.
///
/// These are checked by making the runtime do the forbidden thing and watching
/// it stop, not by reading the configuration back.
#[cfg(test)]
mod limit_tests {
    use super::*;

    /// Evaluate `script` under the same limits a plugin gets, returning the
    /// error message when it is stopped.
    fn run(script: &str) -> Result<String, String> {
        let runtime = Runtime::new().map_err(|e| format!("runtime: {e}"))?;
        runtime.set_memory_limit(HEAP_BYTES);
        let deadline = Rc::new(Cell::new(Instant::now() + Duration::from_millis(150)));
        {
            let deadline = Rc::clone(&deadline);
            runtime.set_interrupt_handler(Some(Box::new(move || Instant::now() > deadline.get())));
        }
        let context = Context::full(&runtime).map_err(|e| format!("context: {e}"))?;
        context.with(|ctx| {
            ctx.eval::<String, _>(script.as_bytes())
                .map_err(|e| format!("{e}"))
        })
    }

    /// An infinite loop is stopped by the deadline rather than hanging the
    /// test process.
    #[test]
    fn a_loop_is_stopped_by_the_deadline() {
        let started = Instant::now();
        let out = run("while (true) {}");
        let elapsed = started.elapsed();

        assert!(out.is_err(), "an endless loop must not return a value");
        // * It has to stop near the deadline, not merely eventually.
        assert!(
            elapsed < Duration::from_secs(5),
            "the loop ran for {elapsed:?}; the deadline did not fire"
        );
    }

    /// Allocating past the heap limit fails instead of taking the host down.
    #[test]
    fn allocating_past_the_heap_limit_fails() {
        let out = run("
            var hold = [];
            for (var i = 0; i < 100000; i++) { hold.push(new Array(100000).join('x')); }
            'allocated'
        ");
        assert!(out.is_err(), "the heap limit must stop the allocation");
    }

    /// A fresh runtime still works, which is how the host recovers after a
    /// plugin is stopped: the failed call did not poison anything.
    #[test]
    fn a_valid_call_still_works_after_a_timeout() {
        assert!(run("while (true) {}").is_err());
        assert_eq!(run("'fine'").as_deref(), Ok("fine"));
    }
}
