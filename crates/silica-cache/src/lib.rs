//! Metadata cache.
//!
//! `index_markdown` turns one note into a `CachedMetadata` record that mirrors
//! Obsidian's public `CachedMetadata` shape, built from the `silica-markdown`
//! tree so that the cache, the editor and the reading view agree on every
//! edge case. `LinkResolver` holds the vault-wide indexes needed to turn link
//! text into a file path with Obsidian's rules.

pub mod links;
pub mod properties;
pub mod resolver;
pub mod vault;

pub use links::{basename, link_path, rewrite_note, LinkFormat, Rewrite};
pub use properties::note_properties;
pub use resolver::LinkResolver;
pub use silica_markdown::{LineIndex, Loc, Pos, Span};
pub use vault::{
    all_properties, all_tags, frontmatter_aliases, frontmatter_tags, Backlink, MetadataCache,
    PropertyEntry, TagEntry,
};

use silica_markdown::{
    parse, walk_inlines, Block, BlockKind, Document, Inline, InlineKind, ListItem,
};
use std::collections::{BTreeMap, BTreeSet};

/// Obsidian's `SectionCache.type` vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SectionKind {
    Paragraph,
    Heading,
    List,
    Code,
    Blockquote,
    Table,
    ThematicBreak,
    Yaml,
    Html,
    FootnoteDefinition,
    Callout,
    Math,
    Comment,
}

impl SectionKind {
    /// The string Obsidian stores in `SectionCache.type`.
    pub fn as_str(self) -> &'static str {
        match self {
            SectionKind::Paragraph => "paragraph",
            SectionKind::Heading => "heading",
            SectionKind::List => "list",
            SectionKind::Code => "code",
            SectionKind::Blockquote => "blockquote",
            SectionKind::Table => "table",
            SectionKind::ThematicBreak => "thematicBreak",
            SectionKind::Yaml => "yaml",
            SectionKind::Html => "html",
            SectionKind::FootnoteDefinition => "footnoteDefinition",
            SectionKind::Callout => "callout",
            SectionKind::Math => "math",
            SectionKind::Comment => "comment",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkCache {
    /// Parsed target (`Note#Heading`), without the alias; percent-decoded for markdown links.
    pub link: String,
    /// The full original text including brackets.
    pub original: String,
    pub display_text: Option<String>,
    pub position: Loc,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagCache {
    /// Includes the leading `#`.
    pub tag: String,
    pub position: Loc,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadingCache {
    /// The heading text as written, with inline markup left in place.
    pub heading: String,
    pub level: u8,
    pub position: Loc,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionCache {
    pub kind: SectionKind,
    /// `^id` attached to the section.
    pub id: Option<String>,
    pub position: Loc,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListItemCache {
    /// Start line of the parent item, or for a root item the negative of the
    /// start line of the list's first item (Obsidian's encoding).
    pub parent: i64,
    /// The character inside `[ ]` for task items.
    pub task: Option<char>,
    pub id: Option<String>,
    pub position: Loc,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockCache {
    pub id: String,
    /// Covers the whole block the id belongs to.
    pub position: Loc,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontmatterLinkCache {
    /// Property path such as `related` or `aliases.1`.
    pub key: String,
    pub link: String,
    pub original: String,
    pub display_text: Option<String>,
    /// Obsidian's own `FrontmatterLinkCache` has no position, but the
    /// backlinks pane has to show a property link beside its line like any
    /// other occurrence, so the indexer finds the written text again in the
    /// raw YAML.
    pub position: Loc,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FootnoteCache {
    pub id: String,
    pub position: Loc,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CachedMetadata {
    pub links: Vec<LinkCache>,
    pub embeds: Vec<LinkCache>,
    pub tags: Vec<TagCache>,
    pub headings: Vec<HeadingCache>,
    pub sections: Vec<SectionCache>,
    pub list_items: Vec<ListItemCache>,
    pub blocks: BTreeMap<String, BlockCache>,
    pub frontmatter: Option<serde_yaml::Value>,
    pub frontmatter_position: Option<Loc>,
    pub frontmatter_links: Vec<FrontmatterLinkCache>,
    /// Footnote definitions.
    pub footnotes: Vec<FootnoteCache>,
    pub footnote_refs: Vec<FootnoteCache>,
}

/// Builds the metadata record for one markdown file.
pub fn index_markdown(src: &str) -> CachedMetadata {
    let doc = parse(src);
    index_document(&doc)
}

/// Builds the metadata record from an already parsed note.
pub fn index_document(doc: &Document<'_>) -> CachedMetadata {
    let mut ix = Indexer {
        src: doc.source,
        lines: LineIndex::new(doc.source),
        meta: CachedMetadata::default(),
    };
    for block in &doc.blocks {
        ix.meta.sections.push(SectionCache {
            kind: section_kind(block),
            id: block.id.map(|i| i.id.to_string()),
            position: ix.lines.loc(block.span),
        });
    }
    ix.blocks(&doc.blocks);
    ix.meta
}

fn section_kind(block: &Block<'_>) -> SectionKind {
    match &block.kind {
        BlockKind::Yaml { .. } => SectionKind::Yaml,
        BlockKind::Paragraph { .. } => SectionKind::Paragraph,
        BlockKind::Heading { .. } => SectionKind::Heading,
        BlockKind::ThematicBreak => SectionKind::ThematicBreak,
        BlockKind::CodeBlock { .. } => SectionKind::Code,
        BlockKind::HtmlBlock { .. } => SectionKind::Html,
        BlockKind::BlockQuote { .. } => SectionKind::Blockquote,
        BlockKind::Callout { .. } => SectionKind::Callout,
        BlockKind::List { .. } => SectionKind::List,
        BlockKind::Table { .. } => SectionKind::Table,
        BlockKind::Math { .. } => SectionKind::Math,
        BlockKind::Comment { .. } => SectionKind::Comment,
        BlockKind::FootnoteDefinition { .. } => SectionKind::FootnoteDefinition,
    }
}

struct Indexer<'a> {
    src: &'a str,
    lines: LineIndex,
    meta: CachedMetadata,
}

/// Whether a markdown link destination points into the vault rather than at a URL.
fn is_vault_destination(dest: &str) -> bool {
    let b = dest.as_bytes();
    if b.first().is_some_and(|c| c.is_ascii_alphabetic()) {
        let scheme_len = b
            .iter()
            .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'.' | b'-'))
            .count();
        if scheme_len >= 2 && b.get(scheme_len) == Some(&b':') {
            return false;
        }
    }
    true
}

pub(crate) fn percent_decode(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hex(b.get(i + 1)), hex(b.get(i + 2))) {
                out.push(h << 4 | l);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

fn hex(c: Option<&u8>) -> Option<u8> {
    (*c? as char).to_digit(16).map(|d| d as u8)
}

impl<'a> Indexer<'a> {
    fn blocks(&mut self, blocks: &[Block<'a>]) {
        for block in blocks {
            self.block(block);
        }
    }

    fn block(&mut self, block: &Block<'a>) {
        if let Some(id) = block.id {
            self.meta.blocks.insert(
                id.id.to_string(),
                BlockCache {
                    id: id.id.to_string(),
                    position: self.lines.loc(block.span),
                },
            );
        }
        match &block.kind {
            BlockKind::Yaml { yaml } => {
                self.meta.frontmatter = serde_yaml::from_str::<serde_yaml::Value>(yaml)
                    .ok()
                    .filter(|v| v.is_mapping());
                self.meta.frontmatter_position = Some(self.lines.loc(block.span));
                if let Some(fm) = self.meta.frontmatter.clone() {
                    let first = self.meta.frontmatter_links.len();
                    self.frontmatter_links("", &fm);
                    self.locate_frontmatter_links(first, block.span);
                }
            }
            BlockKind::Paragraph { content } => self.inlines(content),
            BlockKind::Heading { level, content, .. } => {
                let text = match (content.first(), content.last()) {
                    (Some(f), Some(l)) => self.src[f.span.start..l.span.end].trim().to_string(),
                    _ => String::new(),
                };
                self.meta.headings.push(HeadingCache {
                    heading: text,
                    level: *level,
                    position: self.lines.loc(block.span),
                });
                self.inlines(content);
            }
            BlockKind::BlockQuote { children } | BlockKind::FootnoteDefinition { children, .. } => {
                if let BlockKind::FootnoteDefinition { label, .. } = &block.kind {
                    self.meta.footnotes.push(FootnoteCache {
                        id: label.to_string(),
                        position: self.lines.loc(block.span),
                    });
                }
                self.blocks(children);
            }
            BlockKind::Callout {
                title, children, ..
            } => {
                // The title is a block tree; its inlines still carry the links
                // and tags a title can hold.
                for run in silica_markdown::inline_runs(title) {
                    self.inlines(run);
                }
                self.blocks(children);
            }
            BlockKind::List { items, .. } => self.list(items, None),
            BlockKind::Table { header, rows, .. } => {
                for cell in header.iter().chain(rows.iter().flatten()) {
                    self.inlines(&cell.content);
                }
            }
            BlockKind::ThematicBreak
            | BlockKind::CodeBlock { .. }
            | BlockKind::HtmlBlock { .. }
            | BlockKind::Math { .. }
            | BlockKind::Comment { .. } => {}
        }
    }

    fn list(&mut self, items: &[ListItem<'a>], parent_line: Option<u32>) {
        let first_line = items
            .first()
            .map(|i| self.lines.pos(i.span.start).line)
            .unwrap_or(0);
        for item in items {
            let position = self.lines.loc(item.span);
            let parent = match parent_line {
                Some(l) => l as i64,
                None => -(first_line as i64),
            };
            let id = item.children.first().and_then(|b| match &b.kind {
                BlockKind::Paragraph { .. } => b.id.map(|i| i.id.to_string()),
                _ => None,
            });
            if let Some(id) = &id {
                self.meta.blocks.insert(
                    id.clone(),
                    BlockCache {
                        id: id.clone(),
                        position,
                    },
                );
            }
            self.meta.list_items.push(ListItemCache {
                parent,
                task: item.task,
                id,
                position,
            });
            for child in &item.children {
                match &child.kind {
                    BlockKind::List { items, .. } => {
                        self.list(items, Some(position.start.line));
                    }
                    _ => self.block(child),
                }
            }
        }
    }

    fn inlines(&mut self, inlines: &[Inline<'a>]) {
        let src = self.src;
        let lines = &self.lines;
        let meta = &mut self.meta;
        walk_inlines(inlines, &mut |inline| match &inline.kind {
            InlineKind::Wikilink { body, embed } => {
                let parsed = silica_markdown::parse_linktext(body);
                let link = silica_markdown::link_target(body).to_string();
                let entry = LinkCache {
                    link,
                    original: inline.span.of(src).to_string(),
                    display_text: parsed.alias.map(str::to_string),
                    position: lines.loc(inline.span),
                };
                if *embed {
                    meta.embeds.push(entry);
                } else {
                    meta.links.push(entry);
                }
            }
            InlineKind::Link {
                destination,
                kind: silica_markdown::LinkKind::Inline | silica_markdown::LinkKind::Reference,
                children,
                ..
            } if is_vault_destination(destination) => {
                meta.links.push(LinkCache {
                    link: percent_decode(destination),
                    original: inline.span.of(src).to_string(),
                    display_text: Some(silica_markdown::plain_text(children)),
                    position: lines.loc(inline.span),
                });
            }
            InlineKind::Image {
                destination,
                children,
                ..
            } if is_vault_destination(destination) => {
                meta.embeds.push(LinkCache {
                    link: percent_decode(destination),
                    original: inline.span.of(src).to_string(),
                    display_text: Some(silica_markdown::plain_text(children)),
                    position: lines.loc(inline.span),
                });
            }
            InlineKind::Tag(name) => meta.tags.push(TagCache {
                tag: format!("#{name}"),
                position: lines.loc(inline.span),
            }),
            InlineKind::FootnoteReference(label) => meta.footnote_refs.push(FootnoteCache {
                id: label.to_string(),
                position: lines.loc(inline.span),
            }),
            _ => {}
        });
    }

    fn frontmatter_links(&mut self, key: &str, value: &serde_yaml::Value) {
        match value {
            serde_yaml::Value::String(s) => {
                let doc = silica_markdown::parse(s);
                let mut found = Vec::new();
                for block in &doc.blocks {
                    if let BlockKind::Paragraph { content } = &block.kind {
                        walk_inlines(content, &mut |inline| {
                            if let InlineKind::Wikilink { body, .. } = &inline.kind {
                                let parsed = silica_markdown::parse_linktext(body);
                                let link = silica_markdown::link_target(body).to_string();
                                found.push(FrontmatterLinkCache {
                                    key: key.to_string(),
                                    link,
                                    original: inline.span.of(s).to_string(),
                                    display_text: parsed.alias.map(str::to_string),
                                    position: Loc::default(),
                                });
                            }
                        });
                    }
                }
                self.meta.frontmatter_links.extend(found);
            }
            serde_yaml::Value::Sequence(items) => {
                for (i, item) in items.iter().enumerate() {
                    let k = if key.is_empty() {
                        i.to_string()
                    } else {
                        format!("{key}.{i}")
                    };
                    self.frontmatter_links(&k, item);
                }
            }
            serde_yaml::Value::Mapping(map) => {
                for (k, v) in map {
                    let name = match k {
                        serde_yaml::Value::String(s) => s.clone(),
                        other => serde_yaml::to_string(other)
                            .unwrap_or_default()
                            .trim()
                            .to_string(),
                    };
                    let k = if key.is_empty() {
                        name
                    } else {
                        format!("{key}.{name}")
                    };
                    self.frontmatter_links(&k, v);
                }
            }
            _ => {}
        }
    }

    /// Gives the links collected from the parsed properties their place in
    /// the file: `serde_yaml` hands back scalars without a source span, so
    /// the only way back is to find the written text again in the YAML, the
    /// search the link rewriter also makes. A value the YAML parser unescaped
    /// is not found again and keeps the frontmatter block's own position.
    fn locate_frontmatter_links(&mut self, first: usize, span: Span) {
        let yaml = span.of(self.src);
        let lines = &self.lines;
        let block = lines.loc(span);
        let mut seen: BTreeSet<usize> = BTreeSet::new();
        for link in self.meta.frontmatter_links.iter_mut().skip(first) {
            let found = yaml
                .match_indices(link.original.as_str())
                .map(|(at, _)| at)
                .find(|at| seen.insert(*at));
            link.position = match found {
                Some(at) => lines.loc(Span::new(
                    span.start + at,
                    span.start + at + link.original.len(),
                )),
                None => block,
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTE: &str = "---\ntags: [x]\naliases:\n  - Alt\nrelated: \"[[Other|shown]]\"\n---\n# Title\n\nSee [[Other#Part|there]] and ![[pic.png]] #tag ^blk1\n\n```\n[[not indexed]] #no\n```\n\n## Second\n\n- [ ] task [[In List]]\n  - nested\n- plain ^item\n\n[md](Some%20Note.md) and [ext](https://x.y) ![img](a.png)\n\nNote[^1].\n\n[^1]: Def\n";

    #[test]
    fn indexes_everything_outside_code() {
        let m = index_markdown(NOTE);
        assert_eq!(m.headings.len(), 2);
        assert_eq!(m.headings[0].heading, "Title");
        assert_eq!(m.headings[1].level, 2);
        let links: Vec<&str> = m.links.iter().map(|l| l.link.as_str()).collect();
        assert_eq!(links, vec!["Other#Part", "In List", "Some Note.md"]);
        assert_eq!(m.links[0].display_text.as_deref(), Some("there"));
        assert_eq!(m.links[0].original, "[[Other#Part|there]]");
        let embeds: Vec<&str> = m.embeds.iter().map(|l| l.link.as_str()).collect();
        assert_eq!(embeds, vec!["pic.png", "a.png"]);
        assert_eq!(m.tags.len(), 1);
        assert_eq!(m.tags[0].tag, "#tag");
        assert!(m.blocks.contains_key("blk1"));
        assert!(m.blocks.contains_key("item"));
        let fm = m.frontmatter.as_ref().unwrap();
        assert_eq!(fm["aliases"][0], "Alt");
        assert_eq!(m.frontmatter_links.len(), 1);
        assert_eq!(m.frontmatter_links[0].key, "related");
        assert_eq!(m.frontmatter_links[0].link, "Other");
        assert_eq!(
            m.frontmatter_links[0].display_text.as_deref(),
            Some("shown")
        );
        let at = m.frontmatter_links[0].position;
        assert_eq!(at.start.line, 4);
        assert_eq!(&NOTE[at.start.offset..at.end.offset], "[[Other|shown]]");
        assert_eq!(m.footnotes.len(), 1);
        assert_eq!(m.footnote_refs.len(), 1);
    }

    #[test]
    fn repeated_property_links_get_one_position_each() {
        let src = "---\nfirst: \"[[X]]\"\nsecond: \"[[X]]\"\n---\n";
        let m = index_markdown(src);
        let lines: Vec<u32> = m
            .frontmatter_links
            .iter()
            .map(|l| l.position.start.line)
            .collect();
        assert_eq!(lines, vec![1, 2]);
    }

    #[test]
    fn sections_and_list_items() {
        let m = index_markdown(NOTE);
        let kinds: Vec<&str> = m.sections.iter().map(|s| s.kind.as_str()).collect();
        assert_eq!(
            kinds,
            vec![
                "yaml",
                "heading",
                "paragraph",
                "code",
                "heading",
                "list",
                "paragraph",
                "paragraph",
                "footnoteDefinition"
            ]
        );
        assert_eq!(m.sections[2].id.as_deref(), Some("blk1"));
        assert_eq!(m.frontmatter_position.unwrap().end.line, 5);
        let items: Vec<(i64, Option<char>, Option<&str>)> = m
            .list_items
            .iter()
            .map(|i| (i.parent, i.task, i.id.as_deref()))
            .collect();
        // The list starts on line 16; the nested item's parent is line 16.
        assert_eq!(
            items,
            vec![
                (-16, Some(' '), None),
                (16, None, None),
                (-16, None, Some("item"))
            ]
        );
        assert_eq!(m.blocks["item"].position.start.line, 18);
    }

    #[test]
    fn positions_are_line_and_column_accurate() {
        let m = index_markdown("a\nbb [[c]]\n");
        let p = m.links[0].position;
        assert_eq!((p.start.line, p.start.col), (1, 3));
        assert_eq!((p.end.line, p.end.col), (1, 8));
    }
}
