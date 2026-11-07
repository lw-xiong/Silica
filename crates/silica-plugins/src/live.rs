//! The live plugins: the compatibility hosts, on the thread the app runs on.
//!
//! A QuickJS context cannot leave the thread that made it, so a host cannot
//! sit in the vault state. That state is shared with the background executor,
//! and one `!Send` field in it spreads the bound to every task that touches
//! the vault. The hosts live here instead, in a store the app's own thread
//! fills the first time it asks for a plugin.
//!
//! Native plugins (Rust, compiled in) are global — registered with
//! `register_native` at startup, available to every vault session. They sit
//! beside the per-vault compatibility hosts.

use std::cell::RefCell;
use std::rc::Rc;
use std::path::{Path, PathBuf};

use crate::host::{collect_files, FileEntry};
use crate::native::PluginContext;
use crate::{native::Plugin, PluginHost, Registry, Tier};

thread_local! {
    /// `None` until a vault has been asked about, so the first ask loads.
    static LOADED: RefCell<Option<Loaded>> = const { RefCell::new(None) };

    /// Native plugins compiled into Silica, registered via `register_native`
    /// before any vault opens. Global for the process, not per vault.
    static NATIVE: RefCell<Vec<Box<dyn Plugin>>> = const { RefCell::new(Vec::new()) };
}

struct Loaded {
    root: PathBuf,
    /// Which plugins this load was built for, so a settings change is noticed.
    ///
    /// The root alone is not enough: enabling or disabling a plugin does not
    /// change the vault, and keying on the path alone is what made a newly
    /// enabled plugin wait for a vault switch before it existed. This is the
    /// `(id, enabled)` set the hosts were built from — cheap to compare and
    /// exactly what the load depends on.
    key: Vec<(String, bool)>,
    hosts: Vec<PluginHost>,
}

/// The plugin set a load depends on, in a stable order.
fn load_key(registry: &Registry) -> Vec<(String, bool)> {
    let mut key: Vec<(String, bool)> = registry
        .records()
        .iter()
        .filter(|record| record.tier == Tier::Compat)
        .map(|record| (record.id().to_string(), record.enabled))
        .collect();
    key.sort();
    key
}

/// Registers a native plugin compiled into Silica. Called at startup by core
/// plugins and any third-party native plugin linked into the binary. Plugins
/// are available to every vault session once registered.
pub fn register_native(plugin: Box<dyn Plugin>) {
    NATIVE.with(|native| native.borrow_mut().push(plugin));
}

/// Loads every enabled plugin that can run here, for the vault at `root`.
/// Does nothing the second time for the same vault; a different vault loads
/// again, which is what keeps a plugin's runtime off the next vault's.
pub fn ensure(root: &Path, config_dir: &str, registry: &Registry) {
    LOADED.with(|loaded| {
        let mut loaded = loaded.borrow_mut();
        // * Reload when the vault changes *or* when the enabled set does. A
        // * settings change does not touch the vault, so keying on the path
        // * alone left a newly enabled plugin unloaded until the next vault
        // * switch — and a disabled one still running, because its host was
        // * never dropped.
        let key = load_key(registry);
        if loaded
            .as_ref()
            .is_some_and(|loaded| loaded.root == root && loaded.key == key)
        {
            return;
        }
        let plugins = root.join(config_dir).join("plugins");
        // * One file walk serves every plugin instead of each walking alone.
        let shared_files: Option<Rc<RefCell<Vec<FileEntry>>>> = if registry
            .records()
            .iter()
            .any(|record| record.enabled && record.tier == Tier::Compat)
        {
            Some(Rc::new(RefCell::new(collect_files(root))))
        } else {
            None
        };
        let hosts = registry
            .records()
            .iter()
            .filter(|record| record.enabled && record.tier == Tier::Compat)
            .map(|record| {
                let sf = Rc::clone(shared_files.as_ref().unwrap());
                PluginHost::load(
                    root,
                    &plugins.join(&record.folder),
                    &record.manifest,
                    None,
                    Some(sf),
                )
            })
            .collect();
        *loaded = Some(Loaded {
            root: root.to_path_buf(),
            key,
            hosts,
        });
    });
}

/// Every loaded compatibility host, or an empty slice when none has been
/// loaded yet.
pub fn hosts<R>(f: impl FnOnce(&[PluginHost]) -> R) -> R {
    LOADED.with(|loaded| {
        let loaded = loaded.borrow();
        match loaded.as_ref() {
            Some(loaded) => f(&loaded.hosts),
            None => f(&[]),
        }
    })
}

/// Every registered native plugin, or an empty slice when none have been
/// registered yet.
pub fn native<R>(f: impl FnOnce(&[Box<dyn Plugin>]) -> R) -> R {
    NATIVE.with(|native| f(&native.borrow()))
}

/// Runs a plugin's own command, with whatever it announced while running.
/// `None` means no loaded plugin owns that id. Native plugins are checked
/// first, then the compatibility hosts.
pub fn run(
    id: &str,
    value: Option<&serde_json::Value>,
    ctx: &PluginContext<'_>,
) -> Option<(Result<(), String>, Vec<String>)> {
    // * Native plugins are global: check first, running inside the borrow so
    // * we never hold the NATIVE lock across the return.
    let native_hit = NATIVE.with(|native| {
        native
            .borrow()
            .iter()
            .find(|p| p.commands().iter().any(|c| c.id == id))
            .map(|plugin| (plugin.run_command(id, value, ctx), Vec::new()))
    });
    if native_hit.is_some() {
        return native_hit;
    }

    LOADED.with(|loaded| {
        let loaded = loaded.borrow();
        let loaded = loaded.as_ref()?;
        let hosts = &loaded.hosts;
        let host = hosts.iter().find(|host| host.has_command(id))?;
        // ! Tell the host which note is open right before it runs. Without
        // ! this the open-note guard compares against `None` and never fires,
        // ! so a plugin could overwrite unsaved edits.
        host.set_open_note((ctx.current_note)());
        Some((host.run_command(id), host.take_notices()))
    })
}

/// Runs every registered post-processor across all loaded compatibility
/// hosts, in registration order. Returns the count that ran. The render
/// pass calls this for each HTML block so a plugin's post-processor can
/// inspect the rendered output; the DOM element itself is not yet available
/// to a native renderer, so the callback receives no element — the hook is
/// in place for when a render-pass DOM proxy exists.
pub fn run_all_post_processors() -> usize {
    let mut count = 0;
    LOADED.with(|loaded| {
        let loaded = loaded.borrow();
        let Some(loaded) = loaded.as_ref() else {
            return;
        };
        for host in &loaded.hosts {
            for pp in &host.registrations().post_processors {
                if let Some(key) = &pp.callback {
                    let _ = host.run_post_processor(key);
                    count += 1;
                }
            }
        }
    });
    count
}

/// Forgets the loaded compatibility plugins: the next ask loads them again.
/// Native plugins registered with `register_native` are not affected.
pub fn forget() {
    LOADED.with(|loaded| *loaded.borrow_mut() = None);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::NativeCommand;
    use std::fs;

    struct Counter;

    impl Plugin for Counter {
        fn id(&self) -> &'static str {
            "silica:counter"
        }
        fn name(&self) -> &'static str {
            "Counter"
        }
        fn commands(&self) -> Vec<NativeCommand> {
            vec![NativeCommand {
                id: "silica:counter:count".into(),
                name: "Counter: Count notes".into(),
            }]
        }
        fn run_command(
            &self,
            id: &str,
            _value: Option<&serde_json::Value>,
            _ctx: &PluginContext<'_>,
        ) -> Result<(), String> {
            match id {
                "silica:counter:count" => Ok(()),
                other => Err(crate::native::unknown_command(other)),
            }
        }
    }

    /// Enabling and disabling a plugin in an open vault takes effect at once.
    ///
    /// The bug this covers: `ensure` keyed only on the vault path, so a
    /// settings change did not reload anything — the new plugin stayed absent
    /// and the disabled one kept running until a vault switch.
    #[test]
    fn enabling_and_disabling_a_plugin_takes_effect_without_restarting() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let root = dir.path().join("vault");
        let plugin = root.join(".obsidian/plugins/counter");
        fs::create_dir_all(&plugin).expect("the plugin dir");
        fs::write(root.join("Note.md"), "note\n").expect("a note");
        fs::write(
            plugin.join("manifest.json"),
            r#"{"id":"counter","name":"Counter","version":"1.0.0"}"#,
        )
        .expect("a manifest");
        fs::write(
            plugin.join("main.js"),
            r#"
const { Plugin } = require("obsidian");
class Counter extends Plugin {
  onload() { this.addCommand({ id: "hello", name: "Hello", callback: () => {} }); }
}
module.exports = Counter;
"#,
        )
        .expect("the plugin source");

        let enabled_path = root.join(".obsidian/community-plugins.json");
        fs::write(&enabled_path, r#"[]"#).expect("nothing enabled yet");

        let vault = silica_vault::Vault::open(&root).expect("the vault opens");
        forget();

        // * Start with nothing enabled.
        let registry = Registry::scan(&vault);
        ensure(&root, vault.config_dir(), &registry);
        assert_eq!(hosts(|hosts| hosts.len()), 0, "nothing enabled yet");

        // * Enabling it in the same session must load it.
        fs::write(&enabled_path, r#"["counter"]"#).expect("enable it");
        let registry = Registry::scan(&vault);
        ensure(&root, vault.config_dir(), &registry);
        assert_eq!(
            hosts(|hosts| hosts.iter().map(|h| h.id()).collect::<Vec<_>>()),
            ["counter"],
            "an enabled plugin must load without a restart"
        );

        // * And disabling it must drop the host, not merely hide it.
        fs::write(&enabled_path, r#"[]"#).expect("disable it");
        let registry = Registry::scan(&vault);
        ensure(&root, vault.config_dir(), &registry);
        assert_eq!(
            hosts(|hosts| hosts.len()),
            0,
            "a disabled plugin must stop running at once"
        );

        // * Re-enabling works again, so the drop above left nothing broken.
        fs::write(&enabled_path, r#"["counter"]"#).expect("enable it again");
        let registry = Registry::scan(&vault);
        ensure(&root, vault.config_dir(), &registry);
        assert_eq!(hosts(|hosts| hosts.len()), 1, "re-enabling works");
        forget();
    }

    #[test]
    fn ensuring_a_vault_loads_its_enabled_plugin_and_its_command_runs() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let root = dir.path().join("vault");
        let plugin = root.join(".obsidian/plugins/counter");
        fs::create_dir_all(&plugin).expect("the plugin dir");
        fs::write(root.join("Note.md"), "note\n").expect("a note");
        fs::write(
            plugin.join("manifest.json"),
            r#"{"id":"counter","name":"Counter","version":"1.0.0"}"#,
        )
        .expect("a manifest");
        fs::write(
            plugin.join("main.js"),
            r#"
const { Plugin } = require("obsidian");
class Counter extends Plugin {
  onload() {
    this.addCommand({
      id: "hello",
      name: "Hello",
      callback: () => this.app.vault.create("hi.md", "written by the plugin"),
    });
  }
}
module.exports = Counter;
"#,
        )
        .expect("the plugin source");
        fs::write(
            root.join(".obsidian/community-plugins.json"),
            r#"["counter"]"#,
        )
        .expect("the enabled list");

        let vault = silica_vault::Vault::open(&root).expect("the vault opens");
        let registry = Registry::scan(&vault);

        forget();
        ensure(&root, vault.config_dir(), &registry);

        let ids: Vec<String> = hosts(|hosts| hosts.iter().map(|host| host.id()).collect());
        assert_eq!(ids, ["counter"]);
        let loaded = hosts(|hosts| hosts[0].registrations().commands.len());
        assert_eq!(loaded, 1);
        hosts(|hosts| assert!(!hosts[0].failed(), "{:?}", hosts[0].registrations().failure));

        let ctx = PluginContext {
            vault_root: std::path::Path::new(""),
            read_file: Box::new(|_| Err("no vault".to_string())),
            write_file: Box::new(|_, _| Err("no vault".to_string())),
            append_file: Box::new(|_, _| Err("no vault".to_string())),
            create_file: Box::new(|_, _| Err("no vault".to_string())),
            create_folder: Box::new(|_| Err("no vault".to_string())),
            delete_file: Box::new(|_| Err("no vault".to_string())),
            rename_file: Box::new(|_, _| Err("no vault".to_string())),
            list_dir: Box::new(|_| Ok(vec![])),
            file_exists: Box::new(|_| Ok(false)),
            metadata_cache: Box::new(|_| Err("no vault".to_string())),
            open_note: Box::new(|_| Err("no vault".to_string())),
            current_note: Box::new(|| None),
            notify: Box::new(|_| {}),
        };
        let (outcome, _) = run("counter:hello", None, &ctx).expect("the plugin owns this command");
        outcome.expect("the command ran");
        assert_eq!(
            fs::read_to_string(root.join("hi.md")).expect("the note it wrote"),
            "written by the plugin"
        );

        // ! The app's own ids are not the plugin's to answer for.
        let ctx = PluginContext {
            vault_root: std::path::Path::new(""),
            read_file: Box::new(|_| Err("no vault".to_string())),
            write_file: Box::new(|_, _| Err("no vault".to_string())),
            append_file: Box::new(|_, _| Err("no vault".to_string())),
            create_file: Box::new(|_, _| Err("no vault".to_string())),
            create_folder: Box::new(|_| Err("no vault".to_string())),
            delete_file: Box::new(|_| Err("no vault".to_string())),
            rename_file: Box::new(|_, _| Err("no vault".to_string())),
            list_dir: Box::new(|_| Ok(vec![])),
            file_exists: Box::new(|_| Ok(false)),
            metadata_cache: Box::new(|_| Err("no vault".to_string())),
            open_note: Box::new(|_| Err("no vault".to_string())),
            current_note: Box::new(|| None),
            notify: Box::new(|_| {}),
        };
        assert!(run("silica:open-settings", None, &ctx).is_none());
        // * A second ask for the same vault keeps the same runtime.
        ensure(&root, vault.config_dir(), &registry);
        hosts(|hosts| assert_eq!(hosts.len(), 1));
        forget();
        hosts(|hosts| assert!(hosts.is_empty()));
    }

    #[test]
    fn a_native_plugin_command_runs_through_run() {
        register_native(Box::new(Counter));

        // * Native plugins are global: they run without a vault loaded.
        let ctx = PluginContext {
            vault_root: std::path::Path::new(""),
            read_file: Box::new(|_| Err("no vault".to_string())),
            write_file: Box::new(|_, _| Err("no vault".to_string())),
            append_file: Box::new(|_, _| Err("no vault".to_string())),
            create_file: Box::new(|_, _| Err("no vault".to_string())),
            create_folder: Box::new(|_| Err("no vault".to_string())),
            delete_file: Box::new(|_| Err("no vault".to_string())),
            rename_file: Box::new(|_, _| Err("no vault".to_string())),
            list_dir: Box::new(|_| Ok(vec![])),
            file_exists: Box::new(|_| Ok(false)),
            metadata_cache: Box::new(|_| Err("no vault".to_string())),
            open_note: Box::new(|_| Err("no vault".to_string())),
            current_note: Box::new(|| None),
            notify: Box::new(|_| {}),
        };

        // * Native plugins are global: they run without a vault loaded.
        let (outcome, _) =
            run("silica:counter:count", None, &ctx).expect("native plugin owns this command");
        outcome.expect("the command ran");

        // * Unknown id is not owned by any plugin -> None.
        assert!(run("silica:counter:nope", None, &ctx).is_none());

        // * `forget` clears compat hosts, not native registrations.
        forget();
        let (outcome, _) =
            run("silica:counter:count", None, &ctx).expect("native plugin still registered");
        outcome.expect("the command ran again");

        forget_native();
    }

    /// Clears native registrations so tests stay isolated.
    fn forget_native() {
        NATIVE.with(|native| *native.borrow_mut() = Vec::new());
    }

    /// The guard is only real if the host is told what is open. `ensure`
    /// builds hosts with no note open, so a command that writes the open
    /// note must be stopped at call time — the hole this test exists for.
    #[test]
    fn a_command_cannot_write_the_note_open_right_now() {
        let root = std::env::temp_dir().join(format!("silica-live-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".obsidian/plugins/w")).expect("dirs");
        std::fs::write(
            root.join(".obsidian/plugins/w/manifest.json"),
            r#"{"id":"w","name":"W","version":"1.0.0"}"#,
        )
        .expect("manifest");
        std::fs::write(
            root.join(".obsidian/plugins/w/main.js"),
            r#"
const { Plugin } = require("obsidian");
class W extends Plugin {
  onload() { this.addCommand({ id: "hit", name: "Hit",
    callback: () => this.app.vault.modify("Note.md", "clobbered") }); }
}
module.exports = W;
"#,
        )
        .expect("source");
        std::fs::write(root.join("Note.md"), "original").expect("the note");
        std::fs::write(root.join(".obsidian/community-plugins.json"), r#"["w"]"#).expect("enabled");

        let vault = silica_vault::Vault::open(&root).expect("vault");
        let registry = Registry::scan(&vault);
        forget();
        ensure(&root, vault.config_dir(), &registry);

        let open = "Note.md".to_string();
        let ctx = PluginContext {
            vault_root: &root,
            read_file: Box::new(|_| Ok(String::new())),
            write_file: Box::new(|_, _| Ok(())),
            append_file: Box::new(|_, _| Ok(())),
            create_file: Box::new(|_, _| Ok(())),
            create_folder: Box::new(|_| Ok(())),
            delete_file: Box::new(|_| Ok(())),
            rename_file: Box::new(|_, _| Ok(())),
            list_dir: Box::new(|_| Ok(Vec::new())),
            file_exists: Box::new(|_| Ok(true)),
            metadata_cache: Box::new(|_| Ok("{}".into())),
            open_note: Box::new(|_| Ok(())),
            current_note: Box::new(move || Some(open.clone())),
            notify: Box::new(|_| {}),
        };
        let (outcome, _) = run("w:hit", None, &ctx).expect("the plugin owns it");
        assert!(outcome.is_err(), "the write was allowed: {outcome:?}");
        assert_eq!(
            std::fs::read_to_string(root.join("Note.md")).unwrap(),
            "original",
            "the open note was clobbered"
        );
        forget();
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Vault doctor runs end to end over a real vault through `run`'s native
    /// dispatch, with the context backed by std::fs: attachments and dot
    /// folders stay out of the report, a frontmatter-only note counts as
    /// empty, and the report lands in the vault and opens.
    #[test]
    fn vault_doctor_writes_a_report_that_ignores_attachments_and_dot_folders() {
        register_native(Box::new(crate::vault_doctor::VaultDoctor));

        let dir = tempfile::tempdir().expect("a temp dir");
        let root = dir.path().join("vault");
        std::fs::create_dir_all(root.join(".obsidian")).expect("config dir");
        std::fs::write(
            root.join("Note.md"),
            "links to [[Missing]] and ![[pic.png]]\n",
        )
        .expect("a note");
        std::fs::write(root.join("pic.png"), b"png").expect("an attachment");
        std::fs::write(root.join(".obsidian/config.json"), "{}").expect("a config");
        std::fs::write(root.join("Empty.md"), "---\ntitle: empty\n---\n  \n")
            .expect("a frontmatter-only note");

        let read_root = root.clone();
        let list_root = root.clone();
        let write_root = root.clone();
        let ctx = PluginContext {
            vault_root: &root,
            notify: Box::new(|_: &str| {}),
            read_file: Box::new(move |p| {
                fs::read_to_string(read_root.join(p)).map_err(|e| e.to_string())
            }),
            write_file: Box::new(move |p, text| {
                fs::write(write_root.join(p), text).map_err(|e| e.to_string())
            }),
            append_file: Box::new(|_, _| Err("no vault".to_string())),
            create_file: Box::new(|_, _| Err("no vault".to_string())),
            create_folder: Box::new(|_| Err("no vault".to_string())),
            delete_file: Box::new(|_| Err("no vault".to_string())),
            rename_file: Box::new(|_, _| Err("no vault".to_string())),
            list_dir: Box::new(move |d| {
                let base = if d == "." {
                    list_root.clone()
                } else {
                    list_root.join(d)
                };
                let mut out: Vec<String> = Vec::new();
                for entry in fs::read_dir(&base).map_err(|e| e.to_string())? {
                    let entry = entry.map_err(|e| e.to_string())?;
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
                        out.push(format!("{name}/"));
                    } else {
                        out.push(name);
                    }
                }
                Ok(out)
            }),
            file_exists: Box::new(|_| Ok(false)),
            metadata_cache: Box::new(|_| Err("no vault".to_string())),
            open_note: Box::new(|_| Ok(())),
            current_note: Box::new(|| None),
        };

        let (outcome, _) = run("vault-doctor:run", None, &ctx).expect("the native plugin owns it");
        outcome.expect("the command ran");

        let report = fs::read_to_string(root.join("Vault doctor.md")).expect("the report");
        assert!(
            report.contains("`[[Missing]]`"),
            "the broken link is listed: {report}"
        );
        assert!(
            !report.contains("pic.png"),
            "the attachment is not a broken link: {report}"
        );
        assert!(
            !report.contains("config.json"),
            "dot folders are not scanned: {report}"
        );
        assert!(
            report.contains("[[Empty]]"),
            "the frontmatter-only note is empty: {report}"
        );

        forget_native();
    }
}
