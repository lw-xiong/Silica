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
use std::path::{Path, PathBuf};

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
    hosts: Vec<PluginHost>,
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
        if loaded.as_ref().is_some_and(|loaded| loaded.root == root) {
            return;
        }
        let plugins = root.join(config_dir).join("plugins");
        let hosts = registry
            .records()
            .iter()
            .filter(|record| record.enabled && record.tier == Tier::Compat)
            .map(|record| {
                PluginHost::load(root, &plugins.join(&record.folder), &record.manifest, None)
            })
            .collect();
        *loaded = Some(Loaded {
            root: root.to_path_buf(),
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
pub fn run(id: &str) -> Option<(Result<(), String>, Vec<String>)> {
    // Native plugins are global: check first, running inside the borrow so
    // we never hold the NATIVE lock across the return.
    let native_hit = NATIVE.with(|native| {
        native.borrow().iter().find(|p| {
            p.commands().iter().any(|c| c.id == id)
        }).map(|plugin| (plugin.run_command(id, None), Vec::new()))
    });
    if native_hit.is_some() {
        return native_hit;
    }

    LOADED.with(|loaded| {
        let loaded = loaded.borrow();
        let loaded = loaded.as_ref()?;
        let hosts = &loaded.hosts;
        let host = hosts.iter().find(|host| host.has_command(id))?;
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
        fn run_command(&self, id: &str, _value: Option<&serde_json::Value>) -> Result<(), String> {
            match id {
                "silica:counter:count" => Ok(()),
                other => Err(format!("'{other}' is not a command this plugin registered")),
            }
        }
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
        fs::write(root.join(".obsidian/community-plugins.json"), r#"["counter"]"#)
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

        let (outcome, _) = run("counter:hello").expect("the plugin owns this command");
        outcome.expect("the command ran");
        assert_eq!(
            fs::read_to_string(root.join("hi.md")).expect("the note it wrote"),
            "written by the plugin"
        );

        // The app's own ids are not the plugin's to answer for.
        assert!(run("silica:open-settings").is_none());
        // A second ask for the same vault keeps the same runtime.
        ensure(&root, vault.config_dir(), &registry);
        hosts(|hosts| assert_eq!(hosts.len(), 1));
        forget();
        hosts(|hosts| assert!(hosts.is_empty()));
    }

    #[test]
    fn a_native_plugin_command_runs_through_run() {
        register_native(Box::new(Counter));

        // Native plugins are global: they run without a vault loaded.
        let (outcome, _) = run("silica:counter:count").expect("native plugin owns this command");
        outcome.expect("the command ran");

        // Unknown id is not owned by any plugin -> None.
        assert!(run("silica:counter:nope").is_none());

        // `forget` clears compat hosts, not native registrations.
        forget();
        let (outcome, _) = run("silica:counter:count").expect("native plugin still registered");
        outcome.expect("the command ran again");

        forget_native();
    }

    /// Clears native registrations so tests stay isolated.
    fn forget_native() {
        NATIVE.with(|native| *native.borrow_mut() = Vec::new());
    }
}
