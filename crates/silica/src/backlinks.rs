//! The rows of the Backlinks and Outgoing links panes, in the shape
//! Obsidian lays them out: a section ("Linked mentions", "Unlinked
//! mentions", "Links") holds one collapsible group per file, and a group
//! holds the matches in that file, each with its context and the matched
//! text's range inside it so the pane can highlight it.
//!
//! Nothing here touches gpui; the workspace builds these off the index and
//! draws them.

use crate::explorer::{display_name, SortOrder};
use crate::mentions::Finder;
use rayon::prelude::*;
use silica_cache::links::{link_path, LinkFormat};
use silica_cache::{LinkResolver, MetadataCache};
use silica_formats::FileKind;
use silica_markdown::{parse_linktext, Subpath};
use silica_search::natural_cmp;
use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

/// The most unlinked mentions a section lists in either direction.
pub const MAX_MENTIONS: usize = 200;

pub const LINKED: &str = "Linked mentions";
pub const UNLINKED: &str = "Unlinked mentions";
pub const LINKS: &str = "Links";

/// One occurrence in one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    /// The context shown: the occurrence's line, trimmed, or with more
    /// context the lines either side of it too, joined by newlines.
    pub text: String,
    /// The matched text inside `text`.
    pub ranges: Vec<Range<usize>>,
    /// Byte offset in the file to reveal on a click.
    pub offset: Option<usize>,
    /// An unlinked mention's own bytes in the file, which the Link button
    /// rewrites as a link.
    pub mention: Option<Range<usize>>,
    /// The note an outgoing unlinked mention names, which the "Link this
    /// file" chip under it shows and links to.
    pub target: Option<String>,
}

/// The matches in one file, or one outgoing link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    /// The file to open; `None` when the link resolves to nothing.
    pub path: Option<String>,
    pub label: String,
    /// The link as written, for a row of the Links section: a click follows
    /// it the way clicking the link in the note does, subpath and all, and
    /// creates the note when nothing stands behind it.
    pub link: Option<String>,
    /// A Links row's second line: the note a heading or block belongs to,
    /// or the folder a note sits in.
    pub subtext: Option<String>,
    /// A Links row's glyph: `link` for a note, `heading` or `layout-list`
    /// for a subpath, `file-plus` for a link nothing stands behind.
    pub icon: Option<&'static str>,
    pub matches: Vec<Match>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub label: &'static str,
    pub groups: Vec<Group>,
    /// How many there are, which is more than are listed when `capped`.
    pub count: usize,
    pub capped: bool,
}

impl Section {
    pub fn empty(label: &'static str) -> Section {
        Section {
            label,
            groups: Vec::new(),
            count: 0,
            capped: false,
        }
    }

    fn of(label: &'static str, groups: Vec<Group>) -> Section {
        // A Links row stands for one link; a mentions group for its matches.
        let count = groups.iter().map(|g| g.matches.len().max(1)).sum();
        Section {
            label,
            groups,
            count,
            capped: false,
        }
    }

    /// The key a group's collapse is remembered under.
    pub fn group_key(&self, group: &Group) -> String {
        format!(
            "{}:{}",
            self.label,
            group.path.as_deref().unwrap_or(&group.label)
        )
    }
}

fn label_of(path: &str) -> String {
    display_name(path.rsplit('/').next().unwrap_or(path)).to_string()
}

/// The context of `range` in `text`: its line trimmed, with the range moved
/// to match; with `extra`, the non-blank line before and after it as well.
pub fn context(text: &str, range: Range<usize>, extra: bool) -> (String, Range<usize>) {
    let len = text.len();
    let clamp = |at: usize| {
        let mut at = at.min(len);
        while at > 0 && !text.is_char_boundary(at) {
            at -= 1;
        }
        at
    };
    let range = clamp(range.start)..clamp(range.end.max(range.start));
    let line_start = |at: usize| text[..at].rfind('\n').map_or(0, |i| i + 1);
    let line_end = |at: usize| text[at..].find('\n').map_or(len, |i| at + i);
    let mut start = line_start(range.start);
    let mut end = line_end(range.end);
    if extra {
        if start > 0 {
            let before = line_start(start - 1);
            if !text[before..start].trim().is_empty() {
                start = before;
            }
        }
        if end < len {
            let after = line_end((end + 1).min(len));
            if !text[end..after].trim().is_empty() {
                end = after;
            }
        }
    }
    let mut out = String::new();
    let mut mapped = 0..0;
    let mut at = start;
    for (i, line) in text[start..end].split('\n').enumerate() {
        let body = line.trim_end_matches('\r');
        let lead = body.len() - body.trim_start().len();
        let trimmed = body.trim();
        if i > 0 {
            out.push('\n');
        }
        let base = out.len();
        let map = |p: usize| base + p.saturating_sub(at + lead).min(trimmed.len());
        if range.start >= at && range.start <= at + line.len() {
            mapped.start = map(range.start);
        }
        if range.end >= at && range.end <= at + line.len() {
            mapped.end = map(range.end);
        }
        out.push_str(trimmed);
        at += line.len() + 1;
    }
    if mapped.end < mapped.start {
        mapped.end = mapped.start;
    }
    (out, mapped)
}

/// Linked mentions: every link into `path`, grouped by the note it was
/// written in. A link written in a property shows the property's name as
/// its context, which is what Obsidian names for one.
pub fn linked_mentions(
    cache: &MetadataCache,
    texts: &HashMap<String, Arc<String>>,
    path: &str,
    extra: bool,
) -> Section {
    let mut groups: Vec<Group> = Vec::new();
    for b in cache.backlinks(path) {
        let offset = b.link.position.start.offset;
        let m = match (b.key, texts.get(b.source)) {
            (Some(key), _) => Match {
                text: key.to_string(),
                ranges: Vec::new(),
                offset: Some(offset),
                mention: None,
                target: None,
            },
            (None, Some(text)) => {
                let (snippet, range) = context(text, offset..b.link.position.end.offset, extra);
                Match {
                    text: snippet,
                    ranges: vec![range],
                    offset: Some(offset),
                    mention: None,
                    target: None,
                }
            }
            (None, None) => Match {
                text: String::new(),
                ranges: Vec::new(),
                offset: Some(offset),
                mention: None,
                target: None,
            },
        };
        match groups
            .iter_mut()
            .find(|g| g.path.as_deref() == Some(b.source))
        {
            Some(g) => g.matches.push(m),
            None => groups.push(Group {
                path: Some(b.source.to_string()),
                label: label_of(b.source),
                link: None,
                subtext: None,
                icon: None,
                matches: vec![m],
            }),
        }
    }
    Section::of(LINKED, groups)
}

/// Links: one row per distinct target `path` links to, the properties
/// first and the body in order of first occurrence, as Obsidian lists them:
/// `[[Beta]]`, `[Beta](Beta.md)` and `![[Beta]]` are one "Beta" row. A
/// heading or block link is labelled by that heading or block with the
/// note under it; a note link by the path as written with its folder
/// under it; an alias is never shown.
pub fn outgoing_links(cache: &MetadataCache, path: &str) -> Section {
    let Some(meta) = cache.file(path) else {
        return Section::empty(LINKS);
    };
    let properties = meta.frontmatter_links.iter().map(|l| l.link.as_str());
    let body = meta
        .links
        .iter()
        .chain(meta.embeds.iter())
        .map(|l| l.link.as_str());
    let mut seen: Vec<String> = Vec::new();
    let mut groups: Vec<Group> = Vec::new();
    for link in properties.chain(body) {
        let parsed = parse_linktext(link);
        let target = cache.resolve(link, path).map(str::to_string);
        let written = if parsed.path.is_empty() {
            path.to_string()
        } else {
            strip_md(parsed.path).to_string()
        };
        let subpath = parsed
            .subpath
            .as_ref()
            .map(|s| match s {
                Subpath::Headings(h) => format!("#{}", h.join("#")),
                Subpath::Block(b) => format!("#^{b}"),
            })
            .unwrap_or_default();
        let key = format!(
            "{}{}",
            target.as_deref().unwrap_or(&written).to_lowercase(),
            subpath.to_lowercase()
        );
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        let note = target.as_deref().unwrap_or(&written);
        let other = target.as_deref() != Some(path);
        let (label, subtext, icon) = match (&parsed.subpath, target.is_some()) {
            (Some(Subpath::Headings(h)), _) => (
                h.last().map(|s| s.to_string()).unwrap_or_default(),
                other.then(|| label_of(note)),
                "heading",
            ),
            (Some(Subpath::Block(b)), _) => {
                (b.to_string(), other.then(|| label_of(note)), "layout-list")
            }
            (None, true) => (written.clone(), folder_of(note), "link"),
            (None, false) => (written.clone(), folder_of(&written), "file-plus"),
        };
        groups.push(Group {
            label,
            path: target,
            link: Some(link.to_string()),
            subtext,
            icon: Some(icon),
            matches: Vec::new(),
        });
    }
    Section::of(LINKS, groups)
}

fn strip_md(path: &str) -> &str {
    path.strip_suffix(".md")
        .or_else(|| path.strip_suffix(".MD"))
        .unwrap_or(path)
}

/// The folder part of a path, `None` at the root.
fn folder_of(path: &str) -> Option<String> {
    path.rsplit_once('/')
        .map(|(folder, _)| folder.to_string())
        .filter(|f| !f.is_empty())
}

/// Unlinked mentions of `own` (a note's name and aliases) in every other
/// note: one group per note, each mention once, at most [`MAX_MENTIONS`]
/// in all.
pub fn unlinked_incoming(
    texts: &HashMap<String, Arc<String>>,
    path: &str,
    own: Option<&Finder>,
    extra: bool,
) -> Section {
    let Some(finder) = own else {
        return Section::empty(UNLINKED);
    };
    let mut found: Vec<(String, Range<usize>)> = texts
        .par_iter()
        .filter(|(p, _)| p.as_str() != path)
        .flat_map_iter(|(p, text)| {
            finder
                .find(text)
                .into_iter()
                .map(|m| (p.clone(), m.range))
                .collect::<Vec<_>>()
        })
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.start.cmp(&b.1.start)));
    found.dedup();
    let total = found.len();
    let mut groups: Vec<Group> = Vec::new();
    for (p, range) in found.into_iter().take(MAX_MENTIONS) {
        let Some(text) = texts.get(&p) else {
            continue;
        };
        let m = mention_match(text, range, extra, None);
        match groups.last_mut() {
            Some(g) if g.path.as_deref() == Some(p.as_str()) => g.matches.push(m),
            _ => groups.push(Group {
                label: label_of(&p),
                path: Some(p),
                link: None,
                subtext: None,
                icon: None,
                matches: vec![m],
            }),
        }
    }
    let mut section = Section::of(UNLINKED, groups);
    section.count = total;
    section.capped = total > MAX_MENTIONS;
    section
}

/// Unlinked mentions of other notes in `path`'s own text: one group, the
/// note itself, its mentions in text order, each naming the note it
/// mentions, which is how Obsidian's Outgoing links pane lists them.
pub fn unlinked_outgoing(text: &str, path: &str, names: Option<&Finder>, extra: bool) -> Section {
    let Some(finder) = names else {
        return Section::empty(UNLINKED);
    };
    let mut found: Vec<(String, Range<usize>)> = finder
        .find(text)
        .into_iter()
        .filter(|m| m.path != path)
        .map(|m| (m.path, m.range))
        .collect();
    found.sort_by(|a, b| a.1.start.cmp(&b.1.start).then(a.0.cmp(&b.0)));
    found.dedup();
    let total = found.len();
    let matches: Vec<Match> = found
        .into_iter()
        .take(MAX_MENTIONS)
        .map(|(p, range)| mention_match(text, range, extra, Some(p)))
        .collect();
    let groups = if matches.is_empty() {
        Vec::new()
    } else {
        vec![Group {
            label: label_of(path),
            path: Some(path.to_string()),
            link: None,
            subtext: None,
            icon: None,
            matches,
        }]
    };
    let mut section = Section::of(UNLINKED, groups);
    section.count = total;
    section.capped = total > MAX_MENTIONS;
    section
}

fn mention_match(text: &str, range: Range<usize>, extra: bool, target: Option<String>) -> Match {
    let (snippet, mapped) = context(text, range.clone(), extra);
    Match {
        text: snippet,
        ranges: vec![mapped],
        offset: Some(range.start),
        mention: Some(range),
        target,
    }
}

/// Orders a section's groups the way the pane's sort order says: by name
/// with Obsidian's collation, or by the file times `stat` reports
/// (modified, created), newest first for the "new to old" orders.
pub fn sort_groups(section: &mut Section, order: SortOrder, stat: &dyn Fn(&str) -> (u64, u64)) {
    let time = |g: &Group| match g.path.as_deref() {
        Some(p) => stat(p),
        None => (0, 0),
    };
    section.groups.sort_by(|a, b| {
        let by_name = natural_cmp(&a.label, &b.label).then_with(|| a.path.cmp(&b.path));
        match order {
            SortOrder::Alphabetical => by_name,
            SortOrder::AlphabeticalReverse => by_name.reverse(),
            SortOrder::ByModifiedTime => time(b).0.cmp(&time(a).0).then(by_name),
            SortOrder::ByModifiedTimeReverse => time(a).0.cmp(&time(b).0).then(by_name),
            SortOrder::ByCreatedTime => time(b).1.cmp(&time(a).1).then(by_name),
            SortOrder::ByCreatedTimeReverse => time(a).1.cmp(&time(b).1).then(by_name),
        }
    });
}

/// The section with only what `query` matches, case-insensitively: a group
/// whose name or path matches keeps every match, any other keeps the
/// matches whose context matches. An empty query keeps everything.
pub fn filter(section: &Section, query: &str) -> Section {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return section.clone();
    }
    let hit = |s: &str| s.to_lowercase().contains(&query);
    let groups: Vec<Group> = section
        .groups
        .iter()
        .filter_map(|g| {
            if hit(&g.label) || g.path.as_deref().is_some_and(hit) {
                return Some(g.clone());
            }
            let matches: Vec<Match> = g.matches.iter().filter(|m| hit(&m.text)).cloned().collect();
            (!matches.is_empty()).then(|| Group {
                matches,
                ..g.clone()
            })
        })
        .collect();
    let count = groups.iter().map(|g| g.matches.len().max(1)).sum();
    Section {
        label: section.label,
        groups,
        count,
        capped: false,
    }
}

/// The link the Link button writes over an unlinked mention: the target in
/// the vault's link format, with the mentioned text kept as the alias when
/// it is not the path part itself (an alias, another case, a folder).
pub fn link_for_mention(
    resolver: &LinkResolver,
    target: &str,
    source: &str,
    mentioned: &str,
    format: LinkFormat,
    markdown_links: bool,
) -> String {
    let path = link_path(resolver, target, source, format);
    if markdown_links {
        let dest = if FileKind::from_path(target).is_markdown() {
            format!("{path}.md")
        } else {
            path
        };
        format!("[{mentioned}]({})", dest.replace(' ', "%20"))
    } else if path == mentioned {
        format!("[[{path}]]")
    } else {
        format!("[[{path}|{mentioned}]]")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mentions::Name;

    #[test]
    fn context_is_the_trimmed_line_with_the_range_moved() {
        let text = "first\n  See [[Plan]] here.\r\nlast\n";
        let at = text.find("[[Plan]]").unwrap();
        let (line, range) = context(text, at..at + 8, false);
        assert_eq!(line, "See [[Plan]] here.");
        assert_eq!(&line[range], "[[Plan]]");
    }

    #[test]
    fn more_context_adds_the_lines_either_side_but_not_blank_ones() {
        let text = "intro\n\nabove\n  the plan line\nbelow\n\nfar";
        let at = text.find("plan").unwrap();
        let (snippet, range) = context(text, at..at + 4, true);
        assert_eq!(snippet, "above\nthe plan line\nbelow");
        assert_eq!(&snippet[range], "plan");

        let at = text.find("far").unwrap();
        let (snippet, range) = context(text, at..at + 3, true);
        assert_eq!(snippet, "far", "a blank neighbour is not pulled in");
        assert_eq!(&snippet[range], "far");
    }

    #[test]
    fn a_stale_range_past_the_text_does_not_panic() {
        let (snippet, range) = context("short", 40..50, true);
        assert_eq!(snippet, "short");
        assert_eq!(range, 5..5);
    }

    #[test]
    fn unlinked_mentions_are_grouped_per_file_and_listed_once() {
        let names = [Name {
            text: "Plan",
            path: "Plan.md",
        }];
        let finder = Finder::new(&names).unwrap();
        let mut texts = HashMap::new();
        texts.insert(
            "A.md".to_string(),
            Arc::new("the plan\nno link [[Plan]]\nPlan again".to_string()),
        );
        texts.insert("B.md".to_string(), Arc::new("plan".to_string()));
        texts.insert("Plan.md".to_string(), Arc::new("plan itself".to_string()));
        let section = unlinked_incoming(&texts, "Plan.md", Some(&finder), false);
        assert_eq!(section.label, UNLINKED);
        assert_eq!(section.count, 3);
        assert!(!section.capped);
        let shown: Vec<(&str, Vec<&str>)> = section
            .groups
            .iter()
            .map(|g| {
                (
                    g.path.as_deref().unwrap(),
                    g.matches
                        .iter()
                        .map(|m| &m.text[m.ranges[0].clone()])
                        .collect(),
                )
            })
            .collect();
        assert_eq!(
            shown,
            vec![("A.md", vec!["plan", "Plan"]), ("B.md", vec!["plan"])]
        );
        let first = &section.groups[0].matches[0];
        assert_eq!(first.mention, Some(4..8));
        assert_eq!(first.offset, Some(4));

        let out = unlinked_outgoing("A plan and a plan", "A.md", Some(&finder), false);
        assert_eq!(out.count, 2);
        assert_eq!(out.groups.len(), 1, "one card: the note itself");
        assert_eq!(out.groups[0].path.as_deref(), Some("A.md"));
        assert_eq!(out.groups[0].matches[1].mention, Some(13..17));
        assert_eq!(out.groups[0].matches[1].target.as_deref(), Some("Plan.md"));
    }

    /// One row per target, in first-occurrence order with the properties
    /// first; a heading or block row is labelled by it with the note
    /// under; a note row by the written path with its folder under; a
    /// self link and a root note carry no subtext; the alias is dropped.
    #[test]
    fn links_rows_are_one_per_target_with_obsidians_labels() {
        let mut cache = MetadataCache::default();
        let alpha = "---\nrelated: \"[[Gamma]]\"\n---\nSee [[Beta]] and [[Beta#Heading|the alias]] and [[Beta]], then [[Missing]], [Beta link](Beta.md), ![[Beta]], [[#Local]], [[Gamma#^blk]] and [[Sub/Deep]].\n\n## Local\n";
        cache.insert_file("Alpha.md".into(), silica_cache::index_markdown(alpha));
        for p in ["Beta.md", "Gamma.md", "Sub/Deep.md"] {
            cache.add_path(p);
        }
        cache.reresolve();
        let section = outgoing_links(&cache, "Alpha.md");
        let rows: Vec<(&str, Option<&str>, &str, Option<&str>)> = section
            .groups
            .iter()
            .map(|g| {
                (
                    g.label.as_str(),
                    g.subtext.as_deref(),
                    g.icon.unwrap(),
                    g.path.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            rows,
            vec![
                ("Gamma", None, "link", Some("Gamma.md")),
                ("Beta", None, "link", Some("Beta.md")),
                ("Heading", Some("Beta"), "heading", Some("Beta.md")),
                ("Missing", None, "file-plus", None),
                ("Local", None, "heading", Some("Alpha.md")),
                ("blk", Some("Gamma"), "layout-list", Some("Gamma.md")),
                ("Sub/Deep", Some("Sub"), "link", Some("Sub/Deep.md")),
            ]
        );
        assert_eq!(section.count, 7);
        assert_eq!(section.groups[2].link.as_deref(), Some("Beta#Heading"));
    }

    #[test]
    fn groups_sort_by_name_or_by_the_times_given() {
        let group = |label: &str| Group {
            path: Some(format!("{label}.md")),
            label: label.to_string(),
            link: None,
            subtext: None,
            icon: None,
            matches: Vec::new(),
        };
        let mut section = Section::of(LINKED, vec![group("b"), group("a10"), group("a2")]);
        let stat = |p: &str| match p {
            "b.md" => (3, 1),
            "a10.md" => (1, 3),
            _ => (2, 2),
        };
        fn names(s: &Section) -> Vec<&str> {
            s.groups.iter().map(|g| g.label.as_str()).collect()
        }
        sort_groups(&mut section, SortOrder::Alphabetical, &stat);
        assert_eq!(names(&section), ["a2", "a10", "b"]);
        sort_groups(&mut section, SortOrder::AlphabeticalReverse, &stat);
        assert_eq!(names(&section), ["b", "a10", "a2"]);
        sort_groups(&mut section, SortOrder::ByModifiedTime, &stat);
        assert_eq!(names(&section), ["b", "a2", "a10"]);
        sort_groups(&mut section, SortOrder::ByCreatedTimeReverse, &stat);
        assert_eq!(names(&section), ["b", "a2", "a10"]);
    }

    #[test]
    fn the_filter_keeps_matching_files_whole_and_other_files_matching_lines() {
        let m = |text: &str| Match {
            text: text.to_string(),
            ranges: vec![0..1, 1..1],
            offset: None,
            mention: None,
            target: None,
        };
        let section = Section::of(
            LINKED,
            vec![
                Group {
                    path: Some("Notes/Apple.md".into()),
                    label: "Apple".into(),
                    link: None,
                    subtext: None,
                    icon: None,
                    matches: vec![m("one"), m("two")],
                },
                Group {
                    path: Some("Pear.md".into()),
                    label: "Pear".into(),
                    link: None,
                    subtext: None,
                    icon: None,
                    matches: vec![m("an apple"), m("a fig")],
                },
            ],
        );
        let kept = filter(&section, "APPLE");
        assert_eq!(kept.count, 3);
        assert_eq!(kept.groups[0].matches.len(), 2);
        assert_eq!(kept.groups[1].matches.len(), 1);
        assert_eq!(filter(&section, "  ").count, 4);
        assert!(filter(&section, "zzz").groups.is_empty());
    }

    #[test]
    fn the_link_written_keeps_the_mentioned_text_as_an_alias_when_it_differs() {
        let mut cache = MetadataCache::default();
        cache.add_path("Plan.md");
        cache.add_path("Sub/Deep.md");
        cache.add_path("Other/Deep.md");
        cache.reresolve();
        let r = cache.resolver();
        let link = |target: &str, mentioned: &str, md: bool| {
            link_for_mention(r, target, "A.md", mentioned, LinkFormat::Shortest, md)
        };
        assert_eq!(link("Plan.md", "Plan", false), "[[Plan]]");
        assert_eq!(link("Plan.md", "plan", false), "[[Plan|plan]]");
        // Two notes share the name: the one the short form does not
        // reach is written with its folder, the text kept as the alias.
        let far = if r.resolve("Deep", "A.md") == Some("Sub/Deep.md") {
            "Other/Deep.md"
        } else {
            "Sub/Deep.md"
        };
        assert_eq!(
            link(far, "Deep", false),
            format!("[[{}|Deep]]", far.trim_end_matches(".md"))
        );
        assert_eq!(link("Plan.md", "the plan", true), "[the plan](Plan.md)");
    }
}
