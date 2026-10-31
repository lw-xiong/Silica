//! Reading view: the rendered form of a note.

pub mod model;
pub mod view;

pub use model::{build_with, BuildOptions, EmbedSource, LinkTarget, TaskMarker};
pub use view::{ReadingEvent, ReadingView, TaskToggled};
