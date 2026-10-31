//! The canvas view: draws a `.canvas` file and edits it.
//!
//! `view` owns the element — viewport, painting of edges and card frames,
//! selection, dragging, resizing, drawing edges, saving through
//! `silica_formats::canvas` — over the model in the `silica-canvas` crate.
//! `cards` builds what is drawn inside a card: a text card's Markdown, a file
//! card's note or image, a link card, a group's label.

pub mod cards;
pub mod menu;
pub mod view;

pub use view::{CanvasEvent, CanvasView, FilePick};
