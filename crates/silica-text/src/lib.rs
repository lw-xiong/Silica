//! The editor's text model, free of gpui: one `String` buffer with a line
//! index, the selections over it, the undo history, and the edits the
//! editing commands plan.
//!
//! The source editor is a GPUI view, but the bytes it writes back to the
//! vault are decided here, where they can be tested without a window. That
//! is what the Writer gate needs: the vault round-trips byte-identical, so
//! every path from a keystroke to a file's bytes has to be drivable from a
//! test (`tests/writer_roundtrip.rs`).
//!
//! [`TextBuffer`] holds `\n` line endings; a file's own ending and byte
//! order mark are remembered at load and restored by
//! [`TextBuffer::file_contents`], so a save changes nothing but the edited
//! bytes.

pub mod autopair;
mod buffer;
pub mod find;
pub mod format;
pub mod lists;
mod plan;
pub mod table;
pub mod wrap;

pub use buffer::{
    char_class, next_grapheme_boundary, prev_grapheme_boundary, Cursor, Cursors, EditKind,
    FileText, Sel, TextBuffer,
};
pub use format::BlockInsert;
pub use format::{LinePrefix, LinkSyntax};
pub use plan::{indent_plan, newline_plan, shifted};
