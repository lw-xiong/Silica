//! Obsidian's search query language, implemented once for the Search view,
//! graph filters, bookmarked searches and `query` code blocks.
//!
//! [`parse`] turns a query string into a [`Query`] tree whose [`Query::explain`]
//! output uses the labels Obsidian shows under "Explain search terms", which
//! is the acceptance test for the grammar. [`Search`] compiles a tree (regexes
//! included) and matches it against a [`SearchFile`], returning the byte
//! ranges to highlight. [`fuzzy_match`] and [`simple_search`] are the
//! `prepareFuzzySearch` and `prepareSimpleSearch` counterparts used by the
//! quick switcher and pickers. [`match_lines`] windows a note's hits into
//! the context lines a result row shows. [`natural_cmp`] orders names the
//! way Obsidian's numeric, locale-aware collation does. [`Relevance`]
//! ranks a result list the other way around: the note the query matched
//! most, earliest and in its title comes first.

pub mod collate;
pub mod fuzzy;
pub mod matcher;
pub mod normalize;
pub mod query;
pub mod relevance;
pub mod snippet;

pub use collate::natural_cmp;
pub use fuzzy::{fuzzy_match, simple_search, FuzzyMatch};
pub use matcher::{MatchSet, Search, SearchError, SearchFile, TaskLine};
pub use query::{parse, Compare, Field, Query, index_terms};
pub use relevance::{sort, sort_by_relevance, Ranked, Relevance, ResultSort};
pub use snippet::{context_window, match_lines, MatchLine};
