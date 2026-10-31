//! What a statement is evaluated against.
//!
//! A row's context is the note the row stands for, the base file the view
//! lives in, and the base's own formulas. Nothing here reads a vault: the host
//! fills a [`FileData`] in and answers lookups, so the crate stays free of
//! `silica-vault` and of gpui.

use std::sync::Arc;

use chrono::NaiveDateTime;
use indexmap::IndexMap;

use crate::value::{FileData, Value};

/// The host's side of evaluation.
pub trait Context {
    /// The row's file.
    fn file(&self) -> Option<Arc<FileData>>;

    /// The base file. **`this` is the base file, not the row** — observed, and
    /// the one thing about the namespaces that is easy to get backwards.
    fn base_file(&self) -> Option<Arc<FileData>>;

    /// A formula of the base, by name, as its statement source. Names are
    /// case-insensitive.
    fn formula(&self, name: &str) -> Option<String>;

    /// A file for a path or a link target, for `file()` and `asFile()`.
    fn lookup_file(&self, path: &str) -> Option<Arc<FileData>>;

    /// `now()`. Injectable so a test is not a clock race.
    fn now(&self) -> NaiveDateTime;

    /// `random()`.
    fn random(&self) -> f64 {
        0.0
    }

    /// The row's frontmatter, which a bare ident resolves against and which
    /// `note` is an Object of. Defaults to the row file's own properties.
    fn properties(&self) -> IndexMap<String, Value> {
        self.file()
            .map(|f| f.properties.clone())
            .unwrap_or_default()
    }

    /// One frontmatter property, case-insensitively.
    fn property(&self, name: &str) -> Option<Value> {
        let props = self.properties();
        lookup_ci(&props, name)
    }
}

/// A case-insensitive lookup in a property map. Every *name* in the language is
/// case-insensitive; values are not.
pub fn lookup_ci(map: &IndexMap<String, Value>, name: &str) -> Option<Value> {
    if let Some(v) = map.get(name) {
        return Some(v.clone());
    }
    map.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.clone())
}

/// A context built from values in memory: what a test uses, and what the view
/// can fill a row with.
#[derive(Debug, Clone, Default)]
pub struct SimpleContext {
    pub file: Option<Arc<FileData>>,
    pub base_file: Option<Arc<FileData>>,
    pub formulas: IndexMap<String, String>,
    pub files: Vec<Arc<FileData>>,
    pub now: Option<NaiveDateTime>,
    pub random: f64,
}

impl SimpleContext {
    pub fn new() -> SimpleContext {
        SimpleContext::default()
    }

    pub fn with_file(mut self, file: FileData) -> SimpleContext {
        self.file = Some(Arc::new(file));
        self
    }

    pub fn with_now(mut self, now: NaiveDateTime) -> SimpleContext {
        self.now = Some(now);
        self
    }

    pub fn with_formula(mut self, name: &str, statement: &str) -> SimpleContext {
        self.formulas
            .insert(name.to_string(), statement.to_string());
        self
    }

    pub fn with_vault_file(mut self, file: FileData) -> SimpleContext {
        self.files.push(Arc::new(file));
        self
    }
}

impl Context for SimpleContext {
    fn file(&self) -> Option<Arc<FileData>> {
        self.file.clone()
    }

    fn base_file(&self) -> Option<Arc<FileData>> {
        self.base_file.clone()
    }

    fn formula(&self, name: &str) -> Option<String> {
        self.formulas
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.clone())
    }

    fn lookup_file(&self, path: &str) -> Option<Arc<FileData>> {
        let want = path.trim();
        self.files
            .iter()
            .find(|f| {
                f.path.eq_ignore_ascii_case(want)
                    || f.name.eq_ignore_ascii_case(want)
                    || f.basename.eq_ignore_ascii_case(want)
            })
            .cloned()
            .or_else(|| {
                self.file
                    .iter()
                    .find(|f| {
                        f.path.eq_ignore_ascii_case(want) || f.name.eq_ignore_ascii_case(want)
                    })
                    .cloned()
            })
    }

    fn now(&self) -> NaiveDateTime {
        self.now
            .unwrap_or_else(|| chrono::Local::now().naive_local())
    }

    fn random(&self) -> f64 {
        self.random
    }
}
