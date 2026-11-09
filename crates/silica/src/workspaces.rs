//! Named workspace layouts, kept in the config vault.
//!
//! The Workspaces feature: the layout the session would have written to
//! `workspace.json` — the main area's tab groups and how they are split,
//! the active leaf, the sidebars' collapse state and their tabs,
//! `lastOpenFiles` — recorded whole under a name in `workspaces.json`, so
//! a saved layout can be put back on screen later, in any session.
//!
//! A vault with no `workspaces.json` — every vault that predates the
//! feature — reads as the empty store: nothing saved, nothing to load,
//! and every other config file untouched.

use indexmap::IndexMap;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

use crate::workspace::VaultState;

/// The config file the saved layouts live in, in the vault's config
/// folder, the way Obsidian's own Workspaces plugin keeps them.
pub const WORKSPACES_FILE: &str = "workspaces.json";

/// `workspaces.json`: the named layouts, and the one the session last
/// saved or loaded.
///
/// The map keeps the file's order — a layout saved second is listed
/// second, and overwriting one keeps its place — so the order the layouts
/// were saved in is the order the load prompt offers them in.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceStore {
    #[serde(default, deserialize_with = "null_reads_as_empty")]
    pub workspaces: IndexMap<String, SavedWorkspace>,
    /// The layout the session last saved, the way Obsidian marks it;
    /// `None` while the feature has not been used, or when the layout it
    /// named has been taken out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// A `null` map reads as the empty one: nothing names a layout, so there
/// is nothing to load, which is what a fresh vault means.
fn null_reads_as_empty<'de, D>(d: D) -> Result<IndexMap<String, SavedWorkspace>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Option::deserialize(d)?.unwrap_or_default())
}

/// One saved layout: a `workspace.json` document, flattened, so the entry
/// *is* the layout — the shape Obsidian writes. Keys a newer build added
/// stay in the map and are written back unchanged.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SavedWorkspace {
    #[serde(flatten)]
    pub layout: Map<String, Value>,
}

impl WorkspaceStore {
    /// The saved names, in the order they were saved.
    pub fn names(&self) -> Vec<&str> {
        self.workspaces.keys().map(String::as_str).collect()
    }

    /// The layout a name holds, as the `workspace.json` document it was
    /// saved from, or `None` when no layout goes by that name.
    pub fn layout(&self, name: &str) -> Option<Value> {
        self.workspaces
            .get(name)
            .map(|saved| Value::Object(saved.layout.clone()))
    }
}

/// The name a layout is saved under: trimmed, or `None` when nothing is
/// left to name it or the name cannot be one — a path separator would put
/// the entry where a name is not read back from, and a control character
/// is not something a person typed on purpose.
pub fn normalized_name(name: &str) -> Option<String> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    if name
        .chars()
        .any(|c| c.is_control() || c == '/' || c == '\\')
    {
        return None;
    }
    Some(name.to_string())
}

impl VaultState {
    /// The saved layouts, or the empty store when the vault has no
    /// `workspaces.json`.
    pub fn workspaces(&self) -> Result<WorkspaceStore, String> {
        match self.vault().read_config::<WorkspaceStore>(WORKSPACES_FILE) {
            Ok(Some(store)) => Ok(store),
            Ok(None) => Ok(WorkspaceStore::default()),
            Err(err) => Err(format!("{WORKSPACES_FILE} could not be read: {err}")),
        }
    }

    /// Records `layout` under `name` and makes it the current one,
    /// replacing what the name had. The file is written first: the vault
    /// is the source of truth, so a change that cannot reach disk does not
    /// count.
    pub fn save_workspace(&self, name: &str, layout: &Value) -> Result<(), String> {
        let Some(entry) = layout.as_object() else {
            return Err("the layout is not an object".to_string());
        };
        let mut store = self.workspaces()?;
        store.workspaces.insert(
            name.to_string(),
            SavedWorkspace {
                layout: entry.clone(),
            },
        );
        store.current = Some(name.to_string());
        self.write_workspaces(&store)
    }

    /// Takes a saved layout out, leaving the rest. The file is written
    /// whole, so a name nothing mentions any more leaves the file too.
    pub fn delete_workspace(&self, name: &str) -> Result<(), String> {
        let mut store = self.workspaces()?;
        if store.workspaces.shift_remove(name).is_none() {
            return Err(format!("No saved workspace layout named {name}"));
        }
        if store.current.as_deref() == Some(name) {
            store.current = None;
        }
        self.write_workspaces(&store)
    }

    fn write_workspaces(&self, store: &WorkspaceStore) -> Result<(), String> {
        self.vault()
            .replace_config(WORKSPACES_FILE, store)
            .map_err(|err| err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A vault to read and write `workspaces.json` in, with one note for a
    /// saved layout to name.
    fn vault() -> (tempfile::TempDir, VaultState) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".obsidian")).unwrap();
        std::fs::write(root.join("A.md"), "# A\n").unwrap();
        let state = VaultState::load(&root.to_path_buf()).unwrap();
        (dir, state)
    }

    /// The layout a session with two tabs in a side-by-side split would
    /// have written.
    fn layout() -> Value {
        json!({
            "main": {
                "type": "split",
                "id": "aaaaaaaa",
                "direction": "vertical",
                "children": [
                    {"type": "tabs", "id": "bbbbbbbb", "children": [
                        {"id": "cccccccc", "type": "leaf", "state": {
                            "type": "markdown", "state": {"file": "A.md", "mode": "source"}}}
                    ]},
                    {"type": "tabs", "id": "dddddddd", "currentTab": 1, "children": [
                        {"id": "eeeeeeee", "type": "leaf", "state": {"type": "empty"}},
                        {"id": "ffffffff", "type": "leaf", "state": {
                            "type": "markdown", "state": {"file": "A.md", "mode": "preview"}}}
                    ]}
                ]
            },
            "active": "cccccccc",
            "lastOpenFiles": ["A.md"],
        })
    }

    /// Every vault that predates the feature has no `workspaces.json`,
    /// and one that does reads as nothing saved rather than failing.
    #[test]
    fn a_vault_without_the_file_has_nothing_saved() {
        let (_dir, state) = vault();
        let store = state.workspaces().unwrap();
        assert!(store.workspaces.is_empty());
        assert!(store.current.is_none());
        assert!(store.names().is_empty());
        assert!(store.layout("Focus").is_none());
    }

    /// Save, then read back: the layout recorded is the layout handed
    /// back, and the file on disk is what the next session reads.
    #[test]
    fn saving_a_layout_and_reading_it_back_returns_the_same_layout() {
        let (dir, state) = vault();
        state.save_workspace("Focus", &layout()).unwrap();
        let store = state.workspaces().unwrap();
        assert_eq!(store.names(), vec!["Focus"]);
        assert_eq!(store.layout("Focus"), Some(layout()));
        assert_eq!(store.current.as_deref(), Some("Focus"));
        let text = std::fs::read_to_string(dir.path().join(".obsidian/workspaces.json")).unwrap();
        let on_disk: WorkspaceStore = serde_json::from_str(&text).unwrap();
        assert_eq!(on_disk.layout("Focus"), Some(layout()));
    }

    /// Saving a second layout, then saving the first again: the second
    /// write replaces the layout the name holds and keeps its place in
    /// the file rather than moving it to the end.
    #[test]
    fn saving_twice_replaces_the_layout_and_keeps_its_place() {
        let (_dir, state) = vault();
        state.save_workspace("Focus", &layout()).unwrap();
        state
            .save_workspace("Reading", &json!({"main": null}))
            .unwrap();
        let replaced = json!({"main": {"type": "tabs"}, "lastOpenFiles": ["A.md", "B.md"]});
        state.save_workspace("Focus", &replaced).unwrap();
        let store = state.workspaces().unwrap();
        assert_eq!(store.names(), vec!["Focus", "Reading"]);
        assert_eq!(store.layout("Focus"), Some(replaced));
    }

    /// A save that cannot reach the disk does not count, and the store in
    /// hand is unchanged.
    #[test]
    fn a_layout_that_is_not_an_object_is_refused() {
        let (_dir, state) = vault();
        assert!(state.save_workspace("Focus", &json!([1, 2])).is_err());
        assert!(state.workspaces().unwrap().names().is_empty());
    }

    /// Deleting a layout nothing else was using leaves the mark that
    /// still names a saved one; deleting the marked one clears it.
    #[test]
    fn deleting_a_layout_leaves_the_others() {
        let (_dir, state) = vault();
        state.save_workspace("Focus", &layout()).unwrap();
        state
            .save_workspace("Reading", &json!({"main": null}))
            .unwrap();
        state.delete_workspace("Focus").unwrap();
        let store = state.workspaces().unwrap();
        assert_eq!(store.names(), vec!["Reading"]);
        assert_eq!(store.current.as_deref(), Some("Reading"));
        // * The one the mark names: taking it out clears the mark, so the
        //   file stops pointing at a layout that is gone.
        state.delete_workspace("Reading").unwrap();
        let store = state.workspaces().unwrap();
        assert!(store.names().is_empty());
        assert_eq!(store.current, None);
        // * Deleting what is not there says so rather than writing a file
        //   that drops everything.
        assert!(state.delete_workspace("Focus").is_err());
        assert!(state.workspaces().unwrap().names().is_empty());
    }

    /// A `workspaces.json` Obsidian wrote: the entry is the layout
    /// document itself, keys Silica does not model come back with it, and
    /// the whole file round-trips unchanged.
    #[test]
    fn obsidians_workspaces_file_parses_with_its_own_keys() {
        let text = r#"{"workspaces":{"Personal":{"main":{"type":"split"},"leftCollapsed":false}},"current":"Personal"}"#;
        let store: WorkspaceStore = serde_json::from_str(text).unwrap();
        assert_eq!(store.names(), vec!["Personal"]);
        assert_eq!(store.current.as_deref(), Some("Personal"));
        let personal = store.layout("Personal").unwrap();
        assert_eq!(personal["leftCollapsed"], json!(false));
        assert_eq!(personal["main"]["type"], json!("split"));
        let back: Value = serde_json::to_value(&store).unwrap();
        assert_eq!(back, serde_json::from_str::<Value>(text).unwrap());
    }

    /// An empty or `null` map, and an empty file, read as nothing saved.
    #[test]
    fn an_empty_or_null_map_reads_as_no_layouts() {
        assert!(serde_json::from_str::<WorkspaceStore>("{}")
            .unwrap()
            .names()
            .is_empty());
        assert!(
            serde_json::from_str::<WorkspaceStore>(r#"{"workspaces":null}"#)
                .unwrap()
                .names()
                .is_empty()
        );
        assert!(
            serde_json::from_str::<WorkspaceStore>(r#"{"workspaces":{}}"#)
                .unwrap()
                .names()
                .is_empty()
        );
    }

    /// A key a newer build wrote at the top level survives a read and the
    /// next write.
    #[test]
    fn an_unknown_top_level_key_is_kept() {
        let store: WorkspaceStore =
            serde_json::from_str(r#"{"workspaces":{},"futureKey":[1,2]}"#).unwrap();
        assert_eq!(store.extra["futureKey"], json!([1, 2]));
        let back: Value = serde_json::to_value(&store).unwrap();
        assert_eq!(back["futureKey"], json!([1, 2]));
    }

    #[test]
    fn names_are_trimmed_and_unusable_ones_are_refused() {
        assert_eq!(normalized_name("  Focus  ").as_deref(), Some("Focus"));
        assert_eq!(normalized_name("Session 2").as_deref(), Some("Session 2"));
        assert!(normalized_name("").is_none());
        assert!(normalized_name("   ").is_none());
        assert!(normalized_name("../vault").is_none());
        assert!(normalized_name("a\\b").is_none());
        assert!(normalized_name("bad\nname").is_none());
    }
}
