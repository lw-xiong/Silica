//! Obsidian's template placeholders and the moment.js date formats they
//! take, free of gpui and free of a clock.
//!
//! The Templates core plugin fills `{{date}}`, `{{time}}` and `{{title}}`
//! in a note it inserts, and Daily notes names a file with the same date
//! formats. Both were observed from the running app rather than guessed —
//! what expands, what stays literal, and what every format token renders —
//! and the tests here are that observation: see
//! [[Specification#Observed templates behavior, Obsidian 1.14.4]].
//!
//! The instant is passed in, never read, so every test is a fixed date and
//! the crate needs no dependency for one.

pub mod expand;
pub mod moment;
pub mod stamp;

pub use expand::{expand, Context, DEFAULT_DATE_FORMAT, DEFAULT_TIME_FORMAT};
pub use moment::{format, unsupported_tokens};
pub use stamp::Stamp;
