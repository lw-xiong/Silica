//! The installed community plugins of one vault: each one found, classified
//! into a [`Tier`], and marked enabled or not — but none of them started.
//!
//! This is the pass that runs before any host does. It reads each plugin's
//! `manifest.json` and a bounded prefix of its `main.js`, decides what Silica
//! can do with it, and hands the answer to Settings > Community plugins, so
//! the list says what will run and what will not before the user turns
//! anything on.

use std::io::Read;
use std::path::Path;

use silica_formats::config::PluginManifest;
use silica_vault::Vault;

use crate::tier::{classify, Tier};

/// How much of a plugin's `main.js` the classifier reads.
///
/// A bundled plugin is one file and its imports sit at the top, so the head
/// answers the questions the scan asks. The cap keeps a huge bundle from
/// being read whole just to be classified.
pub const SCAN_PREFIX: u64 = 256 * 1024;

/// One installed plugin, as the Community plugins list needs it.
#[derive(Debug, Clone, PartialEq)]
pub struct PluginRecord {
    /// The folder under `plugins/`. The manifest's id is used when it has
    /// one; the folder is always there.
    pub folder: String,
    pub manifest: PluginManifest,
    pub tier: Tier,
    /// In `community-plugins.json`, the same list Obsidian reads.
    pub enabled: bool,
}

impl PluginRecord {
    /// The id the plugin is known by: its manifest id when set, else the
    /// folder. Matches how [`Registry::scan`] resolves enabling.
    pub fn id(&self) -> &str {
        if self.manifest.id.is_empty() {
            &self.folder
        } else {
            &self.manifest.id
        }
    }
}

/// Every plugin installed in a vault.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Registry {
    records: Vec<PluginRecord>,
}

impl Registry {
    /// Scans `<config>/plugins/`, classifying each folder that holds a
    /// readable `manifest.json`. Folders without one are skipped, the way
    /// [`Vault::plugin_manifests`] skips them.
    pub fn scan(vault: &Vault) -> Registry {
        let enabled = vault.community_plugins().unwrap_or_default();
        let dir = vault.root().join(vault.config_dir()).join("plugins");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return Registry::default();
        };
        let mut records: Vec<PluginRecord> = entries
            .flatten()
            .filter(|e| e.path().is_dir())
            .filter_map(|e| {
                let folder = e.file_name().to_string_lossy().into_owned();
                let text = std::fs::read_to_string(e.path().join("manifest.json")).ok()?;
                let mut manifest: PluginManifest = serde_json::from_str(&text).ok()?;
                if manifest.id.is_empty() {
                    manifest.id = folder.clone();
                }
                // Silica's own marker: a plugin that targets Silica, not
                // Obsidian, and runs in-process.
                let native = e.path().join("silica.json").exists();
                let source = read_prefix(&e.path().join("main.js"), SCAN_PREFIX);
                let tier = classify(&manifest, &source, native);
                let enabled = enabled.iter().any(|id| *id == manifest.id);
                Some(PluginRecord {
                    folder,
                    manifest,
                    tier,
                    enabled,
                })
            })
            .collect();
        records.sort_by(|a, b| a.folder.cmp(&b.folder));
        Registry { records }
    }

    pub fn records(&self) -> &[PluginRecord] {
        &self.records
    }

    /// The plugins in `community-plugins.json`, whatever their tier. What
    /// actually runs is a compat plugin that is enabled; a refused one is
    /// listed with its reason instead.
    pub fn enabled(&self) -> impl Iterator<Item = &PluginRecord> {
        self.records.iter().filter(|r| r.enabled)
    }

    pub fn get(&self, id: &str) -> Option<&PluginRecord> {
        self.records.iter().find(|r| r.id() == id)
    }

    /// How many plugins would run in the compatibility host.
    pub fn compat_count(&self) -> usize {
        self.records
            .iter()
            .filter(|r| r.tier == Tier::Compat)
            .count()
    }
}

/// The first `limit` bytes of a file as text, or empty when it cannot be read.
/// Lossy on a split UTF-8 sequence at the cap, which a scan for ASCII
/// `require("…")` and `document.` does not care about.
fn read_prefix(path: &Path, limit: u64) -> String {
    let Ok(file) = std::fs::File::open(path) else {
        return String::new();
    };
    let mut buf = Vec::new();
    let _ = file.take(limit).read_to_end(&mut buf);
    String::from_utf8_lossy(&buf).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// A vault with three plugins: one vault-only, one that needs Node, and
    /// one that targets Silica. The first is enabled.
    fn vault_with_plugins() -> (tempfile::TempDir, Vault) {
        let dir = tempfile::tempdir().unwrap();
        let plugins = dir.path().join(".obsidian/plugins");
        let write = |folder: &str, manifest: &str, main: &str, marker: bool| {
            let p = plugins.join(folder);
            fs::create_dir_all(&p).unwrap();
            fs::write(p.join("manifest.json"), manifest).unwrap();
            if !main.is_empty() {
                fs::write(p.join("main.js"), main).unwrap();
            }
            if marker {
                fs::write(p.join("silica.json"), "{}").unwrap();
            }
        };
        write(
            "dataview",
            r#"{"id":"dataview","name":"Dataview","version":"0.5.0"}"#,
            r#"const {Plugin}=require("obsidian");module.exports=class extends Plugin{}"#,
            false,
        );
        write(
            "kanban",
            r#"{"id":"kanban","name":"Kanban","version":"1.0.0","isDesktopOnly":true}"#,
            r#"require("fs")"#,
            false,
        );
        write(
            "silica-tags",
            r#"{"id":"silica-tags","name":"Silica Tags","version":"0.1.0"}"#,
            "",
            true,
        );
        fs::write(
            dir.path().join(".obsidian/community-plugins.json"),
            r#"["dataview"]"#,
        )
        .unwrap();
        let vault = Vault::open(dir.path()).unwrap();
        (dir, vault)
    }

    #[test]
    fn scan_classifies_every_installed_plugin() {
        let (_dir, vault) = vault_with_plugins();
        let reg = Registry::scan(&vault);
        assert_eq!(reg.records().len(), 3);
        assert_eq!(reg.get("dataview").unwrap().tier, Tier::Compat);
        assert_eq!(
            reg.get("kanban").unwrap().tier,
            Tier::Refused(crate::tier::Refusal::DesktopOnly)
        );
        assert_eq!(reg.get("silica-tags").unwrap().tier, Tier::Native);
        assert_eq!(reg.compat_count(), 1);
    }

    #[test]
    fn enabled_follows_community_plugins_json() {
        let (_dir, vault) = vault_with_plugins();
        let reg = Registry::scan(&vault);
        let on: Vec<&str> = reg.enabled().map(|r| r.id()).collect();
        assert_eq!(on, ["dataview"]);
        assert!(!reg.get("kanban").unwrap().enabled);
    }

    #[test]
    fn a_vault_with_no_plugins_scans_to_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::open(dir.path()).unwrap();
        assert!(Registry::scan(&vault).records().is_empty());
    }

    #[test]
    fn a_folder_without_a_manifest_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".obsidian/plugins/stray")).unwrap();
        let vault = Vault::open(dir.path()).unwrap();
        assert!(Registry::scan(&vault).records().is_empty());
    }
}
