//! Which tier an installed community plugin lands in, decided from its
//! `manifest.json` and a bounded prefix of its `main.js` — without starting
//! any of it.
//!
//! Obsidian's plugin API is TypeScript over Electron: the DOM and CodeMirror
//! for anything that draws, Node and Electron for anything that reaches the
//! filesystem or the OS. Silica has neither, so a plugin that needs one of
//! them cannot run here however the host is built, and the useful thing to do
//! is say so before loading it. What is left — a plugin that touches only the
//! vault and its index — is the set the compatibility host can run, and this
//! module is where that set is drawn.
//!
//! The scan is deliberately conservative: a construct that *usually* means DOM
//! work refuses the plugin rather than risk running one that draws nothing.
//! See `docs/silica/Plugins.md` for the tiers and the host they feed.

use silica_formats::config::PluginManifest;

/// What Silica can do with an installed plugin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tier {
    /// A plugin Silica ships or one that targets Silica: Rust in-process, no
    /// bridge and no sandbox.
    Native,
    /// JavaScript that touches `app` and the vault index only. The QuickJS
    /// host can run it.
    Compat,
    /// Cannot run here; [`Refusal`] is what the user is told.
    Refused(Refusal),
}

impl Tier {
    /// The label the Settings > Community plugins list shows beside a plugin.
    pub fn label(&self) -> &'static str {
        match self {
            Tier::Native => "Runs natively",
            Tier::Compat => "Runs in the compatibility host",
            Tier::Refused(_) => "Does not run here",
        }
    }
}

/// Why a plugin cannot run here. Each variant is a *kind* of dependency the
/// native app cannot satisfy, so the message can name it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// `isDesktopOnly` in the manifest: it expects Electron or Node.
    DesktopOnly,
    /// Requires a Node or Electron module, named as the source names it.
    Node(String),
    /// Drives CodeMirror, which the native editor replaces.
    CodeMirror,
    /// Touches the DOM, which the native renderer serves and the host does not
    /// shim.
    Dom,
}

impl Refusal {
    /// The sentence the Community plugins list shows under a refused plugin.
    pub fn reason(&self) -> String {
        match self {
            Refusal::DesktopOnly => {
                "Marked desktop-only; it expects Electron APIs a native app does not have."
                    .to_string()
            }
            Refusal::Node(module) => {
                format!("Requires the Node module `{module}`, which a native app has no runtime for.")
            }
            Refusal::CodeMirror => {
                "Drives CodeMirror, which the native editor replaces.".to_string()
            }
            Refusal::Dom => {
                "Draws into the DOM, which the native renderer serves.".to_string()
            }
        }
    }
}

/// Node and Electron modules that no native app can hand a plugin.
///
/// A bundled `main.js` reaches for these through `require("…")`; Electron's
/// own module is the flag that matters most, so it is checked by name.
const NODE_MODULES: &[&str] = &[
    "child_process",
    "cluster",
    "crypto",
    "dgram",
    "electron",
    "fs",
    "http",
    "https",
    "module",
    "net",
    "os",
    "path",
    "process",
    "stream",
    "tls",
    "url",
    "v8",
    "worker_threads",
    "zlib",
];

/// Constructs that mean the plugin drives CodeMirror, which the native editor
/// replaces rather than wraps.
const CODEMIRROR: &[&str] = &[
    "@codemirror/",
    "CodeMirror",
    "EditorView",
    "StateField",
    "ViewPlugin",
];

/// Constructs that mean the plugin draws into or reads the DOM.
/// What means the plugin draws for itself, and cannot be served by a native
/// renderer. The element helpers a settings tab builds its rows with
/// (`createEl`, `createDiv`, `createSpan`) are deliberately absent: the shim
/// serves those as rows, which is how a vault-only plugin with a settings tab
/// stays runnable. A markdown post-processor is present because its output is
/// a node the reading view would have to place.
const DOM: &[&str] = &[
    "addEventListener(",
    "createElement(",
    "document.",
    "document[",
    "getElementById",
    "HTMLElement",
    "innerHTML",
    "MarkdownRenderer",
    "querySelector",
    "registerMarkdownPostProcessor",
    "window.",
];

/// Decides a plugin's [`Tier`] from its manifest and the text of its
/// `main.js`. `native` is true when the plugin folder carries Silica's own
/// marker, which is how a plugin that targets Silica is recognised.
///
/// The manifest's `isDesktopOnly` is checked first because it is the plugin
/// saying so itself, and a manifest that says so is believed over any scan of
/// the source.
pub fn classify(manifest: &PluginManifest, source: &str, native: bool) -> Tier {
    if native {
        return Tier::Native;
    }
    if manifest.is_desktop_only == Some(true) {
        return Tier::Refused(Refusal::DesktopOnly);
    }
    if let Some(module) = required_node_module(source) {
        return Tier::Refused(Refusal::Node(module));
    }
    if contains_any(source, CODEMIRROR) {
        return Tier::Refused(Refusal::CodeMirror);
    }
    if contains_any(source, DOM) {
        return Tier::Refused(Refusal::Dom);
    }
    Tier::Compat
}

/// The first Node or Electron module the source requires, if any.
///
/// Scans `require("name")` and `require('name')`; a bundled Obsidian plugin is
/// one file and reaches for the runtime through `require`, so this is where a
/// Node dependency shows. Matches both `fs` and `node:fs`.
pub fn required_node_module(source: &str) -> Option<String> {
    let mut rest = source;
    while let Some(at) = rest.find("require") {
        rest = &rest[at + "require".len()..];
        let after = rest.trim_start();
        if let Some(arg) = quoted(after) {
            let name = arg.strip_prefix("node:").unwrap_or(&arg);
            if NODE_MODULES.contains(&name) {
                return Some(arg.to_string());
            }
        }
    }
    None
}

/// The single-quoted or double-quoted string that starts `s` once its opening
/// `(` is consumed, if it starts with one.
fn quoted(s: &str) -> Option<String> {
    let s = s.strip_prefix('(')?;
    let quote = s.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let rest = &s[1..];
    let end = rest.find(quote)?;
    Some(rest[..end].to_string())
}

/// Whether the source mentions any of `needles`.
fn contains_any(source: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| source.contains(n))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(desktop_only: Option<bool>) -> PluginManifest {
        PluginManifest {
            id: "x".to_string(),
            is_desktop_only: desktop_only,
            ..PluginManifest::default()
        }
    }

    #[test]
    fn a_manifest_that_says_desktop_only_is_refused_as_such() {
        assert_eq!(
            classify(&manifest(Some(true)), "", false),
            Tier::Refused(Refusal::DesktopOnly)
        );
    }

    #[test]
    fn desktop_only_is_believed_over_a_clean_source() {
        // The manifest is the plugin's own statement; a scan that finds
        // nothing must not overrule it.
        assert_eq!(
            classify(&manifest(Some(true)), "module.exports = {}", false),
            Tier::Refused(Refusal::DesktopOnly)
        );
    }

    #[test]
    fn a_vault_only_plugin_is_compat() {
        let src = r#"
            const { Plugin } = require("obsidian");
            module.exports = class extends Plugin {
                async onload() {
                    this.addCommand({ id: "go", name: "Go", callback: () => {} });
                    const f = this.app.vault.getMarkdownFiles();
                    this.app.metadataCache.getFileCache(f[0]);
                }
            };
        "#;
        assert_eq!(classify(&manifest(None), src, false), Tier::Compat);
    }

    #[test]
    fn requiring_a_node_module_is_refused_and_named() {
        let tier = classify(&manifest(None), r#"require("fs")"#, false);
        assert_eq!(tier, Tier::Refused(Refusal::Node("fs".to_string())));
        // `node:` prefixed names are the same module.
        assert_eq!(
            classify(&manifest(None), r#"require('node:child_process')"#, false),
            Tier::Refused(Refusal::Node("node:child_process".to_string()))
        );
    }

    #[test]
    fn requiring_obsidian_itself_is_not_a_node_dependency() {
        assert_eq!(required_node_module(r#"require("obsidian")"#), None);
    }

    #[test]
    fn codemirror_is_checked_before_the_dom() {
        // A plugin that drives CodeMirror touches the DOM too; the more
        // specific reason is the one worth showing.
        let tier = classify(&manifest(None), "new EditorView({})", false);
        assert_eq!(tier, Tier::Refused(Refusal::CodeMirror));
    }

    #[test]
    fn drawing_into_the_dom_is_refused() {
        let tier = classify(&manifest(None), "el.innerHTML = '<b>x</b>'", false);
        assert_eq!(tier, Tier::Refused(Refusal::Dom));
        let post_processor = classify(
            &manifest(None),
            "this.registerMarkdownPostProcessor((el) => el)",
            false,
        );
        assert_eq!(post_processor, Tier::Refused(Refusal::Dom));
    }

    #[test]
    fn a_settings_tab_is_not_dom_work() {
        // The shim turns these into rows; refusing them would refuse almost
        // every vault-only plugin, which is the tier that can run.
        let source = r#"
const { Plugin, PluginSettingTab, Setting } = require("obsidian");
class Tab extends PluginSettingTab {
  display() {
    this.containerEl.empty();
    this.containerEl.createEl("h2", { text: "Counter" });
    const row = this.containerEl.createDiv({ cls: "setting-item" });
    row.createSpan({ text: "Enabled" });
    new Setting(this.containerEl).setName("Enabled").addToggle((t) => t.setValue(true));
  }
}
class P extends Plugin { onload() { this.addSettingTab(new Tab(this.app, this)); } }
module.exports = P;
"#;
        assert_eq!(classify(&manifest(None), source, false), Tier::Compat);
    }

    #[test]
    fn the_native_marker_wins_over_everything() {
        assert_eq!(
            classify(&manifest(Some(true)), r#"require("fs")"#, true),
            Tier::Native
        );
    }

    #[test]
    fn every_tier_has_a_label_and_every_refusal_a_reason() {
        for tier in [
            Tier::Native,
            Tier::Compat,
            Tier::Refused(Refusal::DesktopOnly),
            Tier::Refused(Refusal::Node("fs".to_string())),
            Tier::Refused(Refusal::CodeMirror),
            Tier::Refused(Refusal::Dom),
        ] {
            assert!(!tier.label().is_empty());
            if let Tier::Refused(r) = &tier {
                assert!(!r.reason().is_empty());
            }
        }
    }
}
