//! The Bases expression language.
//!
//! A `.base` holds statements — filters, formulas, summaries — as strings, and
//! this crate turns one into the tree Obsidian parses it into and evaluates
//! that tree against a row. Everything here is observed rather than inferred:
//! see
//! [[Specification#Observed Bases expression language, Obsidian 1.14.5 (installer build 1.13.7)]],
//! whose precedence table was read off real parse trees rather than deduced
//! from values, and whose function table was read two ways that agreed.
//!
//! This crate has **no printer**, deliberately. Obsidian keeps a statement's
//! original text beside its tree and writes the text back, so `silica-formats`
//! holds statements verbatim and never needs a tree to write one. Nothing in
//! the crate graph lets it: `silica-formats` does not depend on this crate and
//! must not, or a writer could route a statement through an AST and lose the
//! bytes.
//!
//! Evaluation reads no vault. The host implements [`Context`] and fills in a
//! [`FileData`] per row, so the crate stays free of `silica-vault` and of
//! gpui.

pub mod cell;
pub mod context;
pub mod eval;
pub mod expr;
pub mod funcs;
pub mod layout;
pub mod table;
pub mod value;

pub use cell::{Cell, CellKind};
pub use context::{Context, SimpleContext};
pub use eval::{eval, eval_str, matches_filter, EvalError, EvalResult, Evaluator, Scope};
pub use expr::{parse, parse_node, BinaryOp, Expr, Literal, ParseError};
pub use layout::{
    CardsLayout, ImageFit, KanbanLayout, Layout, ListLayout, Markers, RowHeight, TableLayout,
};
pub use table::{Column, ColumnSource, Env, Group, Row, RowContext, Table, VIEW_KINDS};
pub use value::{
    Cmp, DateValue, DurationValue, FileData, LinkValue, RegexpValue, RenderKind, RenderedValue,
    Type, Value,
};
