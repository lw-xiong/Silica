//! The PDF view: draws a `.pdf` file, and the pieces every PDF drawing
//! shares.
//!
//! A PDF is drawn in three places — open in a leaf ([`view`]), embedded in a
//! note as `![[x.pdf]]` ([`embed`], which Live Preview's block widgets and
//! the reading view both draw through the reading renderer), and as a file
//! card on a canvas — and all three draw the same thing: pages rasterized
//! by `silica-pdf` and shown as images. [`documents`] is the one place a
//! file is parsed and a page rendered: a cache of open documents and of
//! rendered pages, filled on the background executor, that every drawing
//! asks for the page it needs and gets a "not yet" from until the page is
//! there. Nothing in this module reads a file on the UI thread.

pub mod documents;
pub mod embed;
pub mod view;

pub use documents::{documents, page_source, DocState, Documents, Loaded};
