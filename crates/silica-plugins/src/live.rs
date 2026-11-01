//! The live plugins: the compatibility hosts, on the thread the app runs on.
//!
//! A QuickJS context cannot leave the thread that made it, so a host cannot
//! sit in the vault state. That state is shared with the background executor,
//! and one `!Send` field in it spreads the bound to every task that touches
//! the vault. The hosts live here instead, in a store the app's own thread
//! fills the first time it asks for a plugin.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use crate::{PluginHost, Registry, Tier};

thread_local! {
    /// `None` until a vault has been asked about, so the first ask loads.
    static LOADED: RefCell<Option<Loaded>> = const { RefCell::new(None) };
}

struct Loaded {
    root: PathBuf,
    hosts: Vec<PluginHost>,
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

/// Every loaded plugin, or an empty slice when none has been loaded yet.
pub fn hosts<R>(f: impl FnOnce(&[PluginHost]) -> R) -> R {
    LOADED.with(|loaded| {
        let loaded = loaded.borrow();
        match loaded.as_ref() {
            Some(loaded) => f(&loaded.hosts),
            None => f(&[]),
        }
    })
}

/// Runs a plugin's own command, with whatever it announced while running.
/// `None` means no loaded plugin owns that id.
pub fn run(id: &str) -> Option<(Result<(), String>, Vec<String>)> {
    LOADED.with(|loaded| {
        let loaded = loaded.borrow();
        let hosts = &loaded.as_ref()?.hosts;
        let host = hosts.iter().find(|host| host.has_command(id))?;
        Some((host.run_command(id), host.take_notices()))
    })
}

/// Forgets the loaded plugins: the next ask loads them again.
pub fn forget() {
    LOADED.with(|loaded| *loaded.borrow_mut() = None);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

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
}
