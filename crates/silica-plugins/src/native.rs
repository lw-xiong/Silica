//! Native plugins: Rust code compiled into Silica.
//!
//! A `silica.json` marker flags a plugin folder as native tier
//! (classification in [`crate::tier::classify`]). The plugin's code runs
//! in-process: no QuickJS, no sandbox, no bridge. Third-party native plugins
//! register themselves with [`register_native`] at startup; core plugins do
//! the same.

use serde_json::Value;

/// A command a native plugin registered, exposed to the palette and the
/// control socket the same way a compatibility plugin's commands are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeCommand {
    pub id: String,
    pub name: String,
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
    fn run_command(&self, id: &str, _value: Option<&Value>) -> Result<(), String> {
        let _ = (id, _value);
        Err(format!("'{id}' is not a command this plugin registered"))
    }
}
