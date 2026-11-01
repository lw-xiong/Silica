//! Reading-view model: an owned render tree built from the shared parser's
//! [`Document`], so the view can hold it across frames without borrowing the
//! note text. Mirrors what `silica_markdown::render_html` emits (the same
//! tree, the same footnote numbering, the same embed rules) but as data a
//! GPUI element tree can be built from: paragraphs become runs of styled
//! text with clickable ranges, embeds are resolved and inlined recursively,
//! and footnote definitions are gathered at the end.

use silica_formats::{
    properties::Property, FileKind, AUDIO_EXTENSIONS, VIDEO_EXTENSIONS,
};
use silica_markdown::{
    canonical_callout_type, html::has_scheme, html::percent_decode, parse, plain_text, Alignment,
    Block, BlockKind, Document, Fold, HighlightColor, Inline, InlineKind, LinkKind, ListItem,
    ListKind, Subpath,
};
use std::cell::RefCell;
use std::ops::Range;
use std::path::PathBuf;

/// Embedded notes nest at most this deep; Obsidian stops at a similar depth.
pub const MAX_EMBED_DEPTH: usize = 4;

/// The bases resolved while building one model, so none is resolved twice.
///
/// Resolving a `.base` runs a view over the **whole vault** — every note of it
/// becomes a row candidate — where every other embed reads one file, so a note
/// that embeds the same base twice, or embeds it again inside an embedded
/// note, must not pay for it twice. Keyed case-insensitively, the way every
/// other vault path comparison is. A miss is remembered too: a base that
/// answered nothing is not asked again either.
///
/// This is a cost guard, not a recursion guard: a base's rows are notes, and a
/// note inside a base is a row rather than an embed, so resolving one never
/// builds another model and nothing can recurse through it.
#[derive(Default)]
struct BaseCache {
    seen: RefCell<Vec<(String, Option<Box<crate::bases::EmbeddedBase>>)>>,
}

impl BaseCache {
    fn resolve(
        &self,
        path: &str,
        vault: &dyn EmbedSource,
    ) -> Option<Box<crate::bases::EmbeddedBase>> {
        // The borrow ends before the resolve: nothing is held across it.
        if let Some(hit) = self
            .seen
            .borrow()
            .iter()
            .find(|(p, _)| p.eq_ignore_ascii_case(path))
            .map(|(_, base)| base.clone())
        {
            return hit;
        }
        let resolved = vault.base(path).map(Box::new);
        self.seen
            .borrow_mut()
            .push((path.to_string(), resolved.clone()));
        resolved
    }
}

/// What the model builder needs from the vault to resolve embeds.
pub trait EmbedSource {
    /// Resolves a link body written in `source_path` to a vault path.
    fn resolve(&self, linktext: &str, source_path: &str) -> Option<String>;
    /// Reads a markdown file's text.
    fn read(&self, path: &str) -> Option<String>;
    /// Absolute path of a vault file, for images.
    fn absolute(&self, path: &str) -> PathBuf;
    /// An embedded `![[x.base]]`, resolved on its first view.
    ///
    /// A base is a view over the whole vault rather than over one file, so
    /// this needs the index and the cache and not just the bytes — which is
    /// why it is its own method rather than a `read` the builder parses. The
    /// default answers nothing, which is what a source with no vault behind
    /// it (a test's stub) has to answer.
    fn base(&self, _path: &str) -> Option<crate::bases::EmbeddedBase> {
        None
    }
}

/// Formatting that applies to a run of text; several may apply at once.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct InlineStyle {
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub highlight: bool,
    /// One of the six colours a `==🔴word==` highlight names, or none for
    /// the plain `==word==`.
    pub highlight_color: Option<HighlightColor>,
    pub code: bool,
    pub link: bool,
    pub tag: bool,
    pub math: bool,
    pub footnote: bool,
    pub html: bool,
}

impl InlineStyle {
    pub fn is_plain(&self) -> bool {
        *self == InlineStyle::default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkTarget {
    /// A wikilink body or a vault-relative markdown link destination,
    /// resolved by the workspace relative to the note it appears in.
    Internal(String),
    External(String),
    Tag(String),
    /// Jump to footnote number `n` at the bottom of the note.
    Footnote(usize),
}

/// One line-wrapped run of inline content.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RichText {
    /// Unique within one model, for GPUI element ids.
    pub id: usize,
    pub text: String,
    /// Non-overlapping styled byte ranges in order; gaps are plain.
    pub runs: Vec<(Range<usize>, InlineStyle)>,
    pub links: Vec<(Range<usize>, LinkTarget)>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Segment {
    Text(RichText),
    Embed(Embed),
    /// `$$x$$` written inside a paragraph: display math, drawn as its own
    /// centred block the way MathJax's `display="true"` container is.
    Math(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Embed {
    pub kind: EmbedKind,
    /// The link body as written, for opening the target.
    pub link: String,
    pub alt: String,
    pub width: Option<f32>,
    pub height: Option<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EmbedKind {
    Image(PathBuf),
    RemoteImage(String),
    /// A local audio file, resolved to its vault-relative path.
    Audio(PathBuf),
    /// A local video file, resolved to its vault-relative path.
    Video(PathBuf),
    /// A remote audio file URL.
    RemoteAudio(String),
    /// A remote video file URL.
    RemoteVideo(String),
    Note {
        path: String,
        /// `Note`, `Note > Heading`, or `Note > ^id`.
        title: String,
        blocks: Vec<RBlock>,
    },
    /// A `.base`, resolved on its first view. Boxed because the resolved
    /// table is far larger than any other embed's payload and every `RBlock`
    /// would otherwise carry the room for one.
    Base(Box<crate::bases::EmbeddedBase>),
    /// A `.pdf`: the path and the subpath, nothing read. The pages come
    /// from the document cache when the embed is drawn (`pdf::embed`).
    Pdf(crate::pdf::embed::PdfEmbed),
    /// The target file does not exist.
    Missing,
    /// The target exists but Silica cannot render it inline yet.
    Unsupported(FileKind),
    TooDeep,
}

/// Classifies a remote URL embed by its file extension: images stay images,
/// audio and video become players, everything else falls back to image.
fn remote_media_kind(url: &str) -> EmbedKind {
    let path = url.split('?').next().unwrap_or(url);
    let ext = path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    match ext.as_deref() {
        Some(e) if AUDIO_EXTENSIONS.contains(&e) => EmbedKind::RemoteAudio(url.to_string()),
        Some(e) if VIDEO_EXTENSIONS.contains(&e) => EmbedKind::RemoteVideo(url.to_string()),
        _ => EmbedKind::RemoteImage(url.to_string()),
    }
}

/// Calls `f` on every embed in `blocks`, in document order, descending into
/// embedded notes, quotes, callouts, lists and footnotes: the walk the
/// control socket's dumps and the canvas's dependency lists share.
pub fn each_embed<'a>(blocks: &'a [RBlock], f: &mut dyn FnMut(&'a Embed)) {
    for block in blocks {
        match block {
            RBlock::Paragraph(segments) => {
                for segment in segments {
                    if let Segment::Embed(embed) = segment {
                        f(embed);
                        if let EmbedKind::Note { blocks, .. } = &embed.kind {
                            each_embed(blocks, f);
                        }
                    }
                }
            }
            RBlock::Quote(children) | RBlock::Callout { children, .. } => each_embed(children, f),
            RBlock::List { items, .. } => {
                for item in items {
                    each_embed(&item.children, f);
                }
            }
            RBlock::Footnotes(notes) => {
                for (_, body) in notes {
                    each_embed(body, f);
                }
            }
            _ => {}
        }
    }
}

/// Where a task's `[ ]` marker sits in the note: a zero-based line and the
/// byte column inside it. The editor's buffer drops a BOM and rewrites
/// `\r\n`, so a byte offset into the file is not one of its offsets, but
/// every line of the file is still a line of the buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskMarker {
    pub line: usize,
    pub col: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RItem {
    pub task: Option<char>,
    /// Where the three bytes `[`, status, `]` are, for a task of the note
    /// itself. An item that came out of an embed has none: its bytes
    /// belong to another file, so clicking it must not write this one.
    pub marker: Option<TaskMarker>,
    pub number: Option<u64>,
    pub children: Vec<RBlock>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RBlock {
    /// The properties block. The entries are the parsed frontmatter with
    /// their byte spans; the renderer resolves each key's type against the
    /// vault's `types.json` and builds the rows, because the same block is
    /// drawn read-only in the reading view and editable in Live Preview.
    Properties(Vec<Property>),
    Paragraph(Vec<Segment>),
    Heading {
        level: u8,
        text: RichText,
        plain: String,
    },
    ThematicBreak,
    Code {
        /// Unique within one model, for GPUI element ids.
        id: usize,
        language: Option<String>,
        text: String,
        /// What the copy button writes: the body with `\n` endings and
        /// exactly one trailing newline removed, as Obsidian's copies.
        copy: String,
    },
    Html(String),
    Quote(Vec<RBlock>),
    Callout {
        /// Canonical type, one of Obsidian's built-ins.
        kind: &'static str,
        fold: Option<bool>,
        title: RichText,
        children: Vec<RBlock>,
    },
    List {
        tight: bool,
        items: Vec<RItem>,
    },
    Table {
        alignments: Vec<Alignment>,
        header: Vec<RichText>,
        rows: Vec<Vec<RichText>>,
        /// The bytes each cell stands for, the header row first, so Live
        /// Preview can edit a cell in place.
        spans: Vec<Vec<Range<usize>>>,
    },
    Math(String),
    Mermaid {
        id: usize,
        text: String,
    },
    /// `(number, body)` in reference order, rendered after a rule.
    Footnotes(Vec<(usize, Vec<RBlock>)>),
}

/// A built note.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Model {
    pub blocks: Vec<RBlock>,
    /// Source byte offset where each top-level block starts, for mapping
    /// cache positions (outline, backlinks) to list items. One entry per
    /// block in `blocks`, built from the same list: a comment or a
    /// footnote definition is dropped from both or from neither, or every
    /// jump past one lands on the wrong block.
    pub starts: Vec<usize>,
    /// Source byte offset of footnote `n`'s definition, `n - 1` indexed, in
    /// the order the Footnotes section lists them. A footnote written
    /// inline has no definition of its own and carries the offset of the
    /// reference instead.
    pub footnotes: Vec<usize>,
}

impl Model {
    /// Index of the top-level block containing byte `offset`.
    pub fn block_at(&self, offset: usize) -> Option<usize> {
        if self.starts.is_empty() {
            return None;
        }
        Some(match self.starts.binary_search(&offset) {
            Ok(i) => i,
            Err(0) => 0,
            Err(i) => i - 1,
        })
    }
}

/// Rendering settings that come from `app.json`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BuildOptions {
    /// Obsidian's "Strict line breaks": a single newline inside a paragraph
    /// is a space rather than a line break.
    pub strict_line_breaks: bool,
}

/// Builds the model for `text`, the content of `source_path`, with the default settings.
#[cfg(test)]
pub fn build(source_path: &str, text: &str, vault: &dyn EmbedSource) -> Model {
    build_with(source_path, text, vault, BuildOptions::default())
}

/// Builds the model for `text`, the content of `source_path`.
pub fn build_with(
    source_path: &str,
    text: &str,
    vault: &dyn EmbedSource,
    opts: BuildOptions,
) -> Model {
    let doc = parse(text);
    let bases = BaseCache::default();
    let mut b = Builder {
        vault,
        source_path,
        text: doc.source,
        opts,
        depth: 0,
        next_id: 0,
        footnotes: FootnoteState::default(),
        line_starts: None,
        bases: &bases,
    };
    b.footnotes.definitions = collect_definitions(&doc.blocks);
    let (mut blocks, mut starts) = b.blocks_with_starts(&doc.blocks);
    let mut footnotes = Vec::new();
    if let Some((section, offsets)) = b.footnote_section(&doc) {
        // The appended section needs a start of its own, or an offset
        // inside a definition resolves past the end of the list.
        starts.push(offsets.iter().copied().min().unwrap_or(doc.source.len()));
        blocks.push(section);
        footnotes = offsets;
    }
    Model {
        blocks,
        starts,
        footnotes,
    }
}

/// Builds one part of `text`, the content of `source_path`: the section
/// under a heading path (`#A#B`, from the heading to the next of the same
/// or a higher level) or the one block carrying an id (`#^id`). `None`
/// when the note has no such heading or block. A canvas file card with a
/// `subpath` shows exactly this, so unlike an embedded section it is built
/// at depth 0: its task markers are positions in the note itself and its
/// embeds nest as deep as the note's own would.
pub(crate) fn build_section(
    source_path: &str,
    text: &str,
    subpath: &Subpath<'_>,
    vault: &dyn EmbedSource,
    opts: BuildOptions,
) -> Option<Model> {
    let doc = parse(text);
    let selected: Vec<Block<'_>> = match subpath {
        Subpath::Headings(names) => section(&doc.blocks, names)?.to_vec(),
        Subpath::Block(id) => vec![block_by_id(&doc.blocks, id)?.clone()],
    };
    let bases = BaseCache::default();
    let mut b = Builder {
        vault,
        source_path,
        text: doc.source,
        opts,
        depth: 0,
        next_id: 0,
        footnotes: FootnoteState::default(),
        line_starts: None,
        bases: &bases,
    };
    b.footnotes.definitions = collect_definitions(&doc.blocks);
    let (mut blocks, mut starts) = b.blocks_with_starts(&selected);
    let mut footnotes = Vec::new();
    if let Some((section, offsets)) = b.footnote_section(&doc) {
        starts.push(offsets.iter().copied().min().unwrap_or(doc.source.len()));
        blocks.push(section);
        footnotes = offsets;
    }
    Some(Model {
        blocks,
        starts,
        footnotes,
    })
}

/// A top-level block Live Preview renders in place of its source lines:
/// a table, a math block, or a paragraph that is one embed.
#[derive(Debug, Clone, PartialEq)]
pub struct Widget {
    /// Byte span of the block in the note (newline after it excluded).
    pub span: Range<usize>,
    pub block: RBlock,
}

/// The blocks of `doc` (the content of `source_path`) that Live Preview
/// shows as widgets, built with ids unique among them.
pub fn build_widgets(
    source_path: &str,
    doc: &Document<'_>,
    vault: &dyn EmbedSource,
    opts: BuildOptions,
) -> Vec<Widget> {
    let bases = BaseCache::default();
    let mut b = Builder {
        vault,
        source_path,
        text: doc.source,
        opts,
        depth: 0,
        next_id: 0,
        footnotes: FootnoteState::default(),
        line_starts: None,
        bases: &bases,
    };
    b.footnotes.definitions = collect_definitions(&doc.blocks);
    let mut out = Vec::new();
    for block in &doc.blocks {
        let widget = match &block.kind {
            // The properties block is a widget so Live Preview can draw it as
            // an editable table instead of raw YAML.
            BlockKind::Yaml { .. } => true,
            BlockKind::Table { .. } | BlockKind::Math { .. } => true,
            BlockKind::CodeBlock { info, .. } => {
                info.as_deref().and_then(|i| i.split_whitespace().next()) == Some("mermaid")
            }
            // A callout is drawn as the same tinted block the reading view
            // draws; a cursor inside it still brings its source lines back.
            BlockKind::Callout { .. } => true,
            // Only the raw HTML the renderer can actually draw earns a
            // widget. Anything else -- a comment, a script, a bare `<div>` --
            // would fold its source lines away to show the same muted text.
            BlockKind::HtmlBlock { content } => html_images(content).is_some(),
            BlockKind::Paragraph { content } => match content.as_slice() {
                [one] => matches!(
                    &one.kind,
                    InlineKind::Wikilink { embed: true, .. }
                        | InlineKind::Image {
                            kind: LinkKind::Inline,
                            ..
                        }
                ),
                _ => false,
            },
            _ => false,
        };
        if !widget {
            continue;
        }
        if let Some(built) = b.block(block, false) {
            out.push(Widget {
                span: block.span.start..block.span.end,
                block: built,
            });
        }
    }
    out
}

/// Builds the model of an embedded note or section with the given nesting.
///
/// Eight arguments, because an embed is built from the whole nesting it sits
/// in: the file it came from, its blocks, how deep it is, the id counter and
/// the base cache it shares with every sibling, the vault, and the options.
/// Bundling them into a struct would only move the same list one line down.
#[allow(clippy::too_many_arguments)]
fn build_embedded(
    source_path: &str,
    doc: &Document<'_>,
    blocks: &[Block<'_>],
    depth: usize,
    next_id: &mut usize,
    bases: &BaseCache,
    vault: &dyn EmbedSource,
    opts: BuildOptions,
) -> Vec<RBlock> {
    let mut b = Builder {
        vault,
        source_path,
        text: doc.source,
        opts,
        depth,
        next_id: *next_id,
        footnotes: FootnoteState::default(),
        line_starts: None,
        bases,
    };
    b.footnotes.definitions = collect_definitions(&doc.blocks);
    let mut out = b.blocks(blocks, false);
    // An embedded section is not jumped into, so its footnote offsets are
    // of no use to anyone.
    if let Some((section, _)) = b.footnote_section(doc) {
        out.push(section);
    }
    *next_id = b.next_id;
    out
}

#[derive(Default)]
struct FootnoteState {
    /// Labels present in the document (first definition wins).
    definitions: Vec<String>,
    /// Labels in order of first reference; index + 1 is the number.
    numbers: Vec<String>,
    /// Inline footnotes keyed by generated label.
    inline: Vec<(String, RichText)>,
}

fn collect_definitions(blocks: &[Block<'_>]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    silica_markdown::walk_blocks(blocks, &mut |b| {
        if let BlockKind::FootnoteDefinition { label, .. } = &b.kind {
            if !out.iter().any(|l| l == label) {
                out.push(label.to_string());
            }
        }
    });
    out
}

struct Builder<'v> {
    vault: &'v dyn EmbedSource,
    /// Shared with every nested builder of the same model, so an embedded
    /// base is resolved once however many times the note names it.
    bases: &'v BaseCache,
    source_path: &'v str,
    /// The whole note, so the properties block can carry file byte spans.
    text: &'v str,
    opts: BuildOptions,
    depth: usize,
    next_id: usize,
    footnotes: FootnoteState,
    /// Where each line of `text` starts, built the first time a task
    /// marker asks for its position.
    line_starts: Option<Vec<usize>>,
}

impl<'v> Builder<'v> {
    /// The line and column of a byte offset in the note.
    fn marker_at(&mut self, offset: usize) -> TaskMarker {
        let text = self.text;
        let starts = self.line_starts.get_or_insert_with(|| {
            std::iter::once(0)
                .chain(text.match_indices('\n').map(|(i, _)| i + 1))
                .collect()
        });
        let line = match starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        };
        TaskMarker {
            line,
            col: offset - starts[line],
        }
    }

    fn id(&mut self) -> usize {
        self.next_id += 1;
        self.next_id
    }

    fn blocks(&mut self, blocks: &[Block<'_>], tight: bool) -> Vec<RBlock> {
        blocks.iter().filter_map(|b| self.block(b, tight)).collect()
    }

    /// The top level, where each block kept has to keep its source offset
    /// too: `block` returns `None` for a comment and for a footnote
    /// definition, so deriving the offsets from the *parsed* list instead
    /// would leave the two lists one apart for every block dropped.
    fn blocks_with_starts(&mut self, blocks: &[Block<'_>]) -> (Vec<RBlock>, Vec<usize>) {
        let mut out = Vec::new();
        let mut starts = Vec::new();
        for block in blocks {
            if let Some(built) = self.block(block, false) {
                out.push(built);
                starts.push(block.span.start);
            }
        }
        (out, starts)
    }

    fn block(&mut self, block: &Block<'_>, _tight: bool) -> Option<RBlock> {
        Some(match &block.kind {
            BlockKind::Yaml { .. } => RBlock::Properties(
                silica_cache::note_properties(self.text)
                    .map(|p| p.entries)
                    .unwrap_or_default(),
            ),
            BlockKind::Paragraph { content } => RBlock::Paragraph(self.segments(content)),
            BlockKind::Heading { level, content, .. } => RBlock::Heading {
                level: *level,
                text: self.rich(content),
                plain: plain_text(content).trim().to_string(),
            },
            BlockKind::ThematicBreak => RBlock::ThematicBreak,
            BlockKind::CodeBlock { info, content, .. } => {
                let lang = info
                    .as_deref()
                    .and_then(|i| i.split_whitespace().next());
                if lang == Some("mermaid") {
                    return Some(RBlock::Mermaid {
                        id: self.id(),
                        text: content.trim_end_matches('\n').to_string(),
                    });
                }
                // Obsidian renders and copies a code block with `\n`
                // endings whatever the file holds, and its copy button
                // drops exactly one trailing newline rather than trimming.
                let body = if content.contains('\r') {
                    content.replace("\r\n", "\n")
                } else {
                    content.to_string()
                };
                RBlock::Code {
                    id: self.id(),
                    language: info
                        .as_deref()
                        .and_then(|i| i.split_whitespace().next())
                        .map(str::to_string),
                    text: body.trim_end_matches('\n').to_string(),
                    copy: body.strip_suffix('\n').unwrap_or(&body).to_string(),
                }
            }
            BlockKind::HtmlBlock { content } => match html_images(content) {
                // Obsidian resolves a raw `<img src>` like a wikilink target
                // and renders the run of them as one block, so they go
                // through the same embeds a markdown image does.
                Some(imgs) => {
                    let mut segments = Vec::with_capacity(imgs.len());
                    for img in imgs {
                        let embed = self.embed(&img.src, img.alt, img.width, img.height);
                        segments.push(Segment::Embed(embed));
                    }
                    RBlock::Paragraph(segments)
                }
                None => RBlock::Html(content.trim_end().to_string()),
            },
            BlockKind::BlockQuote { children } => RBlock::Quote(self.blocks(children, false)),
            BlockKind::Callout {
                callout_type,
                fold,
                title,
                children,
            } => {
                let title = if title.is_empty() {
                    // Shared with the reading view's HTML and with Live
                    // Preview: this was a fourth copy of the derivation, and
                    // the only one that still left `-` in place, so a
                    // `[!bad-type]` callout read "Bad-type" here.
                    RichText {
                        id: self.id(),
                        text: silica_markdown::callout_default_title(callout_type),
                        ..RichText::default()
                    }
                } else {
                    // The title is block-parsed; this view paints one run of
                    // rich text, so flatten the tree's inlines into it. A
                    // heading or a list written as a title therefore shows its
                    // text without the block styling Obsidian gives it -- see
                    // the callout block in docs/silica/Specification.md.
                    let flat: Vec<Inline<'_>> = silica_markdown::inline_runs(title)
                        .into_iter()
                        .flat_map(|run| run.iter().cloned())
                        .collect();
                    self.rich(&flat)
                };
                RBlock::Callout {
                    kind: canonical_callout_type(callout_type),
                    fold: fold.map(|f| f == Fold::Collapsed),
                    title,
                    children: self.blocks(children, false),
                }
            }
            BlockKind::List { kind, tight, items } => RBlock::List {
                tight: *tight,
                items: self.items(kind, items, *tight),
            },
            BlockKind::Table {
                alignments,
                header,
                rows,
            } => RBlock::Table {
                alignments: alignments.clone(),
                header: header.iter().map(|c| self.rich(&c.content)).collect(),
                rows: rows
                    .iter()
                    .map(|r| r.iter().map(|c| self.rich(&c.content)).collect())
                    .collect(),
                spans: std::iter::once(header)
                    .chain(rows.iter())
                    .map(|r| r.iter().map(|c| c.span.start..c.span.end).collect())
                    .collect(),
            },
            BlockKind::Math { content } => RBlock::Math(content.trim_matches('\n').to_string()),
            BlockKind::Comment { .. } | BlockKind::FootnoteDefinition { .. } => return None,
        })
    }

    fn items(&mut self, kind: &ListKind, items: &[ListItem<'_>], tight: bool) -> Vec<RItem> {
        let mut number = match kind {
            ListKind::Ordered { start, .. } => Some(*start),
            ListKind::Bullet { .. } => None,
        };
        items
            .iter()
            .map(|item| {
                let n = number;
                if let Some(n) = number.as_mut() {
                    *n += 1;
                }
                let marker = item
                    .task_span
                    .filter(|_| self.depth == 0)
                    .map(|s| self.marker_at(s.start));
                RItem {
                    task: item.task,
                    marker,
                    number: n,
                    children: self.blocks(&item.children, tight),
                }
            })
            .collect()
    }

    /// A paragraph's content split into text runs and block-level embeds.
    fn segments(&mut self, content: &[Inline<'_>]) -> Vec<Segment> {
        let mut out = Vec::new();
        let mut acc = RichBuilder::default();
        let is_display_math =
            |inline: &Inline<'_>| matches!(inline.kind, InlineKind::Math { display: true, .. });
        for (ix, inline) in content.iter().enumerate() {
            match &inline.kind {
                // The space on either side of a display block inside a
                // paragraph is the block's margin, not text to set: an HTML
                // renderer collapses it at the block's edge.
                InlineKind::Text(t)
                    if ix
                        .checked_sub(1)
                        .is_some_and(|p| is_display_math(&content[p]))
                        || content.get(ix + 1).is_some_and(is_display_math) =>
                {
                    let mut t: &str = t;
                    if ix
                        .checked_sub(1)
                        .is_some_and(|p| is_display_math(&content[p]))
                    {
                        t = t.trim_start();
                    }
                    if content.get(ix + 1).is_some_and(is_display_math) {
                        t = t.trim_end();
                    }
                    acc.push(t);
                }
                InlineKind::Wikilink { body, embed: true } => {
                    self.flush(&mut acc, &mut out);
                    let embed = self.wikilink_embed(body);
                    out.push(Segment::Embed(embed));
                }
                InlineKind::Image {
                    destination,
                    kind: LinkKind::Inline,
                    children,
                    ..
                } if destination.starts_with("http://") || destination.starts_with("https://") => {
                    self.flush(&mut acc, &mut out);
                    let alt_text = plain_text(children);
                    let mut width = None;
                    let mut height = None;
                    let mut alt = None;
                    for part in alt_text.split('|').map(str::trim) {
                        match size_hint(part) {
                            Some((w, h)) => {
                                width = Some(w);
                                height = h;
                            }
                            None if alt.is_none() && !part.is_empty() => alt = Some(part.to_string()),
                            None => {}
                        }
                    }
                    let alt = alt.unwrap_or_default();
                    out.push(Segment::Embed(Embed {
                        kind: remote_media_kind(destination),
                        link: destination.to_string(),
                        alt,
                        width,
                        height,
                    }));
                }
                InlineKind::Image {
                    destination,
                    kind: LinkKind::Inline,
                    children,
                    ..
                } if !has_scheme(destination) => {
                    self.flush(&mut acc, &mut out);
                    let alt_text = plain_text(children);
                    let mut width = None;
                    let mut height = None;
                    let mut alt = None;
                    for part in alt_text.split('|').map(str::trim) {
                        match size_hint(part) {
                            Some((w, h)) => {
                                width = Some(w);
                                height = h;
                            }
                            None if alt.is_none() && !part.is_empty() => alt = Some(part.to_string()),
                            None => {}
                        }
                    }
                    let alt = alt.unwrap_or_default();
                    let dest = percent_decode(destination).into_owned();
                    let embed = self.embed(&dest, alt, width, height);
                    out.push(Segment::Embed(embed));
                }
                InlineKind::Math {
                    content,
                    display: true,
                } => {
                    self.flush(&mut acc, &mut out);
                    out.push(Segment::Math(content.to_string()));
                }
                _ => self.inline(inline, &mut acc),
            }
        }
        self.flush(&mut acc, &mut out);
        out
    }

    fn flush(&mut self, acc: &mut RichBuilder, out: &mut Vec<Segment>) {
        let text = std::mem::take(acc).finish(self.id());
        // Whitespace between two embeds would render as an empty line.
        if !text.text.trim().is_empty() {
            out.push(Segment::Text(text));
        }
    }

    fn rich(&mut self, content: &[Inline<'_>]) -> RichText {
        let mut acc = RichBuilder::default();
        for inline in content {
            self.inline(inline, &mut acc);
        }
        acc.finish(self.id())
    }

    fn inline(&mut self, inline: &Inline<'_>, acc: &mut RichBuilder) {
        match &inline.kind {
            InlineKind::Text(t) => acc.push(t),
            InlineKind::SoftBreak if self.opts.strict_line_breaks => acc.push(" "),
            InlineKind::SoftBreak | InlineKind::HardBreak => acc.push("\n"),
            InlineKind::Code(c) => acc.styled(|s| s.code = true, |a| a.push(c)),
            InlineKind::Html(h) => acc.styled(|s| s.html = true, |a| a.push(h)),
            InlineKind::Emphasis(c) => self.wrap(acc, c, |s| s.italic = true),
            InlineKind::Strong(c) => self.wrap(acc, c, |s| s.bold = true),
            InlineKind::Strikethrough(c) => self.wrap(acc, c, |s| s.strike = true),
            InlineKind::Highlight { color, children } => {
                let saved = acc.style;
                acc.style.highlight = true;
                acc.style.highlight_color = *color;
                for c in children {
                    self.inline(c, acc);
                }
                acc.style = saved;
            }
            InlineKind::Link {
                destination,
                kind,
                children,
                ..
            } => {
                let internal = matches!(kind, LinkKind::Inline | LinkKind::Reference)
                    && silica_markdown::html::is_vault_destination(destination);
                let target = if internal {
                    LinkTarget::Internal(percent_decode(destination).into_owned())
                } else {
                    LinkTarget::External(destination.to_string())
                };
                let start = acc.len();
                if *kind == LinkKind::Autolink {
                    let shown = plain_text(children);
                    let shown = shown.strip_prefix("mailto:").unwrap_or(&shown).to_string();
                    acc.styled(|s| s.link = true, |a| a.push(&shown));
                    acc.link(start, target);
                } else {
                    // The same rule the HTML renderer follows: Obsidian ends a
                    // link at the first child that is a link of its own, and
                    // what comes after that child is not part of it. Without
                    // this the trailing text of `[md [[Links]] link](url)`
                    // would still be clickable through to the outer link.
                    let split = children
                        .iter()
                        .position(silica_markdown::html::renders_as_anchor);
                    let head = split.unwrap_or(children.len());
                    self.wrap(acc, &children[..head], |s| s.link = true);
                    acc.link(start, target);
                    if let Some(at) = split {
                        for child in &children[at..] {
                            self.inline(child, acc);
                        }
                    }
                }
            }
            InlineKind::Image {
                destination,
                children,
                ..
            } => {
                // External or reference-style images: shown as a link until
                // remote images load.
                let start = acc.len();
                let alt = plain_text(children);
                let shown = if alt.is_empty() {
                    destination.to_string()
                } else {
                    alt
                };
                acc.styled(|s| s.link = true, |a| a.push(&shown));
                acc.link(start, LinkTarget::External(destination.to_string()));
            }
            InlineKind::Wikilink { body, embed } => {
                let link = silica_markdown::parse_linktext(body);
                let target = silica_markdown::link_target(body).to_string();
                let shown = match link.alias {
                    Some(a) => a.to_string(),
                    None => wikilink_display(&link),
                };
                let start = acc.len();
                if *embed {
                    acc.push("![[");
                    acc.push(&shown);
                    acc.push("]]");
                } else {
                    acc.styled(|s| s.link = true, |a| a.push(&shown));
                    acc.link(start, LinkTarget::Internal(target));
                }
            }
            InlineKind::Tag(name) => {
                let start = acc.len();
                acc.styled(
                    |s| s.tag = true,
                    |a| {
                        a.push("#");
                        a.push(name);
                    },
                );
                acc.link(start, LinkTarget::Tag(name.to_string()));
            }
            InlineKind::Math { content, .. } => {
                acc.styled(|s| s.math = true, |a| a.push(content));
            }
            InlineKind::Comment(_) => {}
            InlineKind::FootnoteReference(label) => {
                if self
                    .footnotes
                    .definitions
                    .iter()
                    .any(|l| l == label.as_ref())
                {
                    let n = self.footnote_number(label);
                    self.footnote_ref(acc, n);
                } else {
                    acc.push(label);
                }
            }
            InlineKind::InlineFootnote(children) => {
                let label = format!("inline-{}", self.footnotes.inline.len() + 1);
                let body = self.rich(children);
                self.footnotes.inline.push((label.clone(), body));
                let n = self.footnote_number(&label);
                self.footnote_ref(acc, n);
            }
        }
    }

    fn wrap(&mut self, acc: &mut RichBuilder, children: &[Inline<'_>], f: fn(&mut InlineStyle)) {
        let saved = acc.style;
        f(&mut acc.style);
        for c in children {
            self.inline(c, acc);
        }
        acc.style = saved;
    }

    fn footnote_number(&mut self, label: &str) -> usize {
        if let Some(i) = self.footnotes.numbers.iter().position(|l| l == label) {
            return i + 1;
        }
        self.footnotes.numbers.push(label.to_string());
        self.footnotes.numbers.len()
    }

    fn footnote_ref(&mut self, acc: &mut RichBuilder, n: usize) {
        let start = acc.len();
        acc.styled(|s| s.footnote = true, |a| a.push(&format!("[{n}]")));
        acc.link(start, LinkTarget::Footnote(n));
    }

    /// The Footnotes section, and where in the note each definition is
    /// written. A click on a reference reveals that offset, which is the
    /// definition's line in an editing mode and the Footnotes block in
    /// reading.
    fn footnote_section(&mut self, doc: &Document<'_>) -> Option<(RBlock, Vec<usize>)> {
        if self.footnotes.numbers.is_empty() {
            return None;
        }
        let labels = self.footnotes.numbers.clone();
        let mut out = Vec::new();
        let mut offsets = Vec::new();
        for (i, label) in labels.iter().enumerate() {
            let n = i + 1;
            let mut at = None;
            let body =
                if let Some((_, text)) = self.footnotes.inline.iter().find(|(l, _)| l == label) {
                    vec![RBlock::Paragraph(vec![Segment::Text(text.clone())])]
                } else {
                    let mut found = None;
                    silica_markdown::walk_blocks(&doc.blocks, &mut |b| {
                        if found.is_none() {
                            if let BlockKind::FootnoteDefinition { label: l, children } = &b.kind {
                                if l == label {
                                    found = Some(children);
                                    at = Some(b.span.start);
                                }
                            }
                        }
                    });
                    match found {
                        Some(children) => self.blocks(children, false),
                        None => Vec::new(),
                    }
                };
            // An inline footnote is written where it is referenced, and a
            // label nothing defines has nowhere else to point either.
            offsets.push(at.unwrap_or(0));
            out.push((n, body));
        }
        Some((RBlock::Footnotes(out), offsets))
    }

    // ----- embeds --------------------------------------------------------------

    fn wikilink_embed(&mut self, body: &str) -> Embed {
        let link = silica_markdown::parse_linktext(body);
        let target = silica_markdown::link_target(body);
        let mut width = None;
        let mut height = None;
        let mut alt = None;
        if let Some(a) = link.alias {
            for part in a.split('|').map(str::trim) {
                match size_hint(part) {
                    Some((w, h)) => {
                        width = Some(w);
                        height = h;
                    }
                    None if alt.is_none() && !part.is_empty() => alt = Some(part.to_string()),
                    None => {}
                }
            }
        }
        let alt = alt.unwrap_or_else(|| wikilink_display(&link));
        if target.starts_with("http://") || target.starts_with("https://") {
            return Embed {
                kind: remote_media_kind(target),
                link: target.to_string(),
                alt,
                width,
                height,
            };
        }
        self.embed(target, alt, width, height)
    }

    fn embed(&mut self, link: &str, alt: String, width: Option<f32>, height: Option<f32>) -> Embed {
        let parsed = silica_markdown::parse_linktext(link);
        let resolved = self.vault.resolve(link, self.source_path);
        let kind = match resolved {
            None => EmbedKind::Missing,
            Some(path) => match FileKind::from_path(&path) {
                FileKind::Image => EmbedKind::Image(self.vault.absolute(&path)),
                FileKind::Markdown => {
                    if self.depth + 1 >= MAX_EMBED_DEPTH {
                        EmbedKind::TooDeep
                    } else {
                        self.note_embed(&path, parsed.subpath.as_ref(), &wikilink_display(&parsed))
                    }
                }
                // A base nests no further: its rows are notes, and a note
                // inside one is a row rather than an embed, so the depth
                // guard above has nothing to count here — and a note that
                // embeds a base listing that same note still terminates,
                // because a row is cells rather than another model. The cache
                // is the cost guard instead: a view over the whole vault is
                // the one embed worth never resolving twice.
                FileKind::Base => match self.bases.resolve(&path, self.vault) {
                    Some(base) => EmbedKind::Base(base),
                    None => EmbedKind::Missing,
                },
                // A PDF is resolved and nothing more: the file is parsed by
                // the document cache when the embed is drawn, off the UI
                // thread. The subpath (`#page=3&height=400`) is the link's
                // own, not a heading, so it is read from the raw link text.
                FileKind::Pdf => {
                    let id = self.next_id;
                    self.next_id += 1;
                    EmbedKind::Pdf(crate::pdf::embed::PdfEmbed::new(
                        id,
                        path.clone(),
                        self.vault.absolute(&path),
                        link,
                        height,
                    ))
                }
                FileKind::Audio => EmbedKind::Audio(self.vault.absolute(&path)),
                FileKind::Video => EmbedKind::Video(self.vault.absolute(&path)),
                other => EmbedKind::Unsupported(other),
            },
        };
        Embed {
            kind,
            link: link.to_string(),
            alt,
            width,
            height,
        }
    }

    fn note_embed(&mut self, path: &str, subpath: Option<&Subpath<'_>>, title: &str) -> EmbedKind {
        let Some(text) = self.vault.read(path) else {
            return EmbedKind::Missing;
        };
        let doc = parse(&text);
        let selected: Vec<Block<'_>> = match subpath {
            None => doc
                .blocks
                .iter()
                .filter(|b| !matches!(b.kind, BlockKind::Yaml { .. }))
                .cloned()
                .collect(),
            Some(Subpath::Headings(names)) => match section(&doc.blocks, names) {
                Some(s) => s.to_vec(),
                None => return EmbedKind::Missing,
            },
            Some(Subpath::Block(id)) => match block_by_id(&doc.blocks, id) {
                Some(b) => vec![b.clone()],
                None => return EmbedKind::Missing,
            },
        };
        let mut next_id = self.next_id;
        let blocks = build_embedded(
            path,
            &doc,
            &selected,
            self.depth + 1,
            &mut next_id,
            self.bases,
            self.vault,
            self.opts,
        );
        self.next_id = next_id;
        let title = if title.is_empty() {
            path.trim_end_matches(".md").to_string()
        } else {
            title.to_string()
        };
        EmbedKind::Note {
            path: path.to_string(),
            title,
            blocks,
        }
    }
}

/// Accumulates text with a style stack into one [`RichText`].
#[derive(Default)]
struct RichBuilder {
    text: String,
    style: InlineStyle,
    runs: Vec<(Range<usize>, InlineStyle)>,
    links: Vec<(Range<usize>, LinkTarget)>,
}

impl RichBuilder {
    fn len(&self) -> usize {
        self.text.len()
    }

    fn push(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        let start = self.text.len();
        self.text.push_str(s);
        let end = self.text.len();
        if self.style.is_plain() {
            return;
        }
        if let Some((range, style)) = self.runs.last_mut() {
            if *style == self.style && range.end == start {
                range.end = end;
                return;
            }
        }
        self.runs.push((start..end, self.style));
    }

    fn styled(&mut self, f: impl FnOnce(&mut InlineStyle), body: impl FnOnce(&mut RichBuilder)) {
        let saved = self.style;
        f(&mut self.style);
        body(self);
        self.style = saved;
    }

    fn link(&mut self, start: usize, target: LinkTarget) {
        let end = self.text.len();
        if end > start {
            self.links.push((start..end, target));
        }
    }

    fn finish(self, id: usize) -> RichText {
        RichText {
            id,
            text: self.text,
            runs: self.runs,
            links: self.links,
        }
    }
}

/// `200` or `200x100` → `(width, Some(height))`.
fn size_hint(s: &str) -> Option<(f32, Option<f32>)> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit() || b == b'x') {
        return None;
    }
    let mut parts = s.split('x');
    let w: f32 = parts.next()?.parse().ok()?;
    let h = match parts.next() {
        Some(h) => Some(h.parse().ok()?),
        None => None,
    };
    if parts.next().is_some() {
        return None;
    }
    Some((w, h))
}

/// A raw `<img>` of an HTML block, with the attributes Obsidian honours.
struct HtmlImage {
    src: String,
    alt: String,
    width: Option<f32>,
    height: Option<f32>,
}

/// The images of an HTML block that holds nothing but `<img>` tags -- the
/// shape Obsidian renders as one `div.el-img` rather than as source. A
/// block with any other content, or an image pointing outside the vault,
/// gives `None` and keeps its raw text.
fn html_images(content: &str) -> Option<Vec<HtmlImage>> {
    let mut out = Vec::new();
    let mut rest = content.trim();
    while !rest.is_empty() {
        let after = match rest.get(..4) {
            // Only `<img` itself, not `<image>` or some longer tag name.
            Some(tag) if tag.eq_ignore_ascii_case("<img") => &rest[4..],
            _ => return None,
        };
        if !after.starts_with([' ', '\t', '\n', '\r', '/', '>']) {
            return None;
        }
        let (attrs, tail) = split_tag(after)?;
        out.push(html_image(attrs)?);
        rest = tail.trim_start();
    }
    (!out.is_empty()).then_some(out)
}

/// Splits a tag's attributes from the rest of the input at the `>` that
/// closes it, skipping any inside a quoted value.
fn split_tag(s: &str) -> Option<(&str, &str)> {
    let b = s.as_bytes();
    let mut quote = None;
    for (i, &c) in b.iter().enumerate() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == b'"' || c == b'\'' => quote = Some(c),
            None if c == b'>' => return Some((&s[..i], &s[i + 1..])),
            None => {}
        }
    }
    None
}

/// One `<img>`, from the text between `<img` and its `>`.
fn html_image(attrs: &str) -> Option<HtmlImage> {
    let mut src = None;
    let mut alt = String::new();
    let mut width = None;
    let mut height = None;
    for (name, value) in html_attrs(attrs.trim_end().trim_end_matches('/'))? {
        match name.as_str() {
            "src" => src = Some(percent_decode(&value).into_owned()),
            "alt" => alt = value,
            // A raw dimension is CSS pixels; a `%` or a `calc()` is not
            // something the embed can carry, so the block stays source.
            "width" => width = Some(value.parse().ok()?),
            "height" => height = Some(value.parse().ok()?),
            _ => {}
        }
    }
    let src = src?;
    // A remote image is fetched by Obsidian and shown as a link here, the
    // way a markdown image with a scheme already is.
    if src.is_empty() || has_scheme(&src) {
        return None;
    }
    Some(HtmlImage {
        src,
        alt,
        width,
        height,
    })
}

/// A tag's attributes as lowercase name and raw value pairs. A malformed
/// run (an unterminated quote, a value with no name) gives `None`.
fn html_attrs(s: &str) -> Option<Vec<(String, String)>> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        let start = i;
        while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'=' {
            i += 1;
        }
        if i == start {
            return None;
        }
        let name = s[start..i].to_ascii_lowercase();
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut value = String::new();
        if b.get(i) == Some(&b'=') {
            i += 1;
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            match b.get(i) {
                Some(&q) if q == b'"' || q == b'\'' => {
                    i += 1;
                    let from = i;
                    while i < b.len() && b[i] != q {
                        i += 1;
                    }
                    if i >= b.len() {
                        return None;
                    }
                    value = s[from..i].to_string();
                    i += 1;
                }
                _ => {
                    let from = i;
                    while i < b.len() && !b[i].is_ascii_whitespace() {
                        i += 1;
                    }
                    value = s[from..i].to_string();
                }
            }
        }
        out.push((name, value));
    }
    Some(out)
}

/// Display text of a wikilink without an alias: `Note`, `Note > Heading`, `Note > ^id`.
fn wikilink_display(link: &silica_markdown::LinkText<'_>) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !link.path.is_empty() {
        parts.push(link.path.to_string());
    }
    match &link.subpath {
        Some(Subpath::Headings(hs)) => parts.extend(hs.iter().map(|h| h.to_string())),
        Some(Subpath::Block(b)) => parts.push(format!("^{b}")),
        None => {}
    }
    parts.join(" > ")
}

fn heading_matches(block: &Block<'_>, name: &str) -> Option<u8> {
    if let BlockKind::Heading { level, content, .. } = &block.kind {
        if plain_text(content).trim().eq_ignore_ascii_case(name.trim()) {
            return Some(*level);
        }
    }
    None
}

/// The blocks under a heading path (`#A#B`): from the matched heading to
/// the next heading of the same or a higher level.
fn section<'b, 'a>(blocks: &'b [Block<'a>], names: &[&str]) -> Option<&'b [Block<'a>]> {
    let mut range: &[Block<'a>] = blocks;
    for name in names {
        let start = range
            .iter()
            .position(|b| heading_matches(b, name).is_some())?;
        let level = heading_matches(&range[start], name)?;
        let end = range[start + 1..]
            .iter()
            .position(|b| matches!(&b.kind, BlockKind::Heading { level: l, .. } if *l <= level))
            .map(|i| start + 1 + i)
            .unwrap_or(range.len());
        range = &range[start..end];
    }
    Some(range)
}

fn block_by_id<'b, 'a>(blocks: &'b [Block<'a>], id: &str) -> Option<&'b Block<'a>> {
    let mut found = None;
    silica_markdown::walk_blocks(blocks, &mut |b| {
        if found.is_none() && b.id.is_some_and(|i| i.id.eq_ignore_ascii_case(id)) {
            found = Some(b);
        }
    });
    if found.is_some() {
        return found;
    }
    // A list item id lives on the item's first paragraph in the cache but
    // the block that carries it in the tree is the paragraph itself, which
    // `walk_blocks` visits; nothing more to do here.
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use silica_formats::properties::Value as PropertyValue;
    use std::collections::HashMap;

    struct Stub {
        files: HashMap<&'static str, &'static str>,
        /// How many times [`EmbedSource::base`] was asked, so a test can see
        /// that one model resolves a base once however often it is embedded.
        base_calls: std::cell::Cell<usize>,
    }

    impl EmbedSource for Stub {
        fn resolve(&self, linktext: &str, _source: &str) -> Option<String> {
            let target = silica_markdown::parse_linktext(linktext).path;
            let target = if target.is_empty() { "Self" } else { target };
            self.files
                .keys()
                .find(|k| {
                    k.trim_end_matches(".md")
                        .eq_ignore_ascii_case(target.trim_end_matches(".md"))
                })
                .map(|k| k.to_string())
        }
        fn read(&self, path: &str) -> Option<String> {
            self.files.get(path).map(|s| s.to_string())
        }
        fn absolute(&self, path: &str) -> PathBuf {
            PathBuf::from("/vault").join(path)
        }
        /// A base of the stub's own: one named view and no rows, because
        /// what the rows hold is `silica-bases`'s business and not the
        /// model's. `Broken.base` is a file that exists and still answers
        /// nothing, which is the other way an embed comes out `Missing`.
        fn base(&self, path: &str) -> Option<crate::bases::EmbeddedBase> {
            self.base_calls.set(self.base_calls.get() + 1);
            if path == "Broken.base" {
                return None;
            }
            self.files
                .contains_key(path)
                .then(|| crate::bases::EmbeddedBase {
                    path: path.to_string(),
                    view_name: Some("View".to_string()),
                    layout: silica_bases::Layout::default(),
                    table: None,
                    error: None,
                })
        }
    }

    fn stub() -> Stub {
        Stub {
            files: HashMap::from([
                ("Self.md", "root"),
                (
                    "Other.md",
                    "---\ntitle: x\n---\n# A\nfirst\n## B\nsecond ^blk\n# C\nthird",
                ),
                ("Loop.md", "![[Loop]]"),
                ("pic.png", ""),
                ("Field.pdf", ""),
                ("Main.base", ""),
                ("Broken.base", ""),
                ("Nester.md", "![[Main.base]]"),
            ]),
            base_calls: std::cell::Cell::new(0),
        }
    }

    fn first_embed(block: &RBlock) -> &Embed {
        match block {
            RBlock::Paragraph(segs) => match &segs[0] {
                Segment::Embed(embed) => embed,
                other => panic!("not an embed: {other:?}"),
            },
            other => panic!("not a paragraph: {other:?}"),
        }
    }

    fn para_text(block: &RBlock) -> &RichText {
        match block {
            RBlock::Paragraph(segs) => match &segs[0] {
                Segment::Text(t) => t,
                other => panic!("not text: {other:?}"),
            },
            other => panic!("not a paragraph: {other:?}"),
        }
    }

    #[test]
    fn runs_merge_and_links_cover_their_text() {
        let m = build(
            "Self.md",
            "a **b *c*** [[Note|n]] #tag `x` https://e.com",
            &stub(),
        );
        let t = para_text(&m.blocks[0]);
        assert_eq!(t.text, "a b c n #tag x https://e.com");
        let bold = InlineStyle {
            bold: true,
            ..InlineStyle::default()
        };
        let bold_italic = InlineStyle {
            bold: true,
            italic: true,
            ..InlineStyle::default()
        };
        assert_eq!(t.runs[0], (2..4, bold));
        assert_eq!(t.runs[1], (4..5, bold_italic));
        assert!(t
            .runs
            .iter()
            .any(|(r, s)| s.link && &t.text[r.clone()] == "n"));
        assert!(t
            .runs
            .iter()
            .any(|(r, s)| s.code && &t.text[r.clone()] == "x"));
        assert_eq!(
            t.links,
            vec![
                (6..7, LinkTarget::Internal("Note".into())),
                (8..12, LinkTarget::Tag("tag".into())),
                (15..28, LinkTarget::External("https://e.com".into())),
            ]
        );
    }

    #[test]
    fn soft_breaks_become_newlines_and_comments_vanish() {
        let m = build("Self.md", "one\ntwo %%hidden%% three", &stub());
        assert_eq!(para_text(&m.blocks[0]).text, "one\ntwo  three");
        let strict = BuildOptions {
            strict_line_breaks: true,
        };
        let m = build_with("Self.md", "one\ntwo  \nthree", &stub(), strict);
        assert_eq!(para_text(&m.blocks[0]).text, "one two\nthree");
    }

    #[test]
    fn embeds_split_paragraphs_and_resolve_recursively() {
        let m = build(
            "Self.md",
            "before ![[pic.png|200x100]] ![[Other#B]] ![[Other#^blk]] ![[Nope]] after",
            &stub(),
        );
        let RBlock::Paragraph(segs) = &m.blocks[0] else {
            panic!()
        };
        assert_eq!(segs.len(), 6);
        match &segs[1] {
            Segment::Embed(Embed {
                kind: EmbedKind::Image(p),
                width,
                height,
                ..
            }) => {
                assert_eq!(p, &PathBuf::from("/vault/pic.png"));
                assert_eq!((*width, *height), (Some(200.0), Some(100.0)));
            }
            other => panic!("{other:?}"),
        }
        match &segs[2] {
            Segment::Embed(Embed {
                kind: EmbedKind::Note { title, blocks, .. },
                ..
            }) => {
                assert_eq!(title, "Other > B");
                assert!(matches!(&blocks[0], RBlock::Heading { plain, .. } if plain == "B"));
                assert_eq!(para_text(&blocks[1]).text, "second");
                assert_eq!(blocks.len(), 2, "section stops before # C");
            }
            other => panic!("{other:?}"),
        }
        match &segs[3] {
            Segment::Embed(Embed {
                kind: EmbedKind::Note { blocks, .. },
                ..
            }) => assert_eq!(para_text(&blocks[0]).text, "second"),
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            &segs[4],
            Segment::Embed(Embed {
                kind: EmbedKind::Missing,
                ..
            })
        ));
        assert_eq!(para_text(&segs[5].as_text()).text, " after");
    }

    impl Segment {
        fn as_text(&self) -> RBlock {
            match self {
                Segment::Text(t) => RBlock::Paragraph(vec![Segment::Text(t.clone())]),
                other => panic!("{other:?}"),
            }
        }
    }

    #[test]
    fn recursive_embeds_stop_at_the_depth_limit() {
        let m = build("Loop.md", "![[Loop]]", &stub());
        let mut depth = 0;
        let mut blocks = &m.blocks;
        loop {
            let RBlock::Paragraph(segs) = &blocks[0] else {
                panic!()
            };
            match &segs[0] {
                Segment::Embed(Embed {
                    kind: EmbedKind::Note { blocks: inner, .. },
                    ..
                }) => {
                    depth += 1;
                    blocks = inner;
                }
                Segment::Embed(Embed {
                    kind: EmbedKind::TooDeep,
                    ..
                }) => break,
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(depth, MAX_EMBED_DEPTH - 1);
    }

    #[test]
    fn an_embedded_base_is_its_own_kind_wherever_it_is_written() {
        let stub = stub();
        // The same base twice, and once more inside an embedded note.
        let m = build(
            "Self.md",
            "![[Main.base]]\n\n![[Nester]]\n\n![[Main.base]]",
            &stub,
        );
        for ix in [0, 2] {
            match &first_embed(&m.blocks[ix]).kind {
                EmbedKind::Base(base) => {
                    assert_eq!(base.path, "Main.base");
                    assert_eq!(base.view_name.as_deref(), Some("View"));
                }
                other => panic!("{other:?}"),
            }
        }
        // A base inside an embedded note is a base there too: the note's own
        // depth bounds the nesting, and the base below it adds none.
        let EmbedKind::Note { blocks, .. } = &first_embed(&m.blocks[1]).kind else {
            panic!("the middle embed is a note")
        };
        assert!(matches!(first_embed(&blocks[0]).kind, EmbedKind::Base(_)));
        // And the view over the whole vault was resolved exactly once.
        assert_eq!(stub.base_calls.get(), 1);
    }

    #[test]
    fn a_pdf_embed_carries_its_subpath_and_reads_nothing() {
        use crate::pdf::embed::DEFAULT_HEIGHT;
        let m = build(
            "Self.md",
            "![[Field.pdf]]\n\n![[Field.pdf#page=2]]\n\n![[Field.pdf#height=200|300x150]]\n\n\
             ![](Field.pdf#page=3)\n\n![[Nope.pdf]]",
            &stub(),
        );
        let pdf = |ix: usize| match &first_embed(&m.blocks[ix]).kind {
            EmbedKind::Pdf(pdf) => pdf.clone(),
            other => panic!("{other:?}"),
        };
        // The stub never read `Field.pdf` (it has no text to read): the
        // model carries the path and the subpath and nothing else.
        let plain = pdf(0);
        assert_eq!(plain.path, "Field.pdf");
        assert_eq!(plain.absolute, PathBuf::from("/vault/Field.pdf"));
        assert_eq!((plain.page, plain.height), (None, None));
        assert_eq!(plain.box_height(), DEFAULT_HEIGHT);
        // `#page=2` is the link's own subpath, not a heading: zero-based.
        assert_eq!(pdf(1).page, Some(1));
        // `#height=` wins over the alias size; the alias is still the alt.
        let sized = pdf(2);
        assert_eq!(sized.height, Some(200.0));
        assert_eq!(first_embed(&m.blocks[2]).width, Some(300.0));
        // The markdown image form reads the same subpath.
        assert_eq!(pdf(3).page, Some(2));
        // Every embed has an id of its own, so two of one file scroll apart.
        assert_ne!(pdf(0).id, pdf(1).id);
        assert!(matches!(first_embed(&m.blocks[4]).kind, EmbedKind::Missing));
    }

    #[test]
    fn each_embed_walks_into_embedded_notes() {
        let m = build("Self.md", "![[Nester]]\n\n> ![[Field.pdf]]", &stub());
        let mut seen = Vec::new();
        each_embed(&m.blocks, &mut |embed| seen.push(embed.link.clone()));
        assert_eq!(seen, vec!["Nester", "Main.base", "Field.pdf"]);
    }

    #[test]
    fn a_base_nothing_answers_for_is_missing_rather_than_empty() {
        let stub = stub();
        let m = build("Self.md", "![[Nope.base]]\n\n![[Broken.base]]", &stub);
        // A link that resolves to no file at all, and a file that exists and
        // still answers no base: both are the embed's `Missing`.
        assert!(matches!(first_embed(&m.blocks[0]).kind, EmbedKind::Missing));
        assert!(matches!(first_embed(&m.blocks[1]).kind, EmbedKind::Missing));
        // The miss is remembered too, so a second mention asks nothing again.
        let m = build("Self.md", "![[Broken.base]]\n\n![[Broken.base]]", &stub);
        assert!(matches!(first_embed(&m.blocks[1]).kind, EmbedKind::Missing));
        assert_eq!(stub.base_calls.get(), 2, "once per model, not per embed");
    }

    /// `blocks` drops a comment block; `starts` has to drop exactly the
    /// same one, or every jump past it lands a block out.
    #[test]
    fn a_block_comment_shifts_neither_list() {
        let plain = build("Self.md", "# One\n\nbody\n\n# Two\n", &stub());
        assert_eq!(plain.blocks.len(), plain.starts.len());
        let two = plain.starts[2];
        assert!(matches!(&plain.blocks[2], RBlock::Heading { plain, .. } if plain == "Two"));

        // The same note behind a block comment: one parsed block more, the
        // same built blocks, and "Two" still the third of them.
        let text = "%%\na block comment\n%%\n\n# One\n\nbody\n\n# Two\n";
        let with_comment = build("Self.md", text, &stub());
        assert_eq!(
            with_comment.blocks.len(),
            with_comment.starts.len(),
            "{:?}",
            with_comment.blocks
        );
        assert_eq!(with_comment.blocks.len(), plain.blocks.len());
        assert!(matches!(&with_comment.blocks[2], RBlock::Heading { plain, .. } if plain == "Two"));
        // The offsets moved by the comment's length, and `block_at` follows
        // them rather than the index the parsed list would have given.
        assert!(with_comment.starts[2] > two);
        assert_eq!(
            with_comment.block_at(text.find("# Two").unwrap()),
            Some(2),
            "a jump to Two has to land on Two"
        );
    }

    /// An offset inside a footnote definition belongs to the Footnotes
    /// section, which is appended after every parsed block and so needs a
    /// start of its own.
    #[test]
    fn an_offset_in_a_definition_resolves_to_the_footnotes_block() {
        let text = "a[^1] b\n\n# H\n\n[^1]: the note\n";
        let m = build("Self.md", text, &stub());
        assert_eq!(m.blocks.len(), m.starts.len());
        let last = m.blocks.len() - 1;
        assert!(matches!(m.blocks[last], RBlock::Footnotes(_)));
        let definition = text.find("[^1]: ").unwrap();
        assert_eq!(m.footnotes, vec![definition]);
        assert_eq!(m.block_at(definition), Some(last));
    }

    /// Clicking reference `n` reveals definition `n`, not the end of the
    /// note; an inline footnote has no definition and points at itself.
    #[test]
    fn each_footnote_knows_where_its_definition_is() {
        let text = "x[^b] y^[inline] z[^a]\n\n[^a]: A text\n[^b]: B text\n";
        let m = build("Self.md", text, &stub());
        assert_eq!(
            m.footnotes,
            vec![
                text.find("[^b]: ").unwrap(),
                0,
                text.find("[^a]: ").unwrap()
            ],
            "b is 1, the inline one is 2, a is 3"
        );
    }

    #[test]
    fn footnotes_number_by_first_reference_and_collect_at_the_end() {
        let m = build(
            "Self.md",
            "x[^b] y^[inline] z[^a] again[^b] none[^zz]\n\n[^a]: A text\n[^b]: B text",
            &stub(),
        );
        let t = para_text(&m.blocks[0]);
        assert_eq!(t.text, "x[1] y[2] z[3] again[1] nonezz");
        assert_eq!(
            t.links.iter().map(|(_, l)| l.clone()).collect::<Vec<_>>(),
            vec![
                LinkTarget::Footnote(1),
                LinkTarget::Footnote(2),
                LinkTarget::Footnote(3),
                LinkTarget::Footnote(1)
            ]
        );
        let RBlock::Footnotes(notes) = m.blocks.last().unwrap() else {
            panic!("{:?}", m.blocks.last())
        };
        assert_eq!(notes.len(), 3);
        assert_eq!(para_text(&notes[0].1[0]).text, "B text");
        assert_eq!(para_text(&notes[1].1[0]).text, "inline");
        assert_eq!(para_text(&notes[2].1[0]).text, "A text");
    }

    #[test]
    fn properties_lists_and_callouts() {
        let m = build(
            "Self.md",
            "---\ntitle: T\ntags: [a, b]\n---\n> [!TIP]- Fold me\n> body\n\n1. one\n2. [x] two\n\n- [ ] task",
            &stub(),
        );
        let RBlock::Properties(entries) = &m.blocks[0] else {
            panic!("{:?}", m.blocks[0])
        };
        assert_eq!(
            entries.iter().map(|e| e.key.as_str()).collect::<Vec<_>>(),
            vec!["title", "tags"]
        );
        assert_eq!(entries[0].value, PropertyValue::Text("T".into()));
        assert_eq!(
            entries[1].value,
            PropertyValue::List(vec!["a".into(), "b".into()])
        );
        match &m.blocks[1] {
            RBlock::Callout {
                kind, fold, title, ..
            } => {
                assert_eq!(*kind, "tip");
                assert_eq!(*fold, Some(true));
                assert_eq!(title.text, "Fold me");
            }
            other => panic!("{other:?}"),
        }
        match &m.blocks[2] {
            RBlock::List { items, .. } => {
                assert_eq!(items[0].number, Some(1));
                assert_eq!(items[1].number, Some(2));
                assert_eq!(items[1].task, Some('x'));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(m.starts.len(), 4);
        assert_eq!(m.block_at(0), Some(0));
        assert_eq!(m.block_at(m.starts[2] + 3), Some(2));
    }

    #[test]
    fn task_markers_carry_the_line_and_stop_at_an_embed() {
        let vault = Stub {
            files: HashMap::from([("Tasks.md", "- [ ] inner\n")]),
            base_calls: std::cell::Cell::new(0),
        };
        let m = build(
            "Self.md",
            "---\na: 1\n---\n\n- [ ] alpha\n  - [x] beta\n\n![[Tasks]]\n",
            &vault,
        );
        let RBlock::List { items, .. } = &m.blocks[1] else {
            panic!("{:?}", m.blocks[1])
        };
        assert_eq!(items[0].marker, Some(TaskMarker { line: 4, col: 2 }));
        let RBlock::List { items: nested, .. } = &items[0].children[1] else {
            panic!("{:?}", items[0].children)
        };
        assert_eq!(nested[0].marker, Some(TaskMarker { line: 5, col: 4 }));
        // The embedded item's bytes are another file's, so it carries no
        // marker and the reading view leaves its box inert.
        let RBlock::Paragraph(segments) = m.blocks.last().unwrap() else {
            panic!("{:?}", m.blocks.last())
        };
        let Segment::Embed(embed) = &segments[0] else {
            panic!("{:?}", segments[0])
        };
        let EmbedKind::Note { blocks, .. } = &embed.kind else {
            panic!("{:?}", embed.kind)
        };
        let RBlock::List { items, .. } = &blocks[0] else {
            panic!("{blocks:?}")
        };
        assert_eq!(items[0].task, Some(' '));
        assert_eq!(items[0].marker, None);
    }

    #[test]
    fn a_code_block_copies_lf_text_with_one_newline_removed() {
        let m = build("Self.md", "```js\nlet x = 1;\n\n\n```\n", &stub());
        match &m.blocks[0] {
            RBlock::Code {
                language,
                text,
                copy,
                ..
            } => {
                assert_eq!(language.as_deref(), Some("js"));
                assert_eq!(text, "let x = 1;");
                assert_eq!(copy, "let x = 1;\n\n");
            }
            other => panic!("{other:?}"),
        }
        // A CRLF file renders and copies as LF, the way Obsidian does,
        // while the file keeps its own endings.
        let m = build("Self.md", "```\r\none\r\ntwo\r\n```\r\n", &stub());
        match &m.blocks[0] {
            RBlock::Code { text, copy, .. } => {
                assert_eq!(text, "one\ntwo");
                assert_eq!(copy, "one\ntwo");
            }
            other => panic!("{other:?}"),
        }
    }

    fn widgets(src: &str, vault: &dyn EmbedSource) -> Vec<Widget> {
        let doc = parse(src);
        build_widgets("Self.md", &doc, vault, BuildOptions::default())
    }

    #[test]
    fn live_preview_makes_a_callout_a_widget() {
        let src = "top\n\n> [!warning]- Careful\n> body\n> more\n\n> plain quote\n";
        let w = widgets(src, &stub());
        assert_eq!(w.len(), 1, "the plain quote is not a widget: {w:?}");
        let RBlock::Callout {
            kind,
            fold,
            title,
            children,
        } = &w[0].block
        else {
            panic!("{:?}", w[0].block)
        };
        assert_eq!(*kind, "warning");
        assert_eq!(*fold, Some(true), "`-` opens collapsed, as in Obsidian");
        assert_eq!(title.text, "Careful");
        assert_eq!(children.len(), 1);
        // The span covers every line of the callout, so Live Preview folds
        // all three away behind the widget.
        assert_eq!(
            &src[w[0].span.clone()],
            "> [!warning]- Careful\n> body\n> more"
        );
    }

    #[test]
    fn an_image_html_block_becomes_an_image_widget() {
        let w = widgets("<img src=\"pic.png\" width=\"100\" alt=\"A\">\n", &stub());
        assert_eq!(w.len(), 1);
        let RBlock::Paragraph(segs) = &w[0].block else {
            panic!("{:?}", w[0].block)
        };
        match &segs[0] {
            Segment::Embed(Embed {
                kind: EmbedKind::Image(path),
                alt,
                width,
                height,
                ..
            }) => {
                assert_eq!(path, &PathBuf::from("/vault/pic.png"));
                assert_eq!(alt, "A");
                assert_eq!((*width, *height), (Some(100.0), None));
            }
            other => panic!("{other:?}"),
        }
        // Consecutive raw imgs are one block and render as one run of
        // images; a percent-escaped src is the vault path it decodes to.
        let m = build(
            "Self.md",
            "<img src='pic.png'/>\n<img src=pic%2Epng height=50>\n",
            &stub(),
        );
        let RBlock::Paragraph(segs) = &m.blocks[0] else {
            panic!("{:?}", m.blocks[0])
        };
        assert_eq!(segs.len(), 2);
        assert!(matches!(
            &segs[1],
            Segment::Embed(Embed {
                kind: EmbedKind::Image(_),
                height: Some(h),
                ..
            }) if *h == 50.0
        ));
    }

    #[test]
    fn other_html_blocks_stay_source() {
        // A comment, a script and a remote image all reach the builder as
        // the same `HtmlBlock`, and none of them is something the renderer
        // can draw, so they keep their raw text and never fold their lines.
        for src in [
            "<!-- a comment -->\n",
            "<script>window.x = 1</script>\n",
            "<img src=\"https://e.com/a.png\">\n",
            "<img src=\"pic.png\" width=\"50%\">\n",
            "<div>mixed</div>\n<img src=\"pic.png\">\n",
        ] {
            assert!(widgets(src, &stub()).is_empty(), "{src:?} became a widget");
            let m = build("Self.md", src, &stub());
            assert!(
                matches!(&m.blocks[0], RBlock::Html(_)),
                "{src:?} -> {:?}",
                m.blocks[0]
            );
        }
    }

    #[test]
    fn callout_without_title_shows_its_type() {
        let m = build("Self.md", "> [!warning]\n> text", &stub());
        match &m.blocks[0] {
            RBlock::Callout { title, fold, .. } => {
                assert_eq!(title.text, "Warning");
                assert_eq!(*fold, None);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn display_math_in_a_paragraph_is_its_own_segment() {
        let opts = BuildOptions::default();
        let m = build_with("A.md", "Inline $x$ then $$y$$ and after.", &stub(), opts);
        let RBlock::Paragraph(segments) = &m.blocks[0] else {
            panic!("{:?}", m.blocks[0])
        };
        // The space on either side of the block is its margin, not text.
        let texts: Vec<_> = segments
            .iter()
            .map(|s| match s {
                Segment::Text(t) => format!("text:{}", t.text),
                Segment::Math(tex) => format!("math:{tex}"),
                Segment::Embed(_) => "embed".to_string(),
            })
            .collect();
        assert_eq!(
            texts,
            vec!["text:Inline x then", "math:y", "text:and after."]
        );
        // Inline math is a styled run holding the TeX, for the view to
        // typeset over a placeholder.
        let Segment::Text(first) = &segments[0] else {
            unreachable!()
        };
        assert_eq!(
            first.runs,
            vec![(
                7..8,
                InlineStyle {
                    math: true,
                    ..InlineStyle::default()
                }
            )]
        );
        // The block form is a block.
        let m = build_with("A.md", "$$\nz\n$$", &stub(), BuildOptions::default());
        assert_eq!(m.blocks[0], RBlock::Math("z".to_string()));
    }

    #[test]
    fn parses_mermaid_block_and_builds_widget() {
        let text = "```mermaid\nflowchart TD\n    A --> B\n```";
        let doc = silica_markdown::parse(text);
        let stub = Stub {
            files: HashMap::new(),
            base_calls: std::cell::Cell::new(0),
        };
        let m = build("test.md", text, &stub);
        assert_eq!(m.blocks.len(), 1);
        let RBlock::Mermaid { text: diagram, .. } = &m.blocks[0] else {
            panic!("expected Mermaid block");
        };
        assert_eq!(diagram, "flowchart TD\n    A --> B");

        let widgets = build_widgets("test.md", &doc, &stub, BuildOptions::default());
        assert_eq!(widgets.len(), 1);
        assert!(matches!(widgets[0].block, RBlock::Mermaid { .. }));
    }

    #[test]
    fn parses_remote_images() {
        let text = "![Photo](https://example.com/cat.png)\n\n![Sized|300x200](https://example.com/dog.jpg)\n\n![[https://example.com/bird.png]]";
        let stub = Stub {
            files: HashMap::new(),
            base_calls: std::cell::Cell::new(0),
        };
        let m = build("test.md", text, &stub);
        assert_eq!(m.blocks.len(), 3);

        let RBlock::Paragraph(segs0) = &m.blocks[0] else { panic!() };
        let Segment::Embed(embed0) = &segs0[0] else { panic!() };
        assert_eq!(embed0.kind, EmbedKind::RemoteImage("https://example.com/cat.png".into()));
        assert_eq!(embed0.alt, "Photo");
        assert_eq!(embed0.width, None);
        assert_eq!(embed0.height, None);

        let RBlock::Paragraph(segs1) = &m.blocks[1] else { panic!() };
        let Segment::Embed(embed1) = &segs1[0] else { panic!() };
        assert_eq!(embed1.kind, EmbedKind::RemoteImage("https://example.com/dog.jpg".into()));
        assert_eq!(embed1.width, Some(300.0));
        assert_eq!(embed1.height, Some(200.0));

        let RBlock::Paragraph(segs2) = &m.blocks[2] else { panic!() };
        let Segment::Embed(embed2) = &segs2[0] else { panic!() };
        assert_eq!(embed2.kind, EmbedKind::RemoteImage("https://example.com/bird.png".into()));
    }

    #[test]
    fn parses_remote_audio_and_video() {
        let text = "![Song](https://example.com/track.mp3)\n\n![Film](https://example.com/movie.mp4)\n\n![[https://example.com/podcast.ogg]]";
        let stub = Stub {
            files: HashMap::new(),
            base_calls: std::cell::Cell::new(0),
        };
        let m = build("test.md", text, &stub);
        assert_eq!(m.blocks.len(), 3);

        let RBlock::Paragraph(segs0) = &m.blocks[0] else { panic!() };
        let Segment::Embed(embed0) = &segs0[0] else { panic!() };
        assert_eq!(embed0.kind, EmbedKind::RemoteAudio("https://example.com/track.mp3".into()));

        let RBlock::Paragraph(segs1) = &m.blocks[1] else { panic!() };
        let Segment::Embed(embed1) = &segs1[0] else { panic!() };
        assert_eq!(embed1.kind, EmbedKind::RemoteVideo("https://example.com/movie.mp4".into()));

        let RBlock::Paragraph(segs2) = &m.blocks[2] else { panic!() };
        let Segment::Embed(embed2) = &segs2[0] else { panic!() };
        assert_eq!(embed2.kind, EmbedKind::RemoteAudio("https://example.com/podcast.ogg".into()));
    }

    #[test]
    fn parses_local_audio_and_video() {
        let text = "![[song.mp3]]\n\n![[video.mp4]]";
        let stub = Stub {
            files: HashMap::from([("song.mp3", ""), ("video.mp4", "")]),
            base_calls: std::cell::Cell::new(0),
        };
        let m = build("test.md", text, &stub);
        assert_eq!(m.blocks.len(), 2);

        let RBlock::Paragraph(segs0) = &m.blocks[0] else { panic!() };
        let Segment::Embed(embed0) = &segs0[0] else { panic!() };
        assert_eq!(embed0.kind, EmbedKind::Audio(PathBuf::from("/vault/song.mp3")));

        let RBlock::Paragraph(segs1) = &m.blocks[1] else { panic!() };
        let Segment::Embed(embed1) = &segs1[0] else { panic!() };
        assert_eq!(embed1.kind, EmbedKind::Video(PathBuf::from("/vault/video.mp4")));
    }
}
