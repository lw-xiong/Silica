//! Graph view: notes as nodes, resolved links as edges.

pub mod layout;
mod view;

pub(crate) use view::full_document;
pub use view::{bind_keys, GraphEvent, GraphView};
