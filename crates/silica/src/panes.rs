//! The rows of the Tags, All properties and Outline panes, in the shape
//! Obsidian lays them out (probed 2026-10-22, see the three "Observed ...
//! pane behavior" sections of the specification): a tree of tags with
//! inclusive counts, a flat list of property keys with a type icon and a
//! note count, and a structural tree of headings with the one holding the
//! cursor marked. Each pane's sort order, hierarchy toggle, folds and
//! search filter are applied here, and nothing here touches gpui.

use silica_cache::{PropertyEntry, TagEntry};
use silica_formats::properties::PropertyType;
use std::collections::{BTreeMap, HashSet};
use std::ops::Range;

/// The four orders the Tags and All properties sort menus offer, with the
/// values `workspace.json` stores for `sortOrder`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PaneSort {
    Alphabetical,
    AlphabeticalReverse,
    #[default]
    Frequency,
    FrequencyReverse,
}

impl PaneSort {
    /// Menu order: the two name orders, a separator, the two frequency orders.
    pub const ALL: [PaneSort; 4] = [
        PaneSort::Alphabetical,
        PaneSort::AlphabeticalReverse,
        PaneSort::Frequency,
        PaneSort::FrequencyReverse,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            PaneSort::Alphabetical => "alphabetical",
            PaneSort::AlphabeticalReverse => "alphabeticalReverse",
            PaneSort::Frequency => "frequency",
            PaneSort::FrequencyReverse => "frequencyReverse",
        }
    }

    pub fn parse(s: &str) -> Option<PaneSort> {
        PaneSort::ALL.into_iter().find(|o| o.as_str() == s)
    }

    /// The menu label: `noun` is "Tag name" or "Property name".
    pub fn label(self, noun: &str) -> String {
        match self {
            PaneSort::Alphabetical => format!("{noun} (A to Z)"),
            PaneSort::AlphabeticalReverse => format!("{noun} (Z to A)"),
            PaneSort::Frequency => "Frequency (high to low)".to_string(),
            PaneSort::FrequencyReverse => "Frequency (low to high)".to_string(),
        }
    }

    /// Orders two rows by their names (case-insensitively, ties by the
    /// exact spelling) and counts: both frequency orders break ties A to Z.
    fn compare(
        self,
        a_name: &str,
        a_count: usize,
        b_name: &str,
        b_count: usize,
    ) -> std::cmp::Ordering {
        let by_name = || {
            a_name
                .to_lowercase()
                .cmp(&b_name.to_lowercase())
                .then_with(|| a_name.cmp(b_name))
        };
        match self {
            PaneSort::Alphabetical => by_name(),
            PaneSort::AlphabeticalReverse => by_name().reverse(),
            PaneSort::Frequency => b_count.cmp(&a_count).then_with(by_name),
            PaneSort::FrequencyReverse => a_count.cmp(&b_count).then_with(by_name),
        }
    }
}

// ----- tags ----------------------------------------------------------------

/// One tag in the tree: `project` holding `alpha` and `beta`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagNode {
    /// The last segment as first written (`alpha`).
    pub name: String,
    /// The full path as first written (`project/alpha`), what a click
    /// searches for.
    pub path: String,
    /// The full path lowercased: the key folds are remembered under.
    pub key: String,
    /// Occurrences of the tag itself plus every descendant's.
    pub count: usize,
    pub children: Vec<TagNode>,
}

/// A row of the Tags pane as drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagRow {
    pub name: String,
    /// The path up to and including the last `/` (`project/`), empty at
    /// the root; shown only in flat mode.
    pub parent: String,
    pub path: String,
    pub key: String,
    pub count: usize,
    pub depth: usize,
    pub has_children: bool,
    pub collapsed: bool,
}

/// The tag tree from the cache's case-insensitively merged entries, each
/// parent counting its own occurrences plus every descendant's, as
/// `app.metadataCache.getTags()` counts them. A parent nobody wrote on its
/// own (`project` with only `project/alpha` in the vault) still gets a
/// node. A node is spelled the way the first file carrying it or any
/// descendant spells it (`#Project` in Beta.md folds into Alpha.md's
/// `project/alpha`), file names in order standing in for Obsidian's index
/// order. Unsorted: see [`sort_tags`].
pub fn tag_tree(tags: &BTreeMap<String, TagEntry>) -> Vec<TagNode> {
    struct Build {
        name: String,
        key: String,
        /// The file that spelled `name`, for a later entry to beat.
        first: Option<String>,
        count: usize,
        children: Vec<Build>,
    }
    let mut roots: Vec<Build> = Vec::new();
    for (key, entry) in tags {
        let names: Vec<&str> = entry.name.split('/').collect();
        let keys: Vec<&str> = key.split('/').collect();
        if names.len() != keys.len() {
            continue;
        }
        let first = entry.files.iter().next().cloned();
        let mut level = &mut roots;
        for depth in 0..names.len() {
            let node_key = keys[..=depth].join("/");
            let ix = match level.iter().position(|n| n.key == node_key) {
                Some(ix) => ix,
                None => {
                    level.push(Build {
                        name: names[depth].to_string(),
                        key: node_key,
                        first: first.clone(),
                        count: 0,
                        children: Vec::new(),
                    });
                    level.len() - 1
                }
            };
            let node = &mut level[ix];
            let earlier = match (&first, &node.first) {
                (Some(f), Some(n)) => f < n,
                (Some(_), None) => true,
                _ => false,
            };
            if earlier {
                node.name = names[depth].to_string();
                node.first = first.clone();
            }
            if depth + 1 == names.len() {
                node.count += entry.count;
            }
            level = &mut node.children;
        }
    }
    fn finish(node: Build, parent: &str) -> TagNode {
        let path = if parent.is_empty() {
            node.name.clone()
        } else {
            format!("{parent}/{}", node.name)
        };
        let children: Vec<TagNode> = node
            .children
            .into_iter()
            .map(|c| finish(c, &path))
            .collect();
        let count = node.count + children.iter().map(|c| c.count).sum::<usize>();
        TagNode {
            name: node.name,
            path,
            key: node.key,
            count,
            children,
        }
    }
    roots.into_iter().map(|r| finish(r, "")).collect()
}

/// Sorts every level of the tree by `order`: the Z to A order is the exact
/// reverse of A to Z, children included, and the frequency orders break
/// ties by name A to Z at every level.
pub fn sort_tags(nodes: &mut [TagNode], order: PaneSort) {
    nodes.sort_by(|a, b| order.compare(&a.name, a.count, &b.name, b.count));
    for node in nodes {
        sort_tags(&mut node.children, order);
    }
}

/// The rows the pane draws. With `hierarchy` the tree is walked, a
/// collapsed node's children left out; without it every tag is a depth-0
/// row sorted by its full path, the parent part separate. `query` keeps
/// the tags whose full path contains it case-insensitively, and in the
/// tree their ancestors too.
pub fn tag_rows(
    tree: &[TagNode],
    order: PaneSort,
    hierarchy: bool,
    collapsed: &HashSet<String>,
    query: &str,
) -> Vec<TagRow> {
    let query = query.trim().to_lowercase();
    let hit = |node: &TagNode| query.is_empty() || node.key.contains(&query);
    let mut out = Vec::new();
    if hierarchy {
        fn walk(
            nodes: &[TagNode],
            depth: usize,
            collapsed: &HashSet<String>,
            hit: &dyn Fn(&TagNode) -> bool,
            out: &mut Vec<TagRow>,
        ) -> bool {
            let mut any = false;
            for node in nodes {
                let at = out.len();
                out.push(row_of(node, depth, false));
                let is_collapsed = collapsed.contains(&node.key);
                let kept_below = if is_collapsed {
                    // Hidden children still decide whether the parent stays.
                    descendant_hits(node, hit)
                } else {
                    walk(&node.children, depth + 1, collapsed, hit, out)
                };
                if hit(node) || kept_below {
                    out[at].collapsed = is_collapsed;
                    any = true;
                } else {
                    out.truncate(at);
                }
            }
            any
        }
        fn descendant_hits(node: &TagNode, hit: &dyn Fn(&TagNode) -> bool) -> bool {
            node.children
                .iter()
                .any(|c| hit(c) || descendant_hits(c, hit))
        }
        walk(tree, 0, collapsed, &hit, &mut out);
    } else {
        fn flatten<'a>(nodes: &'a [TagNode], out: &mut Vec<&'a TagNode>) {
            for node in nodes {
                out.push(node);
                flatten(&node.children, out);
            }
        }
        let mut all: Vec<&TagNode> = Vec::new();
        flatten(tree, &mut all);
        all.retain(|n| hit(n));
        all.sort_by(|a, b| order.compare(&a.path, a.count, &b.path, b.count));
        out.extend(all.into_iter().map(|n| row_of(n, 0, true)));
    }
    out
}

fn row_of(node: &TagNode, depth: usize, flat: bool) -> TagRow {
    let parent = match node.path.rfind('/') {
        Some(i) => node.path[..=i].to_string(),
        None => String::new(),
    };
    TagRow {
        name: node.name.clone(),
        parent,
        path: node.path.clone(),
        key: node.key.clone(),
        count: node.count,
        depth,
        has_children: !flat && !node.children.is_empty(),
        collapsed: false,
    }
}

/// Every key in the tree, for "Expand all" to fold them all at once.
pub fn tag_keys(tree: &[TagNode]) -> Vec<String> {
    let mut out = Vec::new();
    fn walk(nodes: &[TagNode], out: &mut Vec<String>) {
        for node in nodes {
            out.push(node.key.clone());
            walk(&node.children, out);
        }
    }
    walk(tree, &mut out);
    out
}

// ----- all properties ------------------------------------------------------

/// A row of the All properties pane.
#[derive(Debug, Clone, PartialEq)]
pub struct PropertyRow {
    pub name: String,
    pub count: usize,
    pub ty: Option<PropertyType>,
    /// The Lucide icon in the row's gutter.
    pub icon: &'static str,
}

/// The icon Obsidian draws for a property widget: `text`, `list`,
/// `binary`, `check-square` (Lucide's `square-check`), `calendar`,
/// `clock`, `tags`, `forward`, and `file-question` for a key whose value is
/// a nested mapping (widget "unknown").
pub fn property_icon(ty: Option<PropertyType>) -> &'static str {
    match ty {
        Some(PropertyType::Text) => "text",
        Some(PropertyType::MultiText) => "list",
        Some(PropertyType::Number) => "binary",
        Some(PropertyType::Checkbox) => "square-check",
        Some(PropertyType::Date) => "calendar",
        Some(PropertyType::DateTime) => "clock",
        Some(PropertyType::Tags) => "tags",
        Some(PropertyType::Aliases) => "forward",
        None => "file-question",
    }
}

/// The rows in `order`, keeping those whose key contains `query`
/// case-insensitively.
pub fn property_rows(entries: &[PropertyEntry], order: PaneSort, query: &str) -> Vec<PropertyRow> {
    let query = query.trim().to_lowercase();
    let mut rows: Vec<PropertyRow> = entries
        .iter()
        .filter(|e| query.is_empty() || e.name.to_lowercase().contains(&query))
        .map(|e| PropertyRow {
            name: e.name.clone(),
            count: e.count,
            ty: e.ty,
            icon: property_icon(e.ty),
        })
        .collect();
    rows.sort_by(|a, b| order.compare(&a.name, a.count, &b.name, b.count));
    rows
}

// ----- outline -------------------------------------------------------------

/// One heading of the open note, as the pane needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heading {
    /// The plain text: markup rendered away, `#tags` kept.
    pub text: String,
    pub level: u8,
    /// Zero-based line and byte offset of the heading's line.
    pub line: usize,
    pub offset: usize,
}

/// A row of the Outline pane as drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineRow {
    /// Index into the headings, the key a fold is remembered under.
    pub ix: usize,
    pub depth: usize,
    pub text: String,
    pub level: u8,
    pub line: usize,
    pub offset: usize,
    pub has_children: bool,
    pub collapsed: bool,
    /// The heading whose section holds the cursor.
    pub active: bool,
    /// Byte ranges of `text` the search query matched.
    pub highlights: Vec<Range<usize>>,
}

/// Each heading's depth: a heading is a child of the nearest preceding
/// heading of a smaller level, whatever the numbers, so `####` straight
/// under `##` sits at depth 2 and `###` straight under `#` at depth 1.
pub fn outline_depths(headings: &[Heading]) -> Vec<usize> {
    let mut stack: Vec<u8> = Vec::new();
    headings
        .iter()
        .map(|h| {
            while stack.last().is_some_and(|&l| l >= h.level) {
                stack.pop();
            }
            let depth = stack.len();
            stack.push(h.level);
            depth
        })
        .collect()
}

/// The heading whose section holds byte `offset`: the last one starting at
/// or before it, or none while the offset is above the first heading (the
/// frontmatter, or text before any heading).
pub fn heading_at(headings: &[Heading], offset: usize) -> Option<usize> {
    headings.iter().rposition(|h| h.offset <= offset)
}

/// The rows the pane draws: the tree with a collapsed heading's
/// descendants left out, filtered to the headings whose text contains
/// `query` case-insensitively plus their ancestors, with the match ranges
/// to highlight; `active` is marked only when its row is on the list.
pub fn outline_rows(
    headings: &[Heading],
    collapsed: &HashSet<usize>,
    query: &str,
    active: Option<usize>,
) -> Vec<OutlineRow> {
    let depths = outline_depths(headings);
    let query = query.trim().to_lowercase();
    let n = headings.len();
    // Which headings the filter keeps: a match and its ancestors.
    let mut kept = vec![query.is_empty(); n];
    if !query.is_empty() {
        for ix in 0..n {
            if !find_all(&headings[ix].text, &query).is_empty() {
                kept[ix] = true;
                let mut depth = depths[ix];
                for back in (0..ix).rev() {
                    if depth == 0 {
                        break;
                    }
                    if depths[back] < depth {
                        kept[back] = true;
                        depth = depths[back];
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    let mut hidden_below: Option<usize> = None;
    for (ix, h) in headings.iter().enumerate() {
        let depth = depths[ix];
        if let Some(d) = hidden_below {
            if depth > d {
                continue;
            }
            hidden_below = None;
        }
        if !kept[ix] {
            continue;
        }
        let has_children = depths.get(ix + 1).is_some_and(|&d| d > depth);
        let is_collapsed = has_children && collapsed.contains(&ix);
        if is_collapsed {
            hidden_below = Some(depth);
        }
        out.push(OutlineRow {
            ix,
            depth,
            text: h.text.clone(),
            level: h.level,
            line: h.line,
            offset: h.offset,
            has_children,
            collapsed: is_collapsed,
            active: active == Some(ix),
            highlights: if query.is_empty() {
                Vec::new()
            } else {
                find_all(&h.text, &query)
            },
        });
    }
    out
}

/// Every case-insensitive occurrence of `query` (already lowercased) in
/// `text`, as byte ranges of `text`.
fn find_all(text: &str, query: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    if query.is_empty() {
        return out;
    }
    let mut from = 0;
    while from < text.len() {
        let Some(start) = text[from..]
            .char_indices()
            .map(|(i, _)| from + i)
            .find(|&i| text[i..].to_lowercase().starts_with(query))
        else {
            break;
        };
        // The end is where the lowercased chars have covered the query.
        let mut end = start;
        let mut covered = 0;
        for (i, c) in text[start..].char_indices() {
            if covered >= query.len() {
                break;
            }
            covered += c.to_lowercase().map(char::len_utf8).sum::<usize>();
            end = start + i + c.len_utf8();
        }
        out.push(start..end);
        from = end.max(start + 1);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, count: usize, files: &[&str]) -> (String, TagEntry) {
        (
            name.to_lowercase(),
            TagEntry {
                name: name.to_string(),
                count,
                files: files.iter().map(|f| f.to_string()).collect(),
            },
        )
    }

    /// The probe's vault as the cache holds it: each tag's own occurrences
    /// (the pane adds a parent's descendants), the first-written spelling
    /// and the files.
    fn probe_tags() -> BTreeMap<String, TagEntry> {
        [
            entry("project/alpha", 3, &["Alpha.md", "Props.md"]),
            entry(
                "common",
                9,
                &["Alpha.md", "Beta.md", "Delta.md", "Epsilon.md", "Gamma.md"],
            ),
            entry("project/beta", 1, &["Beta.md"]),
            entry("Project", 1, &["Beta.md"]),
            entry("Lambda", 2, &["Beta.md", "Gamma.md"]),
            entry("project/beta/deep", 1, &["Gamma.md"]),
            entry("topic", 2, &["Gamma.md", "Outline.md"]),
            entry("Kappa", 1, &["Epsilon.md"]),
            entry("meta", 1, &["Props.md"]),
            entry("single", 1, &["Outline.md"]),
            entry("a", 1, &["NoHeadings.md"]),
        ]
        .into_iter()
        .collect()
    }

    fn paths(rows: &[TagRow]) -> Vec<(String, usize, usize)> {
        rows.iter()
            .map(|r| (r.path.clone(), r.count, r.depth))
            .collect()
    }

    #[test]
    fn parents_count_their_descendants_and_keep_the_first_spelling() {
        let mut tree = tag_tree(&probe_tags());
        sort_tags(&mut tree, PaneSort::Frequency);
        let rows = tag_rows(&tree, PaneSort::Frequency, true, &HashSet::new(), "");
        assert_eq!(
            paths(&rows),
            vec![
                ("common".into(), 9, 0),
                ("project".into(), 6, 0),
                ("project/alpha".into(), 3, 1),
                ("project/beta".into(), 2, 1),
                ("project/beta/deep".into(), 1, 2),
                ("Lambda".into(), 2, 0),
                ("topic".into(), 2, 0),
                ("a".into(), 1, 0),
                ("Kappa".into(), 1, 0),
                ("meta".into(), 1, 0),
                ("single".into(), 1, 0),
            ]
        );
        assert!(rows[1].has_children && !rows[2].has_children);
        assert_eq!(rows[2].parent, "project/");
        assert_eq!(rows[2].name, "alpha");
    }

    #[test]
    fn a_parent_nobody_wrote_still_gets_a_node() {
        let tags: BTreeMap<String, TagEntry> = [
            entry("Area/one", 2, &["Zed.md"]),
            entry("area/two", 1, &["Apple.md"]),
        ]
        .into_iter()
        .collect();
        let tree = tag_tree(&tags);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].name, "area", "spelled as the first file spells it");
        assert_eq!(tree[0].children[0].path, "area/one");
        assert_eq!(tree[0].count, 3);
        assert_eq!(tree[0].children.len(), 2);
    }

    #[test]
    fn the_four_orders_match_obsidians() {
        let mut tree = tag_tree(&probe_tags());
        let names = |tree: &[TagNode], order: PaneSort| -> Vec<String> {
            tag_rows(tree, order, true, &HashSet::new(), "")
                .iter()
                .map(|r| r.key.clone())
                .collect()
        };
        sort_tags(&mut tree, PaneSort::Alphabetical);
        assert_eq!(
            names(&tree, PaneSort::Alphabetical),
            [
                "a",
                "common",
                "kappa",
                "lambda",
                "meta",
                "project",
                "project/alpha",
                "project/beta",
                "project/beta/deep",
                "single",
                "topic"
            ]
        );
        sort_tags(&mut tree, PaneSort::AlphabeticalReverse);
        assert_eq!(
            names(&tree, PaneSort::AlphabeticalReverse),
            [
                "topic",
                "single",
                "project",
                "project/beta",
                "project/beta/deep",
                "project/alpha",
                "meta",
                "lambda",
                "kappa",
                "common",
                "a"
            ]
        );
        sort_tags(&mut tree, PaneSort::FrequencyReverse);
        assert_eq!(
            names(&tree, PaneSort::FrequencyReverse),
            [
                "a",
                "kappa",
                "meta",
                "single",
                "lambda",
                "topic",
                "project",
                "project/beta",
                "project/beta/deep",
                "project/alpha",
                "common"
            ]
        );
    }

    #[test]
    fn flat_mode_lists_every_tag_at_depth_zero_with_its_parent_part() {
        let mut tree = tag_tree(&probe_tags());
        sort_tags(&mut tree, PaneSort::Alphabetical);
        let rows = tag_rows(&tree, PaneSort::Alphabetical, false, &HashSet::new(), "");
        assert!(rows.iter().all(|r| r.depth == 0 && !r.has_children));
        let deep = rows.iter().find(|r| r.key == "project/beta/deep").unwrap();
        assert_eq!(deep.parent, "project/beta/");
        assert_eq!(deep.name, "deep");
        assert_eq!(rows.iter().find(|r| r.key == "project").unwrap().count, 6);
        let keys: Vec<&str> = rows.iter().map(|r| r.key.as_str()).collect();
        assert_eq!(
            keys[5..9],
            [
                "project",
                "project/alpha",
                "project/beta",
                "project/beta/deep"
            ]
        );
    }

    #[test]
    fn folds_hide_children_and_keep_the_count() {
        let mut tree = tag_tree(&probe_tags());
        sort_tags(&mut tree, PaneSort::Frequency);
        let folded: HashSet<String> = ["project".to_string()].into_iter().collect();
        let rows = tag_rows(&tree, PaneSort::Frequency, true, &folded, "");
        assert_eq!(rows.len(), 8);
        let project = &rows[1];
        assert!(project.collapsed && project.count == 6);
        assert_eq!(tag_keys(&tree).len(), 11);
    }

    #[test]
    fn the_filter_keeps_the_matching_subtree_and_nothing_for_zzz() {
        let mut tree = tag_tree(&probe_tags());
        sort_tags(&mut tree, PaneSort::Frequency);
        let rows = tag_rows(&tree, PaneSort::Frequency, true, &HashSet::new(), "proj");
        let keys: Vec<&str> = rows.iter().map(|r| r.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "project",
                "project/alpha",
                "project/beta",
                "project/beta/deep"
            ]
        );
        // A child match keeps its ancestors on the list.
        let rows = tag_rows(&tree, PaneSort::Frequency, true, &HashSet::new(), "DEEP");
        let keys: Vec<&str> = rows.iter().map(|r| r.key.as_str()).collect();
        assert_eq!(keys, ["project", "project/beta", "project/beta/deep"]);
        assert!(tag_rows(&tree, PaneSort::Frequency, true, &HashSet::new(), "zzz").is_empty());
        assert_eq!(
            tag_rows(&tree, PaneSort::Frequency, false, &HashSet::new(), "com").len(),
            1
        );
    }

    fn prop(name: &str, count: usize, ty: Option<PropertyType>) -> PropertyEntry {
        PropertyEntry {
            name: name.to_string(),
            count,
            ty,
        }
    }

    #[test]
    fn property_rows_sort_filter_and_carry_the_widget_icon() {
        let entries = vec![
            prop("status", 4, Some(PropertyType::Text)),
            prop("tags", 4, Some(PropertyType::Tags)),
            prop("title", 3, Some(PropertyType::Text)),
            prop("count", 2, Some(PropertyType::Number)),
            prop("book.year", 1, Some(PropertyType::Number)),
            prop("book.title", 1, Some(PropertyType::Text)),
            prop("done", 1, Some(PropertyType::Checkbox)),
            prop("shelf", 1, None),
            prop("aliases", 1, Some(PropertyType::Aliases)),
        ];
        let names = |order: PaneSort, q: &str| -> Vec<String> {
            property_rows(&entries, order, q)
                .into_iter()
                .map(|r| r.name)
                .collect()
        };
        assert_eq!(
            names(PaneSort::Frequency, ""),
            [
                "status",
                "tags",
                "title",
                "count",
                "aliases",
                "book.title",
                "book.year",
                "done",
                "shelf"
            ]
        );
        assert_eq!(
            names(PaneSort::FrequencyReverse, ""),
            [
                "aliases",
                "book.title",
                "book.year",
                "done",
                "shelf",
                "count",
                "title",
                "status",
                "tags"
            ]
        );
        assert_eq!(
            names(PaneSort::Alphabetical, "")[..3],
            ["aliases", "book.title", "book.year"]
        );
        assert_eq!(names(PaneSort::AlphabeticalReverse, "")[0], "title");
        assert_eq!(
            names(PaneSort::Frequency, "boo"),
            ["book.title", "book.year"]
        );
        assert!(names(PaneSort::Frequency, "zzz").is_empty());
        let rows = property_rows(&entries, PaneSort::Frequency, "");
        let icon = |n: &str| rows.iter().find(|r| r.name == n).unwrap().icon;
        assert_eq!(icon("status"), "text");
        assert_eq!(icon("tags"), "tags");
        assert_eq!(icon("count"), "binary");
        assert_eq!(icon("done"), "square-check");
        assert_eq!(icon("shelf"), "file-question");
        assert_eq!(icon("aliases"), "forward");
        assert_eq!(property_icon(Some(PropertyType::Date)), "calendar");
        assert_eq!(property_icon(Some(PropertyType::DateTime)), "clock");
        assert_eq!(property_icon(Some(PropertyType::MultiText)), "list");
    }

    fn heading(text: &str, level: u8, line: usize) -> Heading {
        Heading {
            text: text.to_string(),
            level,
            line,
            offset: line * 10,
        }
    }

    /// The probe's note: nesting is structural, not by the number of `#`.
    fn probe_headings() -> Vec<Heading> {
        vec![
            heading("Top heading", 1, 4),
            heading("Second level", 2, 19),
            heading("Third level under second", 3, 34),
            heading("Fourth level", 4, 49),
            heading("Another second", 2, 64),
            heading("Fourth under second, skipping third", 4, 79),
            heading("Second top", 1, 94),
            heading("Third under top, skipping second", 3, 109),
            heading("Last top", 1, 124),
        ]
    }

    #[test]
    fn headings_nest_under_the_nearest_smaller_level() {
        assert_eq!(
            outline_depths(&probe_headings()),
            [0, 1, 2, 3, 1, 2, 0, 1, 0]
        );
        let rows = outline_rows(&probe_headings(), &HashSet::new(), "", None);
        assert_eq!(rows.len(), 9);
        assert!(rows[0].has_children && !rows[3].has_children);
        assert!(rows[6].has_children && !rows[8].has_children);
    }

    #[test]
    fn the_section_holding_the_cursor_is_marked_unless_hidden() {
        let hs = probe_headings();
        assert_eq!(heading_at(&hs, 10), None, "above the first heading");
        assert_eq!(heading_at(&hs, 40), Some(0));
        assert_eq!(heading_at(&hs, 510), Some(3));
        assert_eq!(heading_at(&hs, 640), Some(4));
        assert_eq!(heading_at(&hs, 9999), Some(8));
        let rows = outline_rows(&hs, &HashSet::new(), "", Some(3));
        assert!(rows[3].active && !rows[0].active);
        // Folding Top heading hides Fourth level: nothing is marked, the
        // parent included.
        let folded: HashSet<usize> = [0].into_iter().collect();
        let rows = outline_rows(&hs, &folded, "", Some(3));
        assert_eq!(rows.len(), 4);
        assert!(rows.iter().all(|r| !r.active));
        assert!(rows[0].collapsed);
    }

    #[test]
    fn the_filter_keeps_matches_and_their_ancestors_with_the_ranges() {
        let rows = outline_rows(&probe_headings(), &HashSet::new(), "second", None);
        let texts: Vec<&str> = rows.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(
            texts,
            [
                "Top heading",
                "Second level",
                "Third level under second",
                "Another second",
                "Fourth under second, skipping third",
                "Second top",
                "Third under top, skipping second",
            ]
        );
        assert!(
            rows[0].highlights.is_empty(),
            "an ancestor is kept unmarked"
        );
        assert_eq!(rows[1].highlights, vec![0..6]);
        assert_eq!(&rows[2].text[rows[2].highlights[0].clone()], "second");
        assert!(outline_rows(&probe_headings(), &HashSet::new(), "zzz", None).is_empty());
    }

    #[test]
    fn find_all_handles_repeats_and_multibyte_text() {
        assert_eq!(find_all("aAa", "a"), vec![0..1, 1..2, 2..3]);
        assert_eq!(find_all("Éclair éclair", "éclair"), vec![0..7, 8..15]);
        assert!(find_all("abc", "").is_empty());
    }
}
