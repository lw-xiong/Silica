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

/// Context handed to a plugin when one of its commands runs. The app fills
/// in the closures; the plugin only calls them.
pub struct PluginContext<'a> {
    /// Vault root directory.
    pub vault_root: &'a Path,
    /// Read a vault-relative file's contents.
    pub read_file: Box<dyn Fn(&str) -> Result<String, String> + 'a>,
    /// Write (overwrite) a vault-relative file's contents.
    pub write_file: Box<dyn Fn(&str, &str) -> Result<(), String> + 'a>,
    /// Append text to a vault-relative file.
    pub append_file: Box<dyn Fn(&str, &str) -> Result<(), String> + 'a>,
    /// Create a new vault-relative file (fails if it exists).
    pub create_file: Box<dyn Fn(&str, &str) -> Result<(), String> + 'a>,
    /// Create a folder (parents too).
    pub create_folder: Box<dyn Fn(&str) -> Result<(), String> + 'a>,
    /// Delete a file (moves to trash).
    pub delete_file: Box<dyn Fn(&str) -> Result<(), String> + 'a>,
    /// Rename a file or folder.
    pub rename_file: Box<dyn Fn(&str, &str) -> Result<(), String> + 'a>,
    /// List entries in a vault-relative directory.
    pub list_dir: Box<dyn Fn(&str) -> Result<Vec<String>, String> + 'a>,
    /// Check if a file exists.
    pub file_exists: Box<dyn Fn(&str) -> Result<bool, String> + 'a>,
    /// Get metadata cache for a note (JSON).
    pub metadata_cache: Box<dyn Fn(&str) -> Result<String, String> + 'a>,
    /// Open a note in the editor (by vault-relative path).
    pub open_note: Box<dyn Fn(&str) -> Result<(), String> + 'a>,
    /// Get the currently open note's vault-relative path, if any.
    pub current_note: Box<dyn Fn() -> Option<String> + 'a>,
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
        Err(format!("'{id}' is not a command this plugin registered"))
    }
}
