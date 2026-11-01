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
    pub post_processors: usize,
    #[serde(default)]
    pub failure: Option<String>,
}

/// A file in the vault, as the plugin API's `TAbstractFile` reports one.
#[derive(Debug, Clone)]
struct FileEntry {
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
    files: Vec<FileEntry>,
    /// Set by anything that changes the vault, so the next read of the file
    /// list walks it again instead of answering from a stale snapshot.
    files_stale: bool,
    notices: Vec<String>,
    active_file: Option<String>,
    /// The note open in the editor. Writes to it are refused: a plugin writing
    /// under an editor with unsaved changes would lose them.
    open_note: Option<String>,
}

impl Shared {
    /// The vault's files, re-walked if anything has changed since the last
    /// read. Ponytail: the walk is the honest source while the plugin crate
    /// has no handle on the app's index; for a vault large enough for that to
    /// matter, the app should hand its index over instead.
    fn files(&mut self) -> &[FileEntry] {
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
        Ok(out)
    }

    fn writable(&self, path: &str) -> Result<PathBuf, String> {
        let resolved = self.resolve(path)?;
        let relative = resolved
            .strip_prefix(&self.root)
            .unwrap_or(&resolved)
            .to_string_lossy()
            .replace('\\', "/");
        if self.open_note.as_deref() == Some(relative.as_str()) {
            return Err(format!(
                "'{relative}' is open in the editor; a plugin cannot write it while its unsaved changes are here"
            ));
        }
        Ok(resolved)
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
    ) -> PluginHost {
        let id = if manifest.id.is_empty() {
            plugin_dir
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default()
        } else {
            manifest.id.clone()
        };
        let shared = Rc::new(RefCell::new(Shared {
            id,
            root: vault.to_path_buf(),
            data_path: plugin_dir.join("data.json"),
            files: collect_files(vault),
            files_stale: false,
            notices: Vec::new(),
            active_file: open_note.clone(),
            open_note,
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
        // Loading is bounded: the shim registers nothing that computes, so a
        // plugin still running after the deadline is looping, not loading.
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

        // The shim, the plugin and the bootstrap run in one borrow. The
        // promise jobs the plugin queued are drained between borrows, because
        // `Context::with` holds the runtime and draining inside it is a
        // re-entrant borrow of the same cell.
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
            ctx.eval::<String, _>(b"__silica_take()")
                .map_err(|error| format!("its registrations could not be read: {}", describe(&ctx, error)))
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
        let result = self
            .context
            .with(|ctx| ctx.eval::<(), _>(call.as_bytes()).map_err(|error| describe(&ctx, error)));
        // An async command does its work in the jobs after the call returns.
        drain_jobs(&self._runtime);
        result
    }

    /// Runs a command the plugin registered, by id.
    pub fn run_command(&self, id: &str) -> Result<(), String> {
        let command = self
            .registrations
            .commands
            .iter()
            .find(|c| c.id == id)
            .ok_or_else(|| format!("'{id}' is not a command this plugin registered"))?;
        match &command.callback {
            Some(key) => self.run(key, None),
            None => Err(format!("'{}' was registered without a callback", command.id)),
        }
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

    let s = Rc::clone(&shared);
    set!("__silica_raw_read_data", move || -> String {
        std::fs::read_to_string(s.borrow().data_path.clone()).unwrap_or_default()
    });

    let s = Rc::clone(&shared);
    set!("__silica_raw_write_data", move |ctx: Ctx<'js>, text: String| -> rquickjs::Result<()> {
        let path = s.borrow().data_path.clone();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| Exception::throw_message(&ctx, &error.to_string()))?;
        }
        std::fs::write(&path, text).map_err(|error| Exception::throw_message(&ctx, &error.to_string()))
    });

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

    let s = Rc::clone(&shared);
    set!("__silica_raw_vault_read", move |ctx: Ctx<'js>, path: String| -> rquickjs::Result<String> {
        let resolved = s
            .borrow()
            .resolve(&path)
            .map_err(|error| Exception::throw_message(&ctx, &error))?;
        std::fs::read_to_string(&resolved).map_err(|error| {
            Exception::throw_message(&ctx, &format!("'{path}' could not be read: {error}"))
        })
    });

    let s = Rc::clone(&shared);
    set!("__silica_raw_vault_write", move |ctx: Ctx<'js>, path: String, text: String| -> rquickjs::Result<()> {
        s.borrow_mut().mark_changed();
        let resolved = s
            .borrow()
            .writable(&path)
            .map_err(|error| Exception::throw_message(&ctx, &error))?;
        std::fs::write(&resolved, text)
            .map_err(|error| Exception::throw_message(&ctx, &format!("'{path}' could not be written: {error}")))
    });

    let s = Rc::clone(&shared);
    set!("__silica_raw_vault_append", move |ctx: Ctx<'js>, path: String, text: String| -> rquickjs::Result<()> {
        s.borrow_mut().mark_changed();
        let resolved = s
            .borrow()
            .writable(&path)
            .map_err(|error| Exception::throw_message(&ctx, &error))?;
        let mut existing = std::fs::read_to_string(&resolved).unwrap_or_default();
        existing.push_str(&text);
        std::fs::write(&resolved, existing)
            .map_err(|error| Exception::throw_message(&ctx, &format!("'{path}' could not be written: {error}")))
    });

    let s = Rc::clone(&shared);
    set!("__silica_raw_vault_create", move |ctx: Ctx<'js>, path: String, text: String| -> rquickjs::Result<()> {
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
            std::fs::create_dir_all(parent)
                .map_err(|error| Exception::throw_message(&ctx, &error.to_string()))?;
        }
        std::fs::write(&resolved, text)
            .map_err(|error| Exception::throw_message(&ctx, &format!("'{path}' could not be written: {error}")))
    });

    let s = Rc::clone(&shared);
    set!("__silica_raw_vault_create_folder", move |ctx: Ctx<'js>, path: String| -> rquickjs::Result<()> {
        s.borrow_mut().mark_changed();
        let resolved = s
            .borrow()
            .resolve(&path)
            .map_err(|error| Exception::throw_message(&ctx, &error))?;
        std::fs::create_dir_all(&resolved).map_err(|error| {
            Exception::throw_message(&ctx, &format!("'{path}' could not be created: {error}"))
        })
    });

    let s = Rc::clone(&shared);
    set!("__silica_raw_vault_delete", move |ctx: Ctx<'js>, path: String| -> rquickjs::Result<()> {
        s.borrow_mut().mark_changed();
        let resolved = s
            .borrow()
            .writable(&path)
            .map_err(|error| Exception::throw_message(&ctx, &error))?;
        // A plugin deletes into the vault's trash rather than unlinking, so a
        // mistaken delete is recoverable.
        let shared_root = s.borrow().root.clone();
        let trash = shared_root.join(".trash");
        let name = resolved
            .strip_prefix(&shared_root)
            .unwrap_or(&resolved)
            .to_string_lossy()
            .replace('/', "-");
        std::fs::create_dir_all(&trash)
            .map_err(|error| Exception::throw_message(&ctx, &error.to_string()))?;
        std::fs::rename(&resolved, trash.join(name))
            .map_err(|error| Exception::throw_message(&ctx, &format!("'{path}' could not be deleted: {error}")))
    });

    let s = Rc::clone(&shared);
    set!("__silica_raw_vault_rename", move |ctx: Ctx<'js>, from: String, to: String| -> rquickjs::Result<()> {
        s.borrow_mut().mark_changed();
        let source = s
            .borrow()
            .writable(&from)
            .map_err(|error| Exception::throw_message(&ctx, &error))?;
        let target = s
            .borrow()
            .writable(&to)
            .map_err(|error| Exception::throw_message(&ctx, &error))?;
        std::fs::rename(&source, &target)
            .map_err(|error| Exception::throw_message(&ctx, &format!("'{from}' could not be renamed: {error}")))
    });

    let s = Rc::clone(&shared);
    set!("__silica_raw_execute", move |_id: String| -> bool {
        // Commands the app itself owns cannot be reached from a plugin's own
        // runtime without a round trip to the app, which a synchronous call
        // cannot make. Reported as not run rather than silently ignored.
        let _ = &s;
        false
    });

    set!("__silica_raw_post_processor", move |_order: i64, _key: String| {});

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
  // `onload` is usually async and the registrations happen after its first
  // await, so the promise is caught here and its jobs are drained by the
  // caller before the registrations are read.
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
        // A job that throws has already been recorded as the plugin's
        // failure; a job that loops is stopped by the load deadline.
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

/// The vault's files, as `TAbstractFile` reports them. Hidden folders are
/// skipped, which keeps `.obsidian` and `.trash` out of what a plugin sees.
fn collect_files(root: &Path) -> Vec<FileEntry> {
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
            let Ok(kind) = entry.file_type() else { continue };
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
        serde_yaml::Value::Sequence(items) => Value::Array(items.iter().map(yaml_to_json).collect()),
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
