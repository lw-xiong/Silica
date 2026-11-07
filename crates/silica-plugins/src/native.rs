//! Native plugins: Rust code compiled into Silica.
//!
//! A `silica.json` marker flags a plugin folder as native tier
//! (classification in [`crate::tier::classify`]). The plugin's code runs
//! in-process: no QuickJS, no sandbox, no bridge. Third-party native plugins
//! register themselves with [`register_native`] at startup; core plugins do
//! the same.

use serde_json::Value;
use std::path::Path;

/// A command a native plugin registered, exposed to the palette and the
/// control socket the same way a compatibility plugin's commands are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeCommand {
    pub id: String,
    pub name: String,
}

type ReadFile<'a> = Box<dyn Fn(&str) -> Result<String, String> + 'a>;
type WriteFile<'a> = Box<dyn Fn(&str, &str) -> Result<(), String> + 'a>;
type ListDir<'a> = Box<dyn Fn(&str) -> Result<Vec<String>, String> + 'a>;
type FileExists<'a> = Box<dyn Fn(&str) -> Result<bool, String> + 'a>;
type OpenNote<'a> = Box<dyn Fn() -> Option<String> + 'a>;
type StrToUnit<'a> = Box<dyn Fn(&str) -> Result<(), String> + 'a>;
/// Show a line to the user. A command that computes an answer has nowhere
/// else to put it: `run_command` returns only success or failure.
type NoticeSink<'a> = Box<dyn Fn(&str) + 'a>;

/// Context handed to a plugin when one of its commands runs. The app fills
/// in the closures; the plugin only calls them.
pub struct PluginContext<'a> {
    /// Vault root directory.
    pub vault_root: &'a Path,
    /// Read a vault-relative file's contents.
    pub read_file: ReadFile<'a>,
    /// Write (overwrite) a vault-relative file's contents.
    pub write_file: WriteFile<'a>,
    /// Append text to a vault-relative file.
    pub append_file: WriteFile<'a>,
    /// Create a new vault-relative file (fails if it exists).
    pub create_file: WriteFile<'a>,
    /// Create a folder (parents too).
    pub create_folder: StrToUnit<'a>,
    /// Delete a file (moves to trash).
    pub delete_file: StrToUnit<'a>,
    /// Rename a file or folder.
    pub rename_file: WriteFile<'a>,
    /// List entries in a vault-relative directory.
    pub list_dir: ListDir<'a>,
    /// Check if a file exists.
    pub file_exists: FileExists<'a>,
    /// Get metadata cache for a note (JSON).
    pub metadata_cache: ReadFile<'a>,
    /// Open a note in the editor (by vault-relative path).
    pub open_note: StrToUnit<'a>,
    /// Get the currently open note's vault-relative path, if any.
    pub current_note: OpenNote<'a>,
    /// Show a line to the user, the way Obsidian's `Notice` does.
    ///
    /// A command that gathers something — backlinks, outgoing links, a word
    /// count — has no return value to put it in: `run_command` reports only
    /// success or failure. Without this the work was done and the result was
    /// dropped on the floor.
    pub notify: NoticeSink<'a>,
}

/// The one message a `run_command` answers an unknown id with.
///
/// Every plugin — the native core plugins, the vault doctor, and the
/// compatibility host — says the same sentence, so it lives here instead
/// of being spelled out at each call site where the copies could drift.
pub fn unknown_command(id: &str) -> String {
    format!("'{id}' is not a command this plugin registered")
}

/// A plugin Silica runs in-process: no bridge, no sandbox, no QuickJS.
pub trait Plugin {
    /// Stable id the plugin is known by, e.g. `silica:backlinks`.
    fn id(&self) -> &'static str;

    /// Human name shown in Settings and the palette.
    fn name(&self) -> &'static str;

    /// Commands this plugin registers.
    fn commands(&self) -> Vec<NativeCommand> {
        let _ = self;
        Vec::new()
    }

    /// Runs a command by id. Returns an error for an unknown id.
    fn run_command(
        &self,
        id: &str,
        value: Option<&Value>,
        ctx: &PluginContext<'_>,
    ) -> Result<(), String> {
        let _ = (id, value, ctx);
        Err(unknown_command(id))
    }
}
