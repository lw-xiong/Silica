//! The plugin host: what a vault's Obsidian plugins are, which of them a
//! native app can run, and the machinery that runs them.
//!
//! Silica opens a vault unchanged and draws it natively — no Chromium, no
//! DOM, no CodeMirror, no Node. Obsidian's plugin API assumes all four, so
//! "run Obsidian plugins" resolves to a much larger set than people expect
//! and a smaller one than "all of them": a plugin that touches only the vault
//! and its index runs here, and one that draws into the DOM cannot be made to
//! without rebuilding Obsidian. [`tier`] is that line; [`registry`] finds and
//! classifies what a vault has.
//!
//! The crate is separate from the app so the JavaScript engine the
//! compatibility host needs is linked into nothing else.
//!
//! See `docs/silica/Plugins.md` for the design and the tiers.

pub mod host;
pub mod live;
pub mod native;
pub mod native_plugins;
pub mod registry;
pub mod tier;

pub use host::{
    CodeBlockRegistration, CommandRegistration, PluginHost, PostProcessorRegistration, Registrations,
    RibbonRegistration, SettingOption, SettingRow, TabRegistration, CALL_DEADLINE, LOAD_DEADLINE,
};
pub use native::{NativeCommand, Plugin, PluginContext};
pub use native_plugins::register_all;
pub use registry::{PluginRecord, Registry, SCAN_PREFIX};
pub use tier::{classify, required_node_module, Refusal, Tier};
