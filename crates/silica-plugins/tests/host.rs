//! The host, exercised end to end on a plugin written the way a vault-only
//! Obsidian plugin is written: async `onload`, commands, a ribbon item, a
//! settings tab built out of `Setting`, and the vault's own API.

use std::fs;
use std::path::PathBuf;

use silica_formats::config::PluginManifest;
use silica_plugins::PluginHost;

struct Fixture {
    _dir: tempfile::TempDir,
    vault: PathBuf,
    plugin: PathBuf,
}

fn fixture(source: &str) -> Fixture {
    let dir = tempfile::tempdir().expect("a temp dir");
    let vault = dir.path().join("vault");
    let plugin = vault.join(".obsidian/plugins/counter");
    fs::create_dir_all(&plugin).expect("the plugin dir");
    fs::write(
        vault.join("Note.md"),
        "---\ntags: [one]\n---\n\n# Title\n\nLinks to [[Other]] and #two\n",
    )
    .expect("a note");
    fs::write(vault.join("Other.md"), "other\n").expect("another note");
    fs::write(
        plugin.join("manifest.json"),
        r#"{"id":"counter","name":"Counter","version":"1.0.0"}"#,
    )
    .expect("a manifest");
    fs::write(plugin.join("main.js"), source).expect("the plugin source");
    Fixture {
        _dir: dir,
        vault,
        plugin,
    }
}

fn manifest() -> PluginManifest {
    PluginManifest {
        id: "counter".into(),
        name: "Counter".into(),
        version: "1.0.0".into(),
        ..PluginManifest::default()
    }
}

const COUNTER: &str = r#"
const { Plugin, PluginSettingTab, Setting, Notice } = require("obsidian");

class CounterTab extends PluginSettingTab {
  display() {
    this.containerEl.createEl("h2", { text: "Counter" });
    new Setting(this.containerEl)
      .setName("Enabled")
      .setDesc("Count things when opened")
      .addToggle((toggle) => toggle.setValue(true).onChange((value) => { this.plugin.settings.enabled = value; }));
    new Setting(this.containerEl)
      .setName("Prefix")
      .setDesc("Prepended to a new note's name")
      .addText((text) => text.setPlaceholder("count-").setValue(this.plugin.settings.prefix));
    new Setting(this.containerEl)
      .setName("Mode")
      .addDropdown((drop) => drop.addOptions({ fast: "Fast", careful: "Careful" }));
    new Setting(this.containerEl)
      .setName("Rebuild")
      .addButton((button) => button.setButtonText("Rebuild").onClick(() => this.plugin.count()));
  }
}

class Counter extends Plugin {
  async onload() {
    this.settings = (await this.loadData()) || { enabled: true, prefix: "count-" };
    const notes = this.app.vault.getMarkdownFiles();
    this.addCommand({ id: "count-notes", name: "Count notes", callback: () => this.count() });
    this.addCommand({
      id: "create-file",
      name: "Create a note",
      callback: () => this.app.vault.create(this.settings.prefix + "new.md", "created by counter"),
    });
    this.addCommand({
      id: "overwrite-open",
      name: "Overwrite the open note",
      callback: () => {
        try {
          this.app.vault.modify(this.app.workspace.getActiveFile(), "clobbered");
        } catch (error) {
          new Notice("refused: " + error.message);
        }
      },
    });
    this.addCommand({
      id: "escape",
      name: "Read outside the vault",
      callback: () => {
        try {
          this.app.vault.read("../../secret.txt");
        } catch (error) {
          new Notice("refused: " + error.message);
        }
      },
    });
    this.addRibbonIcon("dice", "Count notes", () => this.count());
    this.addSettingTab(new CounterTab(this.app, this));
    this.registerMarkdownPostProcessor((el) => el);
    this.registerMarkdownCodeBlockProcessor("counter", (source) => source);
    this.app.workspace.onLayoutReady(() => { this.ready = true; });
    await this.saveData(this.settings);
    new Notice(notes.length + " notes ready");
  }

  count() {
    new Notice(this.app.vault.getMarkdownFiles().length + " notes");
  }
}

module.exports = Counter;
"#;

fn load(fixture: &Fixture, open_note: Option<&str>) -> PluginHost {
    PluginHost::load(
        &fixture.vault,
        &fixture.plugin,
        &manifest(),
        open_note.map(|note| note.to_string()),
    )
}

#[test]
fn a_vault_only_plugin_registers_what_it_added() {
    let fixture = fixture(COUNTER);
    let host = load(&fixture, None);
    assert!(!host.failed(), "load failed: {:?}", host.registrations().failure);

    let registered = host.registrations();
    let ids: Vec<&str> = registered.commands.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "counter:count-notes",
            "counter:create-file",
            "counter:overwrite-open",
            "counter:escape",
        ]
    );
    assert_eq!(registered.commands[0].name, "Counter: Count notes");
    assert!(registered.commands[0].callback.is_some());

    assert_eq!(registered.ribbon.len(), 1);
    assert_eq!(registered.ribbon[0].icon, "dice");
    assert_eq!(registered.ribbon[0].title, "Count notes");

    assert_eq!(registered.post_processors.len(), 1);
    assert_eq!(registered.code_blocks[0].language, "counter");

    // The settings tab is the rows its own `display()` produced.
    assert_eq!(registered.tabs.len(), 1);
    let tab = &registered.tabs[0];
    assert_eq!(tab.name, "Counter");
    let controls: Vec<&str> = tab.rows.iter().map(|r| r.control.as_str()).collect();
    assert_eq!(controls, ["none", "toggle", "text", "dropdown", "button"]);
    assert_eq!(tab.rows[0].kind, "heading");
    assert_eq!(tab.rows[0].name, "Counter");
    assert_eq!(tab.rows[1].name, "Enabled");
    assert_eq!(tab.rows[1].desc, "Count things when opened");
    assert_eq!(tab.rows[1].value, serde_json::json!(true));
    assert_eq!(tab.rows[2].placeholder.as_deref(), Some("count-"));
    assert_eq!(tab.rows[3].options.len(), 2);
    assert_eq!(tab.rows[4].value, serde_json::json!("Rebuild"));

    // What the plugin saved is on disk, where a real Obsidian would find it.
    let data = fs::read_to_string(fixture.plugin.join("data.json")).expect("data.json written");
    assert!(data.contains("\"prefix\":\"count-\""), "{data}");

    let notices = host.take_notices();
    assert!(notices.iter().any(|n| n == "2 notes ready"), "{notices:?}");
    assert!(host.take_notices().is_empty(), "notices drain");
}

#[test]
fn running_a_command_does_the_work() {
    let fixture = fixture(COUNTER);
    let host = load(&fixture, None);

    host.run_command("counter:create-file").expect("the command ran");
    let created = fixture.vault.join("count-new.md");
    assert_eq!(
        fs::read_to_string(&created).expect("the note"),
        "created by counter"
    );

    host.run_command("counter:count-notes").expect("the command ran");
    assert!(host.take_notices().iter().any(|n| n == "3 notes"));

    let unregistered = host.run_command("not-a-command").unwrap_err();
    assert!(
        unregistered.contains("not a command this plugin registered"),
        "{unregistered}"
    );
}

#[test]
fn a_plugin_that_requires_electron_is_refused_with_the_reason() {
    let fixture = fixture(
        r#"
const { Plugin, Notice } = require("obsidian");
const { shell } = require("electron");
class NeedsElectron extends Plugin {
  onload() { this.addCommand({ id: "open", name: "Open", callback: () => new Notice("hi") }); }
}
module.exports = NeedsElectron;
"#,
    );
    let host = load(&fixture, None);
    assert!(host.failed());
    let reason = host.registrations().failure.clone().unwrap();
    assert!(reason.contains("cannot require the module 'electron'"), "{reason}");
    assert!(host.registrations().commands.is_empty(), "nothing half-registered");
}

#[test]
fn onload_that_throws_is_a_reason_not_a_crash() {
    let fixture = fixture(
        r#"
const { Plugin } = require("obsidian");
class Broken extends Plugin {
  onload() { throw new Error("the vault has no calendar"); }
}
module.exports = Broken;
"#,
    );
    let host = load(&fixture, None);
    let reason = host.registrations().failure.clone().unwrap();
    assert!(reason.contains("the vault has no calendar"), "{reason}");
}

#[test]
fn a_write_to_the_note_open_in_the_editor_is_refused() {
    let fixture = fixture(COUNTER);
    let host = load(&fixture, Some("Note.md"));
    let before = fs::read_to_string(fixture.vault.join("Note.md")).expect("the note");

    host.run_command("counter:overwrite-open").expect("the command ran");

    assert_eq!(
        fs::read_to_string(fixture.vault.join("Note.md")).expect("the note"),
        before
    );
    let notices = host.take_notices();
    assert!(notices.iter().any(|n| n.contains("open in the editor")), "{notices:?}");
}

#[test]
fn a_path_out_of_the_vault_is_refused() {
    let fixture = fixture(COUNTER);
    let host = load(&fixture, None);
    host.run_command("counter:escape").expect("the command ran");
    let notices = host.take_notices();
    assert!(
        notices.iter().any(|n| n.contains("points outside the vault")),
        "{notices:?}"
    );
}

#[test]
fn the_metadata_cache_answers_in_obsidians_shape() {
    let fixture = fixture(COUNTER);
    let host = load(&fixture, None);
    let cache = host.metadata("Note.md").expect("the cache entry");

    assert_eq!(cache["frontmatter"]["tags"], serde_json::json!(["one"]));
    assert_eq!(cache["headings"][0]["heading"], "Title");
    assert_eq!(cache["headings"][0]["level"], 1);
    assert_eq!(cache["headings"][0]["position"]["start"]["line"], 4);
    assert_eq!(cache["tags"][0]["tag"], "#two");
    assert!(cache["links"]
        .as_array()
        .expect("links")
        .iter()
        .any(|link| link["link"] == "Other"));
    assert!(cache["sections"].as_array().expect("sections").len() >= 2);
}
