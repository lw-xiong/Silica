//! What the vault offers for a query.
//!
//! The link suggester's list is Obsidian's `getLinkSuggestions()`: every
//! file and every alias, and — unlike the quick switcher — no unresolved
//! link targets. An empty query is ordered by modification time, newest
//! first; a query is fuzzy matched. See
//! [[Specification#Observed autocompletion behavior, Obsidian 1.14.4]].

use crate::blocks::Placement;
use crate::trigger::{Kind, Triggered};
use silica_cache::{frontmatter_aliases, MetadataCache};
use silica_formats::FileKind;
use silica_markdown::{parse_linktext, Subpath};
use silica_search::fuzzy_match;
use silica_vault::VaultFile;
use std::ops::Range;

/// Obsidian's limit on every suggester.
pub const LIMIT: usize = 100;

/// What the vault looks like to the suggester: the file list, the metadata
/// cache and the note the link is being written in.
pub struct Vault<'a> {
    pub files: &'a [VaultFile],
    pub cache: &'a MetadataCache,
    /// The vault path of the note being edited.
    pub source: &'a str,
    /// `showUnsupportedFiles`: whether files Silica has no view for are offered.
    pub show_unsupported: bool,
    /// A note's text, for the labels a block row reads. Only a `#^` query
    /// asks for it, so the common `[[` keystroke costs nothing. The same
    /// shape as the `texts` the rename planner takes.
    pub text: &'a dyn Fn(&str) -> Option<String>,
}

/// What accepting a row links to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    /// A file in the vault, by vault path.
    File { path: String },
    /// An alias of a file; the alias becomes the link's display text.
    Alias { path: String, alias: String },
    /// A heading in a file. `path` is empty for a heading in the note being
    /// edited (`[[#`).
    Heading {
        path: String,
        heading: String,
        level: u8,
    },
    /// A block of a file. `path` is empty for a block of the note being
    /// edited (`[[#^`).
    Block { path: String, id: BlockId },
    /// A tag, without its `#`.
    Tag { tag: String },
    /// The query as typed: a note that does not exist yet, or display text
    /// after a `|` that is not one of the target's aliases.
    Literal,
}

/// The id a block row links by: the one it carries, or the one that has to
/// be written into the target note before the link can be made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockId {
    Existing(String),
    /// Where the block ends in the target's text, and how an id goes in.
    New {
        end: usize,
        placement: Placement,
    },
}

/// One row of the popup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub item: Item,
    /// What the row reads.
    pub label: String,
    /// The muted text at the end of the row.
    pub detail: Option<String>,
    /// Byte ranges of `label` that the query matched.
    pub ranges: Vec<Range<usize>>,
}

impl Suggestion {
    fn new(item: Item, label: String, detail: Option<String>) -> Suggestion {
        Suggestion {
            item,
            label,
            detail,
            ranges: Vec::new(),
        }
    }
}

/// A markdown file's path without `.md`, which is what the link suggester
/// matches and shows; other files keep their extension.
fn display_path(path: &str) -> &str {
    if FileKind::from_path(path).is_markdown() {
        path.strip_suffix(".md")
            .or_else(|| path.strip_suffix(".MD"))
            .unwrap_or(path)
    } else {
        path
    }
}

fn folder_detail(path: &str) -> Option<String> {
    path.rsplit_once('/').map(|(dir, _)| dir.to_string())
}

/// The rows to offer for `t`, at most [`LIMIT`] of them.
pub fn suggestions(t: &Triggered<'_>, v: &Vault<'_>) -> Vec<Suggestion> {
    let mut out = match t.kind {
        Kind::Link | Kind::Embed => link_suggestions(t.query, v),
        Kind::Tag => tag_suggestions(t.query, v),
    };
    out.truncate(LIMIT);
    out
}

/// The link suggester splits the one query itself: `Alpha#First` asks for a
/// heading, `Alpha|disp` for display text, `Alpha#^id` for a block.
fn link_suggestions(query: &str, v: &Vault<'_>) -> Vec<Suggestion> {
    let parsed = parse_linktext(query);
    if let Some(alias) = parsed.alias {
        return alias_suggestions(parsed.path, alias, v);
    }
    match parsed.subpath {
        Some(Subpath::Block(query)) => block_suggestions(parsed.path, query, v),
        Some(Subpath::Headings(headings)) => {
            heading_suggestions(parsed.path, headings.last().copied().unwrap_or(""), v)
        }
        None => file_suggestions(query.trim(), v),
    }
}

/// Every file and alias, by modification time while the query is empty and
/// fuzzy matched once it is not.
fn file_suggestions(query: &str, v: &Vault<'_>) -> Vec<Suggestion> {
    let mut scored: Vec<(f64, u64, Suggestion)> = Vec::new();
    for file in v.files {
        if file.kind == FileKind::Other && !v.show_unsupported {
            continue;
        }
        let shown = display_path(&file.path);
        let push = |scored: &mut Vec<(f64, u64, Suggestion)>, item: Item, label: &str| {
            let detail = match &item {
                Item::Alias { path, .. } => Some(format!("alias of {}", display_path(path))),
                _ => folder_detail(&file.path),
            };
            if query.is_empty() {
                scored.push((
                    0.,
                    file.mtime,
                    Suggestion::new(item, label.to_string(), detail),
                ));
                return;
            }
            if let Some(m) = fuzzy_match(query, label) {
                let mut s = Suggestion::new(item, label.to_string(), detail);
                s.ranges = m.ranges;
                scored.push((m.score, file.mtime, s));
            }
        };
        push(
            &mut scored,
            Item::File {
                path: file.path.clone(),
            },
            shown,
        );
        if let Some(meta) = v.cache.file(&file.path) {
            for alias in frontmatter_aliases(meta) {
                push(
                    &mut scored,
                    Item::Alias {
                        path: file.path.clone(),
                        alias: alias.clone(),
                    },
                    &alias,
                );
            }
        }
    }
    // An alias scores just below its file, and the newest file wins a tie.
    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.1.cmp(&a.1))
            .then_with(|| a.2.label.cmp(&b.2.label))
    });
    let mut out: Vec<Suggestion> = scored.into_iter().map(|(_, _, s)| s).collect();
    // A query nothing matches offers to link it as typed; no file is created
    // until the link is followed.
    if out.is_empty() && !query.is_empty() {
        out.push(Suggestion::new(
            Item::Literal,
            query.to_string(),
            Some("Not created yet".to_string()),
        ));
    }
    out
}

/// The headings of the note the query points at, in document order. An
/// empty path means the note being edited (`[[#`).
fn heading_suggestions(path: &str, query: &str, v: &Vault<'_>) -> Vec<Suggestion> {
    let Some(target) = target_note(path, v) else {
        return Vec::new();
    };
    let Some(meta) = v.cache.file(&target) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for h in &meta.headings {
        let item = Item::Heading {
            path: if path.is_empty() {
                String::new()
            } else {
                target.clone()
            },
            heading: h.heading.clone(),
            level: h.level,
        };
        let mut s = Suggestion::new(item, h.heading.clone(), Some("#".repeat(h.level as usize)));
        if !query.is_empty() {
            match fuzzy_match(query, &h.heading) {
                Some(m) => s.ranges = m.ranges,
                None => continue,
            }
        }
        out.push(s);
    }
    out
}

/// The blocks of the note the query points at that already carry an id, in
/// document order.
///
/// Obsidian also offers blocks with no id and writes one into the target
/// note when such a row is accepted. Silica offers only the rows it can
/// link to without touching another file; the rest wait for the commit that
/// writes ids. See
/// [[Specification#Observed block reference behavior, Obsidian 1.14.4]].
fn block_suggestions(path: &str, query: &str, v: &Vault<'_>) -> Vec<Suggestion> {
    let target = match target_note(path, v) {
        Some(t) => t,
        None => return Vec::new(),
    };
    let Some(meta) = v.cache.file(&target) else {
        return Vec::new();
    };
    let text = (v.text)(&target).unwrap_or_default();
    let shown_path = if path.is_empty() {
        String::new()
    } else {
        target.clone()
    };
    // Every block of the note in document order: the sections Obsidian
    // writes an id onto, and a list's items rather than the list itself.
    let sections = meta.sections.iter().filter_map(|s| {
        let placement = Placement::of(s.kind)?;
        Some((s.position, s.id.clone(), placement))
    });
    let items = meta
        .list_items
        .iter()
        .map(|i| (i.position, i.id.clone(), Placement::Inline));
    let mut rows: Vec<(usize, Suggestion)> = Vec::new();
    for (position, carried, placement) in sections.chain(items) {
        let label = block_label(&text, position.start.offset..position.end.offset);
        if label.is_empty() {
            continue;
        }
        let id = match carried {
            Some(id) => BlockId::Existing(id),
            None => BlockId::New {
                end: position.end.offset,
                placement,
            },
        };
        let detail = match &id {
            BlockId::Existing(id) => Some(format!("^{id}")),
            BlockId::New { .. } => None,
        };
        let item = Item::Block {
            path: shown_path.clone(),
            id: id.clone(),
        };
        let mut s = Suggestion::new(item, label.clone(), detail);
        if !query.is_empty() {
            // A caret query is matched as a substring, not fuzzily the way
            // a file query is: Obsidian offers `A paragraph.` for `^ara`
            // and does not offer `Some text with ...`, which a subsequence
            // match would. It matches the block's text or the id it
            // carries, either way and whatever the case.
            let by_id = match &id {
                BlockId::Existing(id) => contains_fold(id, query).is_some(),
                BlockId::New { .. } => false,
            };
            match contains_fold(&label, query) {
                Some(range) => s.ranges = vec![range],
                None if by_id => {}
                None => continue,
            }
        }
        rows.push((position.start.offset, s));
    }
    // A list and its only item can start at the same byte; the item wins,
    // being the row Obsidian offers.
    rows.sort_by_key(|(at, _)| *at);
    rows.dedup_by(|a, b| a.0 == b.0);
    rows.into_iter().map(|(_, s)| s).collect()
}

/// Where `needle` sits in `hay`, ignoring case. ASCII-folded, which is
/// what a block id and the text around one are in practice.
fn contains_fold(hay: &str, needle: &str) -> Option<Range<usize>> {
    let (h, n) = (hay.to_lowercase(), needle.to_lowercase());
    // Lowercasing can change byte lengths, so the hit is mapped back by
    // walking the original's char boundaries.
    let at = h.find(&n)?;
    let prefix_chars = h[..at].chars().count();
    let start = hay
        .char_indices()
        .nth(prefix_chars)
        .map(|(i, _)| i)
        .unwrap_or(hay.len());
    let len_chars = n.chars().count();
    let end = hay[start..]
        .char_indices()
        .nth(len_chars)
        .map(|(i, _)| start + i)
        .unwrap_or(hay.len());
    Some(start..end)
}

/// What a block row reads: the block's own text with the `^id` taken off,
/// since the id is shown beside it rather than in it.
fn block_label(text: &str, span: Range<usize>) -> String {
    let raw = text.get(span).unwrap_or_default().trim();
    let stripped = match raw.rsplit_once('^') {
        Some((before, id))
            if !id.is_empty() && id.chars().all(|c| c.is_alphanumeric() || c == '-') =>
        {
            before.trim_end()
        }
        _ => raw,
    };
    stripped.trim().to_string()
}

/// The note a `#` or `#^` query points at: the one named, or the one being
/// edited when the path is empty.
fn target_note(path: &str, v: &Vault<'_>) -> Option<String> {
    if path.is_empty() {
        Some(v.source.to_string())
    } else {
        v.cache.resolve(path, v.source).map(str::to_string)
    }
}

/// The target's aliases, or the text typed after the `|` as display text.
fn alias_suggestions(path: &str, typed: &str, v: &Vault<'_>) -> Vec<Suggestion> {
    let mut out = Vec::new();
    if let Some(target) = v.cache.resolve(path, v.source) {
        let target = target.to_string();
        if let Some(meta) = v.cache.file(&target) {
            for alias in frontmatter_aliases(meta) {
                let mut s = Suggestion::new(
                    Item::Alias {
                        path: target.clone(),
                        alias: alias.clone(),
                    },
                    alias.clone(),
                    Some("alias".to_string()),
                );
                if !typed.is_empty() {
                    match fuzzy_match(typed, &alias) {
                        Some(m) => s.ranges = m.ranges,
                        None => continue,
                    }
                }
                out.push(s);
            }
        }
    }
    if out.is_empty() && !typed.is_empty() {
        out.push(Suggestion::new(
            Item::Literal,
            typed.to_string(),
            Some("display text".to_string()),
        ));
    }
    out
}

/// Every tag in the vault, parent segments included, alphabetically.
fn tag_suggestions(query: &str, v: &Vault<'_>) -> Vec<Suggestion> {
    let mut names: Vec<&str> = Vec::new();
    for name in v.cache.tags().keys() {
        names.push(name);
    }
    // `project/alpha` also offers `project`; the tag pane counts them the
    // same way.
    let mut all: Vec<String> = Vec::new();
    for name in &names {
        let mut at = 0;
        while let Some(ix) = name[at..].find('/') {
            all.push(name[..at + ix].to_string());
            at += ix + 1;
        }
        all.push((*name).to_string());
    }
    all.sort();
    all.dedup();
    let mut out = Vec::new();
    for tag in all {
        let count = v.cache.tags().get(&tag).map(|t| t.count);
        let mut s = Suggestion::new(
            Item::Tag { tag: tag.clone() },
            tag.clone(),
            count.map(|c| c.to_string()),
        );
        if !query.is_empty() {
            match fuzzy_match(query, &tag) {
                Some(m) => s.ranges = m.ranges,
                None => continue,
            }
        }
        out.push(s);
    }
    if !query.is_empty() {
        out.sort_by(|a, b| {
            a.label
                .len()
                .cmp(&b.label.len())
                .then(a.label.cmp(&b.label))
        });
    }
    out
}
