//! Graph view: notes as nodes, resolved links as edges.

pub mod image;
pub mod layout;
pub mod local;
mod view;

pub use local::LocalGraph;
pub(crate) use view::full_document;
pub use view::{bind_keys, GraphEvent, GraphView};
