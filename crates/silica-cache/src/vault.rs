//! Vault-wide metadata: the per-file records plus the derived indexes
//! Obsidian exposes on `MetadataCache` (`resolvedLinks`, `unresolvedLinks`,
//! backlinks, and the tag index), built once from every note and updated one
//! file at a time.

use crate::{CachedMetadata, LinkCache, LinkResolver};
use silica_formats::config::PropertyTypes;
use silica_formats::properties::{reserved_type, PropertyType, Value};
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// One tag as the tag pane shows it: merged case-insensitively across inline
/// and frontmatter occurrences.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TagEntry {
    /// The tag as first written (without `#`), used for display.
    pub name: String,
    /// Total occurrences across the vault.
    pub count: usize,
    /// Files the tag appears in.
    pub files: BTreeSet<String>,
}

/// A link into a file from another file, with the occurrence for context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backlink<'a> {
    pub source: &'a str,
    /// The occurrence. A link written in a property has no `LinkCache` of its
    /// own in Obsidian's cache shape, so that one is rebuilt from the
    /// `FrontmatterLinkCache` and owned by the row.
    pub link: Cow<'a, LinkCache>,
    /// Property path (`related`, `aliases.1`) when the link was written in
    /// frontmatter, which Obsidian shows as the row's context.
    pub key: Option<&'a str>,
}

#[derive(Debug, Default, Clone)]
pub struct MetadataCache {
    resolver: LinkResolver,
    files: BTreeMap<String, CachedMetadata>,
    /// Source path -> target path -> count (`metadataCache.resolvedLinks`).
    resolved: HashMap<String, BTreeMap<String, usize>>,
    /// Source path -> raw link text -> count (`metadataCache.unresolvedLinks`).
    unresolved: HashMap<String, BTreeMap<String, usize>>,
    /// Target path -> source paths.
    backlinks: HashMap<String, BTreeSet<String>>,
    /// Lowercased tag -> entry.
    tags: BTreeMap<String, TagEntry>,
}

impl MetadataCache {
    /// Builds the cache from already indexed notes. `resolver` must know
    /// every file in the vault, attachments included.
    pub fn new<I, P>(resolver: LinkResolver, notes: I) -> MetadataCache
    where
        I: IntoIterator<Item = (P, CachedMetadata)>,
        P: Into<String>,
    {
        let mut cache = MetadataCache {
            resolver,
            ..MetadataCache::default()
        };
        for (path, meta) in notes {
            cache.insert_file(path.into(), meta);
        }
        cache
    }

    pub fn resolver(&self) -> &LinkResolver {
        &self.resolver
    }

    pub fn file(&self, path: &str) -> Option<&CachedMetadata> {
        self.files.get(path)
    }

    pub fn files(&self) -> impl Iterator<Item = (&str, &CachedMetadata)> {
        self.files.iter().map(|(p, m)| (p.as_str(), m))
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Resolves a link body written in `source_path` to a vault path.
    pub fn resolve(&self, linktext: &str, source_path: &str) -> Option<&str> {
        self.resolver.resolve(linktext, source_path)
    }

    /// Makes a non-note file (an attachment) known to link resolution.
    pub fn add_path(&mut self, path: &str) {
        self.resolver.insert(path);
    }

    /// Replaces (or adds) one note's record and re-derives its edges.
    pub fn insert_file(&mut self, path: String, meta: CachedMetadata) {
        self.remove_file(&path);
        self.resolver.insert(&path);
        self.link_edges(&path, &meta);
        for tag in all_tags(&meta) {
            let entry = self.tags.entry(tag.to_lowercase()).or_default();
            if entry.name.is_empty() {
                entry.name = tag;
            }
            entry.count += 1;
            entry.files.insert(path.clone());
        }
        self.files.insert(path, meta);
    }

    fn link_edges(&mut self, path: &str, meta: &CachedMetadata) {
        let mut resolved: BTreeMap<String, usize> = BTreeMap::new();
        let mut unresolved: BTreeMap<String, usize> = BTreeMap::new();
        let links = meta
            .links
            .iter()
            .chain(meta.embeds.iter())
            .map(|l| l.link.as_str())
            .chain(meta.frontmatter_links.iter().map(|l| l.link.as_str()));
        for link in links {
            match self.resolver.resolve(link, path) {
                Some(target) => {
                    let target = target.to_string();
                    self.backlinks
                        .entry(target.clone())
                        .or_default()
                        .insert(path.to_string());
                    *resolved.entry(target).or_default() += 1;
                }
                None => {
                    let target = silica_markdown::parse_linktext(link).path.to_string();
                    *unresolved.entry(target).or_default() += 1;
                }
            }
        }
        if !resolved.is_empty() {
            self.resolved.insert(path.to_string(), resolved);
        }
        if !unresolved.is_empty() {
            self.unresolved.insert(path.to_string(), unresolved);
        }
    }

    fn unlink_edges(&mut self, path: &str) {
        if let Some(targets) = self.resolved.remove(path) {
            for target in targets.keys() {
                if let Some(sources) = self.backlinks.get_mut(target) {
                    sources.remove(path);
                    if sources.is_empty() {
                        self.backlinks.remove(target);
                    }
                }
            }
        }
        self.unresolved.remove(path);
    }

    /// Re-derives every link edge, after files were added or removed and
    /// links elsewhere may resolve differently.
    pub fn reresolve(&mut self) {
        self.resolved.clear();
        self.unresolved.clear();
        self.backlinks.clear();
        let files: Vec<(String, CachedMetadata)> = self
            .files
            .iter()
            .map(|(p, m)| (p.clone(), m.clone()))
            .collect();
        for (path, meta) in &files {
            self.link_edges(path, meta);
        }
    }

    /// Drops one note and every edge it contributed. The file stays known to
    /// the resolver only if `forget_path` is called too.
    pub fn remove_file(&mut self, path: &str) {
        let Some(meta) = self.files.remove(path) else {
            return;
        };
        self.unlink_edges(path);
        for tag in all_tags(&meta) {
            let key = tag.to_lowercase();
            if let Some(entry) = self.tags.get_mut(&key) {
                entry.count -= 1;
                entry.files.remove(path);
                if entry.count == 0 {
                    self.tags.remove(&key);
                }
            }
        }
    }

    /// Removes a path from link resolution (after a delete or rename).
    pub fn forget_path(&mut self, path: &str) {
        self.remove_file(path);
        self.resolver.remove(path);
    }

    /// Resolved outgoing links of `path`: target path -> count.
    pub fn resolved_links(&self, path: &str) -> Option<&BTreeMap<String, usize>> {
        self.resolved.get(path)
    }

    /// Unresolved outgoing links of `path`: link text -> count.
    pub fn unresolved_links(&self, path: &str) -> Option<&BTreeMap<String, usize>> {
        self.unresolved.get(path)
    }

    /// Paths of the notes with at least one resolved link, embed or
    /// frontmatter link to `target`.
    pub fn sources_linking_to(&self, target: &str) -> impl Iterator<Item = &str> {
        self.backlinks
            .get(target)
            .into_iter()
            .flat_map(|s| s.iter().map(String::as_str))
    }

    /// Every link, embed and property link in other notes that resolves to
    /// `path`, grouped by source file in path order.
    pub fn backlinks(&self, path: &str) -> Vec<Backlink<'_>> {
        let Some(sources) = self.backlinks.get(path) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for source in sources {
            let Some(meta) = self.files.get(source) else {
                continue;
            };
            // Properties first, the order they are written in the file.
            for link in &meta.frontmatter_links {
                if self.resolver.resolve(&link.link, source) == Some(path) {
                    out.push(Backlink {
                        source: source.as_str(),
                        link: Cow::Owned(LinkCache {
                            link: link.link.clone(),
                            original: link.original.clone(),
                            display_text: link.display_text.clone(),
                            position: link.position,
                        }),
                        key: Some(link.key.as_str()),
                    });
                }
            }
            for link in meta.links.iter().chain(meta.embeds.iter()) {
                if self.resolver.resolve(&link.link, source) == Some(path) {
                    out.push(Backlink {
                        source: source.as_str(),
                        link: Cow::Borrowed(link),
                        key: None,
                    });
                }
            }
        }
        out
    }

    /// Tags across the vault keyed by lowercased name, in key order.
    pub fn tags(&self) -> &BTreeMap<String, TagEntry> {
        &self.tags
    }
}

/// One row of the All properties pane: a property key as the vault uses it.
#[derive(Debug, Clone, PartialEq)]
pub struct PropertyEntry {
    /// The key as `types.json` spells it, else as the first note (in path
    /// order) wrote it; counts merge casings.
    pub name: String,
    /// Notes whose properties block has the key.
    pub count: usize,
    /// The key's type by Obsidian's precedence (`types.json`, then the
    /// reserved keys, then the first note's value); `None` for a key whose
    /// value is a nested mapping, which Obsidian shows with a question mark
    /// rather than a type.
    pub ty: Option<PropertyType>,
}

/// Every property key in the vault, the way Obsidian's All properties pane
/// lists them (observed 1.14.4): one row per key merged case-insensitively,
/// with the keys `types.json` names listed even when no note uses them
/// (`cssclasses` at 0). Sorted by frequency, high to low, ties by name
/// case-insensitively, which puts digits before letters.
pub fn all_properties(cache: &MetadataCache, types: &PropertyTypes) -> Vec<PropertyEntry> {
    // Lowercased key -> entry, in first-seen order.
    let mut rows: Vec<(String, PropertyEntry)> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut insert = |name: &str, ty: Option<PropertyType>, count: usize| {
        let key = name.to_lowercase();
        match index.get(&key) {
            Some(&ix) => rows[ix].1.count += count,
            None => {
                index.insert(key.clone(), rows.len());
                rows.push((
                    key,
                    PropertyEntry {
                        name: name.to_string(),
                        count,
                        ty,
                    },
                ));
            }
        }
    };
    for (key, ty) in &types.types {
        insert(key, PropertyType::parse(ty), 0);
    }
    for (_, meta) in cache.files() {
        let Some(serde_yaml::Value::Mapping(map)) = &meta.frontmatter else {
            continue;
        };
        for (key, value) in map {
            let Some(key) = key.as_str() else {
                continue;
            };
            let ty = reserved_type(key)
                .map(Some)
                .unwrap_or_else(|| inferred_type(value));
            insert(key, ty, 1);
        }
    }
    let mut out: Vec<PropertyEntry> = rows.into_iter().map(|(_, e)| e).collect();
    out.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
    out
}

/// What Obsidian infers for a frontmatter value with no assigned type: the
/// rules of [`Value::inferred_type`], and `None` for a nested mapping.
fn inferred_type(value: &serde_yaml::Value) -> Option<PropertyType> {
    let v = match value {
        serde_yaml::Value::Mapping(_) => return None,
        serde_yaml::Value::Null => Value::Empty,
        serde_yaml::Value::Bool(b) => Value::Checkbox(*b),
        serde_yaml::Value::Number(n) => Value::Number(n.as_f64().unwrap_or(0.0)),
        serde_yaml::Value::String(s) => Value::Text(s.clone()),
        serde_yaml::Value::Sequence(_) => Value::List(Vec::new()),
        serde_yaml::Value::Tagged(t) => return inferred_type(&t.value),
    };
    Some(v.inferred_type())
}

/// Inline and frontmatter tags of one note, without `#`, in document order
/// (frontmatter first), as Obsidian's `getAllTags` returns them.
pub fn all_tags(meta: &CachedMetadata) -> Vec<String> {
    let mut out = frontmatter_tags(meta);
    out.extend(
        meta.tags
            .iter()
            .map(|t| t.tag.trim_start_matches('#').to_string()),
    );
    out
}

/// Tags declared in the `tags` (or legacy `tag`) property: a list, or a
/// string split on commas and whitespace; a leading `#` is dropped.
pub fn frontmatter_tags(meta: &CachedMetadata) -> Vec<String> {
    frontmatter_strings(meta, &["tags", "tag"])
        .into_iter()
        .map(|t| t.trim_start_matches('#').to_string())
        .filter(|t| !t.is_empty())
        .collect()
}

/// Aliases declared in the `aliases` (or legacy `alias`) property.
pub fn frontmatter_aliases(meta: &CachedMetadata) -> Vec<String> {
    frontmatter_strings(meta, &["aliases", "alias"])
}

fn frontmatter_strings(meta: &CachedMetadata, keys: &[&str]) -> Vec<String> {
    let Some(serde_yaml::Value::Mapping(map)) = &meta.frontmatter else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for key in keys {
        match map.get(serde_yaml::Value::String((*key).to_string())) {
            Some(serde_yaml::Value::Sequence(items)) => {
                for item in items {
                    if let Some(s) = scalar_string(item) {
                        out.push(s);
                    }
                }
            }
            Some(value) => {
                if let Some(s) = scalar_string(value) {
                    out.extend(
                        s.split(|c: char| c == ',' || c.is_whitespace())
                            .map(str::trim)
                            .filter(|p| !p.is_empty())
                            .map(str::to_string),
                    );
                }
            }
            None => {}
        }
    }
    out
}

fn scalar_string(value: &serde_yaml::Value) -> Option<String> {
    match value {
        serde_yaml::Value::String(s) => Some(s.trim().to_string()).filter(|s| !s.is_empty()),
        serde_yaml::Value::Number(n) => Some(n.to_string()),
        serde_yaml::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index_markdown;

    fn cache() -> MetadataCache {
        let notes = [
            (
                "Home.md",
                "---\ntags: [Project, inbox]\naliases: Start\n---\n# Home\n[[Plan]] and [[Plan#Goals]] and [[missing]] #project\n![[pic.png]]",
            ),
            ("Projects/Plan.md", "Back to [[Home|home]]. #Inbox/weekly\n- [[Plan]] self"),
            ("Projects/Notes.md", "[[Projects/Plan]] [[Home]] [[Home]]"),
        ];
        let resolver = LinkResolver::new([
            "Home.md",
            "Projects/Plan.md",
            "Projects/Notes.md",
            "Assets/pic.png",
        ]);
        MetadataCache::new(
            resolver,
            notes.iter().map(|(p, src)| (*p, index_markdown(src))),
        )
    }

    #[test]
    fn resolved_and_unresolved_links_count_occurrences() {
        let c = cache();
        let home = c.resolved_links("Home.md").unwrap();
        assert_eq!(home.get("Projects/Plan.md"), Some(&2));
        assert_eq!(home.get("Assets/pic.png"), Some(&1));
        assert_eq!(
            c.unresolved_links("Home.md").unwrap().get("missing"),
            Some(&1)
        );
        let notes = c.resolved_links("Projects/Notes.md").unwrap();
        assert_eq!(notes.get("Home.md"), Some(&2));
        assert!(c.unresolved_links("Projects/Notes.md").is_none());
    }

    #[test]
    fn backlinks_list_each_occurrence_with_its_position() {
        let c = cache();
        let links = c.backlinks("Home.md");
        let sources: Vec<&str> = links.iter().map(|b| b.source).collect();
        assert_eq!(
            sources,
            vec!["Projects/Notes.md", "Projects/Notes.md", "Projects/Plan.md"]
        );
        assert_eq!(links[2].link.display_text.as_deref(), Some("home"));
        assert_eq!(links[2].link.position.start.line, 0);
        let plan = c.backlinks("Projects/Plan.md");
        assert_eq!(
            plan.len(),
            4,
            "two from Home, one from Notes, one self link"
        );
        assert!(c.backlinks("Assets/pic.png").len() == 1);
        assert!(c.backlinks("nope.md").is_empty());
    }

    #[test]
    fn a_link_written_in_a_property_is_a_backlink() {
        let source = "---\ntitle: Source\nrelated: \"[[Target]]\"\n---\nNo link in the body.\n";
        let c = MetadataCache::new(
            LinkResolver::new(["Source.md", "Target.md"]),
            [
                ("Source.md", index_markdown(source)),
                ("Target.md", index_markdown("# Target")),
            ],
        );
        assert_eq!(
            c.resolved_links("Source.md").unwrap().get("Target.md"),
            Some(&1)
        );
        let back = c.backlinks("Target.md");
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].source, "Source.md");
        assert_eq!(back[0].key, Some("related"));
        let at = back[0].link.position;
        assert_eq!(at.start.line, 2);
        assert_eq!(&source[at.start.offset..at.end.offset], "[[Target]]");
    }

    #[test]
    fn property_occurrences_come_before_body_ones() {
        let c = MetadataCache::new(
            LinkResolver::new(["Source.md", "Target.md"]),
            [
                (
                    "Source.md",
                    index_markdown(
                        "---\nrelated:\n  - \"[[Target]]\"\n---\nAlso [[Target|here]].\n",
                    ),
                ),
                ("Target.md", index_markdown("# Target")),
            ],
        );
        let back = c.backlinks("Target.md");
        let keys: Vec<Option<&str>> = back.iter().map(|b| b.key).collect();
        assert_eq!(keys, vec![Some("related.0"), None]);
        assert_eq!(back[1].link.display_text.as_deref(), Some("here"));
    }

    #[test]
    fn all_properties_merges_casings_lists_typed_keys_and_sorts_by_frequency() {
        let mut c = MetadataCache::default();
        c.insert_file(
            "b/Note B.md".into(),
            index_markdown("---\nstatus: draft\nzebra: 1\n---\n"),
        );
        c.insert_file(
            "a/Note A.md".into(),
            index_markdown(
                "---\nStatus: done\nzebra: 2\ndue: 2026-10-22\ndone: true\nshelf:\n  row: 1\ntags: [x]\n2026-10-20: hi\n---\n",
            ),
        );
        let mut types = PropertyTypes::default();
        types.types.insert("tags".into(), "tags".into());
        types.types.insert("aliases".into(), "aliases".into());
        types.types.insert("cssclasses".into(), "multitext".into());
        types.types.insert("zebra".into(), "datetime".into());
        let rows = all_properties(&c, &types);
        let names: Vec<(&str, usize)> = rows.iter().map(|r| (r.name.as_str(), r.count)).collect();
        assert_eq!(
            names,
            vec![
                ("Status", 2),
                ("zebra", 2),
                ("2026-10-20", 1),
                ("done", 1),
                ("due", 1),
                ("shelf", 1),
                ("tags", 1),
                ("aliases", 0),
                ("cssclasses", 0),
            ]
        );
        let ty = |n: &str| rows.iter().find(|r| r.name == n).unwrap().ty;
        assert_eq!(ty("zebra"), Some(PropertyType::DateTime));
        assert_eq!(ty("Status"), Some(PropertyType::Text));
        assert_eq!(ty("due"), Some(PropertyType::Date));
        assert_eq!(ty("done"), Some(PropertyType::Checkbox));
        assert_eq!(ty("shelf"), None);
        assert_eq!(ty("tags"), Some(PropertyType::Tags));
        assert_eq!(ty("cssclasses"), Some(PropertyType::MultiText));
    }

    #[test]
    fn tags_merge_frontmatter_and_inline_case_insensitively() {
        let c = cache();
        let tags = c.tags();
        let project = tags.get("project").unwrap();
        assert_eq!(project.name, "Project");
        assert_eq!(project.count, 2);
        assert_eq!(project.files.len(), 1);
        let inbox = tags.get("inbox").unwrap();
        assert_eq!(inbox.count, 1);
        assert_eq!(tags.get("inbox/weekly").unwrap().name, "Inbox/weekly");
        assert_eq!(tags.len(), 3);
    }

    #[test]
    fn frontmatter_helpers_accept_lists_and_strings() {
        let meta = index_markdown("---\ntags: \"#a, b  c\"\naliases:\n  - One\n  - Two\n---\n");
        assert_eq!(frontmatter_tags(&meta), vec!["a", "b", "c"]);
        assert_eq!(frontmatter_aliases(&meta), vec!["One", "Two"]);
        let none = index_markdown("plain");
        assert!(frontmatter_tags(&none).is_empty());
    }

    #[test]
    fn removing_a_file_drops_its_edges_and_tags() {
        let mut c = cache();
        c.remove_file("Projects/Notes.md");
        let home = c.backlinks("Home.md");
        assert_eq!(home.len(), 1);
        assert!(c.resolved_links("Projects/Notes.md").is_none());
        c.forget_path("Home.md");
        assert!(c.tags().get("project").is_none());
        assert!(c.resolve("Home", "x.md").is_none());
        // Plan's link to Home is now unresolved after re-indexing Plan.
        let plan = index_markdown("Back to [[Home|home]].");
        c.insert_file("Projects/Plan.md".into(), plan);
        assert_eq!(
            c.unresolved_links("Projects/Plan.md").unwrap().get("Home"),
            Some(&1)
        );
        // Home comes back: re-resolving turns the unresolved link into a backlink.
        c.insert_file("Home.md".into(), index_markdown("# Home"));
        c.reresolve();
        assert!(c.unresolved_links("Projects/Plan.md").is_none());
        assert_eq!(c.backlinks("Home.md").len(), 1);
        c.add_path("Assets/new.png");
        assert_eq!(c.resolve("new.png", "Home.md"), Some("Assets/new.png"));
    }
}
