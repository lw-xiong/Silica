//! Editor autocompletion: what the text before the cursor asks for, what
//! the vault offers for it, and the text accepting one writes.
//!
//! Obsidian's suggesters are four `EditorSuggest` instances over the whole
//! line before the cursor; what each triggers on, offers and writes was
//! observed from the running app and is recorded in
//! [[Specification#Observed autocompletion behavior, Obsidian 1.14.4]].
//! This crate is that behavior without a window: the GPUI popup in
//! `silica::suggest` renders [`Suggestion`]s and sends the key that
//! accepted one back here, so the bytes a completion writes into the vault
//! are decided where a test can drive them.
//!
//! One line of text, one query, three answers:
//!
//! ```
//! # use silica_suggest::{trigger, Kind};
//! let t = trigger("see [[Alpha#first", 17).expect("a link query");
//! assert_eq!(t.kind, Kind::Link);
//! assert_eq!(t.query, "Alpha#first");
//! ```

pub mod blocks;
mod candidates;
mod insert;
mod trigger;

pub use candidates::{suggestions, BlockId, Item, Suggestion, Vault, LIMIT};
pub use insert::{insert, Accept, Insertion, Options};
pub use trigger::{trigger, Kind, Triggered};
